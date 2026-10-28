# Douyin Batch Download Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a modular, manually triggered, persistent batch-download system for Douyin history entries, including normal video, image-post, live-photo, and soundtrack assets.

**Architecture:** A central SQLite migration registry installs additive download tables. A desktop-owned background service serializes persistence, permits one active job, starts a managed `--download-resolver` child process for authenticated CDP media resolution, and runs at most three Rust HTTP asset workers. The frontend integrates through namespaced commands and a dedicated `app/src/download/` module.

**Tech Stack:** Rust 2021, Tauri 2, rusqlite/WAL, Tokio, reqwest with Rustls, tokio-util cancellation, Windows APIs, Svelte 5, TypeScript, Vitest.

## Global Constraints

- Production remains Windows x64 and adds no Python, Go, Node, FFmpeg, Chromium, proxy certificate, or download sidecar runtime.
- Downloads are manual only; startup recovery performs no browser or network activity.
- Only one download job is active; at most three assets transfer concurrently inside it.
- Cookies, signed URLs, authorization headers, and raw browser responses remain memory-only.
- Download tables have no foreign keys to `videos` or `watch_records`.
- Database migration ordering is central; collector result protocol remains schema version two.
- Every implementation task follows red-green-refactor and ends with a focused commit.
- Existing unrelated worktree changes must not be staged or committed.

---

## Planned File Structure

Backend files:

- `src-tauri/migrations/003_download_jobs.sql`: additive download tables and indexes.
- `src-tauri/src/download/mod.rs`: public module exports.
- `src-tauri/src/download/contracts.rs`: states, DTOs, quality and asset contracts.
- `src-tauri/src/download/quality.rs`: deterministic format selection.
- `src-tauri/src/download/paths.rs`: naming, containment and Windows reparse-point validation.
- `src-tauri/src/download/catalog.rs`: history selection expansion and deterministic deduplication.
- `src-tauri/src/download/storage.rs`: download queries and transaction helpers.
- `src-tauri/src/download/persistence.rs`: single-writer actor and post-commit events.
- `src-tauri/src/download/recovery.rs`: startup recovery and manifest reconciliation.
- `src-tauri/src/download/worker.rs`: safe HTTP transfer, Range resume and validation.
- `src-tauri/src/download/queue.rs`: one-active-job scheduler and ownership keys.
- `src-tauri/src/download/resolver.rs`: resolver trait, manifests and fixture parser.
- `src-tauri/src/download/resolver_ipc.rs`: length-delimited parent/child protocol.
- `src-tauri/src/download/browser_adapter.rs`: CDP detail-response adapter.
- `src-tauri/src/download/service.rs`: command-facing orchestration.
- `src-tauri/src/download/errors.rs`: stable errors and Chinese descriptions.
- `src-tauri/tests/download_*.rs`: focused integration tests.

Frontend files:

- `app/src/download/types.ts`: frontend job, item and settings types.
- `app/src/download/api.ts`: namespaced command wrappers and preview stubs.
- `app/src/download/settings.ts`: quality and directory setting normalization.
- `app/src/download/selection.ts`: selection request construction.
- `app/src/download/DownloadStartDialog.svelte`: job confirmation and overrides.
- `app/src/download/DownloadTaskCenter.svelte`: persistent task UI.
- `tests/app/download-*.test.ts`: Vitest tests for pure frontend logic.

Existing integration files remain small: `src-tauri/src/lib.rs`, `src-tauri/src/main.rs`, `src-tauri/Cargo.toml`, `src-tauri/capabilities/default.json`, `app/src/App.svelte`, `app/src/types.ts`, `app/src/app-api.ts`, `app/src/styles.css`, `dist/**`, and `dist/build-manifest.json`.

---

### Task 1: Central Migration Registry And Download Schema

**Files:**
- Create: `src-tauri/migrations/003_download_jobs.sql`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/tests/storage_v2.rs`
- Create: `src-tauri/tests/download_storage.rs`

**Interfaces:**
- Produces: `pub const APPLICATION_SCHEMA_VERSION: i64 = 3`
- Produces: `pub fn configure_connection(connection: &Connection) -> Result<(), String>`
- Produces: `pub fn initialize_schema(connection: &mut Connection) -> Result<(), String>`
- Preserves: `COLLECTOR_SCHEMA_VERSION == 2`

- [ ] **Step 1: Write failing migration-registry tests**

Add tests that open an in-memory database, call `initialize_schema`, and assert `PRAGMA user_version = 3`, all three download tables exist, a second call is idempotent, and collector result schema two still imports.

```rust
#[test]
fn registry_applies_download_schema_without_changing_collector_protocol() {
    let mut db = Connection::open_in_memory().unwrap();
    initialize_schema(&mut db).unwrap();
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 3);
    for table in ["download_jobs", "download_items", "download_assets"] {
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1, "missing {table}");
    }
    assert_eq!(COLLECTOR_SCHEMA_VERSION, 2);
}
```

- [ ] **Step 2: Run tests and verify the expected failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_storage registry_applies_download_schema_without_changing_collector_protocol`

Expected: compilation fails because `initialize_schema` and `APPLICATION_SCHEMA_VERSION` do not exist.

- [ ] **Step 3: Add migration 003 and registry implementation**

Create SQL with the exact job/item/asset states, unique `(job_id, aweme_id)` and `(item_id, stable_key, manifest_generation)` constraints, indexes for active jobs and historical asset reuse, and `PRAGMA user_version = 3` performed by Rust after the transaction succeeds.

Implement:

```rust
pub const APPLICATION_SCHEMA_VERSION: i64 = 3;
const MIGRATIONS: &[(i64, &str)] = &[
    (2, include_str!("../migrations/002_react_history.sql")),
    (3, include_str!("../migrations/003_download_jobs.sql")),
];

pub fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())
}

pub fn initialize_schema(connection: &mut Connection) -> Result<(), String> {
    configure_connection(connection)?;
    let mut version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version > APPLICATION_SCHEMA_VERSION {
        return Err(format!("unsupported_schema_version:{version}"));
    }
    for (target, sql) in MIGRATIONS {
        if version < *target {
            let tx = connection.transaction().map_err(|e| e.to_string())?;
            tx.execute_batch(sql).map_err(|e| e.to_string())?;
            tx.pragma_update(None, "user_version", target).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            version = *target;
        }
    }
    Ok(())
}
```

Replace every `initialize_schema_v2` call in application database paths with `initialize_schema`. Keep `validate_and_import_result` checking collector schema two.

- [ ] **Step 4: Run migration and existing storage tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_storage --test storage_v2`

Expected: all tests pass; old v1 migration reaches v3 and current history behavior remains unchanged.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/migrations/003_download_jobs.sql src-tauri/src/lib.rs src-tauri/src/main.rs src-tauri/tests/storage_v2.rs src-tauri/tests/download_storage.rs
git commit -m "feat: add central download schema migration"
```

### Task 2: Download Contracts, Quality Selection, And Stable Errors

**Files:**
- Create: `src-tauri/src/download/mod.rs`
- Create: `src-tauri/src/download/contracts.rs`
- Create: `src-tauri/src/download/quality.rs`
- Create: `src-tauri/src/download/errors.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/download_contracts.rs`

**Interfaces:**
- Produces: `DownloadQuality::{Highest,Balanced,SpaceSaver}`
- Produces: `DownloadJobState`, `DownloadItemState`, `DownloadAssetState`
- Produces: `DownloadCandidate`, `ResolvedFormat`, `ResolvedAsset`, `ResolvedMediaManifest`
- Produces: `select_format(formats: &[ResolvedFormat], quality: DownloadQuality) -> Result<ResolvedFormat, DownloadError>`

- [ ] **Step 1: Write failing contract and quality tests**

Cover camelCase serialization, strict unknown-field rejection, H.264 tie preference, 1080p and 720p caps, fallback above a cap, stable asset keys, and Chinese error descriptions.

```rust
#[test]
fn balanced_prefers_best_h264_at_or_below_1080p() {
    let formats = vec![
        format("2160p", 3840, 2160, 12_000, "h265"),
        format("1080p-h265", 1920, 1080, 6_000, "h265"),
        format("1080p-h264", 1920, 1080, 6_000, "h264"),
    ];
    assert_eq!(select_format(&formats, DownloadQuality::Balanced).unwrap().id, "1080p-h264");
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_contracts`

Expected: compilation fails because module contracts do not exist.

- [ ] **Step 3: Implement minimal contracts and selector**

Use string-backed serde enums with `rename_all = "snake_case"`, `deny_unknown_fields` on external structs, decimal-string `aweme_id` validation, and deterministic sorting by height, width, bitrate and codec preference.

- [ ] **Step 4: Run focused tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_contracts`

Expected: all contract and quality tests pass.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/download src-tauri/src/lib.rs src-tauri/tests/download_contracts.rs
git commit -m "feat: define download contracts and quality profiles"
```

### Task 3: Catalog Expansion, Deterministic Deduplication, And Job Snapshots

**Files:**
- Create: `src-tauri/src/download/catalog.rs`
- Create: `src-tauri/src/download/storage.rs`
- Create: `src-tauri/tests/download_catalog.rs`
- Modify: `src-tauri/src/download/mod.rs`

**Interfaces:**
- Produces: `DownloadSelection::{Checked{watch_record_ids},Filtered{query},All}`
- Produces: `expand_candidates(connection: &Connection, selection: &DownloadSelection) -> Result<Vec<DownloadCandidate>, DownloadError>`
- Produces: `create_download_job(connection: &mut Connection, request: &CreateDownloadJobRequest) -> Result<i64, DownloadError>`

- [ ] **Step 1: Write failing catalog tests**

Seed duplicate watch records for one `aweme_id` and assert newest `history_time_ms`, then ascending `source_position`, then descending ID selects the snapshot. Verify checked duplicates produce one item and filtered selection ignores pagination.

```rust
assert_eq!(candidates.len(), 1);
assert_eq!(candidates[0].aweme_id, "1001");
assert_eq!(candidates[0].title.as_deref(), Some("newest"));
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_catalog`

Expected: compilation fails because catalog functions do not exist.

- [ ] **Step 3: Implement SQL expansion and atomic job creation**

Use a window function:

```sql
ROW_NUMBER() OVER (
  PARTITION BY v.aweme_id
  ORDER BY wr.history_time_ms DESC, wr.source_position ASC, wr.id DESC
) AS candidate_rank
```

Insert the job and all item snapshots in one immediate transaction. Never store a foreign key to history tables.

- [ ] **Step 4: Run catalog and storage tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_catalog --test download_storage`

Expected: all tests pass.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/download/catalog.rs src-tauri/src/download/storage.rs src-tauri/src/download/mod.rs src-tauri/tests/download_catalog.rs
git commit -m "feat: create deduplicated download job snapshots"
```

### Task 4: Safe Paths, Asset Ownership, And Recovery State Machine

**Files:**
- Create: `src-tauri/src/download/paths.rs`
- Create: `src-tauri/src/download/recovery.rs`
- Modify: `src-tauri/src/download/storage.rs`
- Create: `src-tauri/tests/download_paths.rs`
- Create: `src-tauri/tests/download_recovery.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `build_work_directory(candidate: &DownloadCandidate) -> PathBuf`
- Produces: `asset_ownership_key(root: &Path, aweme_id: &str, stable_key: &str, quality: &str) -> String`
- Produces: `partial_filename(asset_id: i64, nonce: u64, extension: &str) -> String`
- Produces: `recover_download_jobs(connection: &mut Connection, roots: &dyn RootAccess) -> Result<RecoverySummary, DownloadError>`

- [ ] **Step 1: Write failing path and recovery tests**

Test Windows reserved names, invalid characters, trailing dots, path length, containment, duplicate ownership keys, unique partial names, and every recovery transition from the specification.

```rust
assert_eq!(sanitize_component("CON. "), "_CON");
assert_ne!(partial_filename(1, 10, "mp4"), partial_filename(2, 10, "mp4"));
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_paths --test download_recovery`

Expected: compilation fails because path and recovery modules do not exist.

- [ ] **Step 3: Implement deterministic paths and recovery matrix**

Add required Windows handle flags to `windows-sys`. Keep a small `RootAccess` abstraction so unit tests use temporary directories while Windows production validates reparse points at creation and rename.

Implement recovery as one transaction that converts running rows, then filesystem reconciliation in bounded operations, followed by a second transaction with the observed results.

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_paths --test download_recovery`

Expected: all tests pass on Windows; platform-neutral sanitization tests pass everywhere.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/Cargo.toml src-tauri/src/download/paths.rs src-tauri/src/download/recovery.rs src-tauri/src/download/storage.rs src-tauri/tests/download_paths.rs src-tauri/tests/download_recovery.rs
git commit -m "feat: secure download paths and recovery"
```

### Task 5: Persistence Actor And Single-Active-Job Queue

**Files:**
- Create: `src-tauri/src/download/persistence.rs`
- Create: `src-tauri/src/download/queue.rs`
- Create: `src-tauri/tests/download_queue.rs`
- Modify: `src-tauri/src/download/storage.rs`
- Modify: `src-tauri/src/download/mod.rs`

**Interfaces:**
- Produces: `PersistenceHandle::transition(DownloadMutation) -> Result<DownloadEvent, DownloadError>`
- Produces: `DownloadQueue::start_next`, `pause`, `resume`, `cancel`, `retry_failed`
- Guarantees: one active job, post-commit events, derived aggregate counters

- [ ] **Step 1: Write failing actor and queue tests**

Use a temporary file-backed SQLite database and concurrent Tokio tasks. Assert mutations serialize, only one queued job becomes running, aggregate counts equal row state, and failed commits emit no event.

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_queue`

Expected: compilation fails because persistence and queue types do not exist.

- [ ] **Step 3: Implement bounded-channel single writer**

```rust
pub struct PersistenceHandle {
    tx: tokio::sync::mpsc::Sender<PersistenceRequest>,
}

struct PersistenceRequest {
    mutation: DownloadMutation,
    reply: tokio::sync::oneshot::Sender<Result<DownloadEvent, DownloadError>>,
}
```

The actor owns its connection, calls `initialize_schema`, executes short transactions, recomputes job aggregates in SQL, commits, then replies with the event snapshot.

- [ ] **Step 4: Run queue tests and storage regressions**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_queue --test download_storage --test storage_v2`

Expected: all tests pass without `database is locked`.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/download/persistence.rs src-tauri/src/download/queue.rs src-tauri/src/download/storage.rs src-tauri/src/download/mod.rs src-tauri/tests/download_queue.rs
git commit -m "feat: add persistent single-job download queue"
```

### Task 6: Safe HTTP Worker, Range Resume, And Validation

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Create: `src-tauri/src/download/worker.rs`
- Create: `src-tauri/tests/download_worker.rs`
- Modify: `src-tauri/src/download/mod.rs`

**Interfaces:**
- Produces: `AddressPolicy` trait and production `PublicInternetPolicy`
- Produces: `DownloadWorker::transfer(request, cancellation) -> Result<CompletedAsset, DownloadError>`
- Consumes: ownership key and contained paths from Task 4

- [ ] **Step 1: Write a failing in-process HTTP integration suite**

Implement a tiny Tokio TCP fixture with routes for full body, valid Range, ignored Range, changed ETag, redirects, HTML challenge, truncation, timeout, and disconnect. Inject `TestAddressPolicy` that permits only the fixture's exact loopback socket.

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_worker`

Expected: compilation fails because the worker and address policy do not exist.

- [ ] **Step 3: Add HTTP dependencies and minimal worker**

Add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream"] }
tokio-util = { version = "0.7", features = ["rt"] }
bytes = "1"
```

Implement a reqwest DNS resolver that filters all returned addresses at resolution time, a five-redirect policy, HTTP/1 buffer limit, 20 GiB per-asset and 2 TiB per-job counters, Range/If-Range behavior, three retries, MIME/signature validation, flush, and ownership-checked atomic rename.

- [ ] **Step 4: Run worker tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_worker -- --test-threads=1`

Expected: all routes pass, cancellation leaves a valid owned partial, and challenge HTML never becomes final media.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/download/worker.rs src-tauri/src/download/mod.rs src-tauri/tests/download_worker.rs
git commit -m "feat: stream and resume validated media assets"
```

### Task 7: Resolver Contracts, Manifest Reconciliation, And Child IPC

**Files:**
- Create: `src-tauri/src/download/resolver.rs`
- Create: `src-tauri/src/download/resolver_ipc.rs`
- Create: `src-tauri/tests/fixtures/download/*.json`
- Create: `src-tauri/tests/download_resolver.rs`
- Modify: `src-tauri/src/download/storage.rs`
- Modify: `src-tauri/src/download/mod.rs`

**Interfaces:**
- Produces: `MediaResolver` async trait
- Produces: `parse_manifest(Value, DownloadQuality) -> Result<ResolvedMediaManifest, DownloadError>`
- Produces: `reconcile_manifest(connection, item_id, manifest) -> Result<Vec<AssetAction>, DownloadError>`
- Produces: length-delimited `ResolverRequest`, `ResolverResponse`, `ResolverControl`

- [ ] **Step 1: Write failing fixture and reconciliation tests**

Fixtures cover video formats, image plus music, paired live photos, missing fields, unsupported content, expiring request metadata, new assets in generation two, and superseded generation-one assets.

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_resolver`

Expected: compilation fails because resolver modules do not exist.

- [ ] **Step 3: Implement parser, generation reconciliation, and framed IPC**

Use a four-byte little-endian length followed by strict UTF-8 JSON. Reject frames over 8 MiB. Request metadata is deserialized only in parent memory and never passed to storage functions.

- [ ] **Step 4: Run resolver tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_resolver`

Expected: all fixtures, IPC round trips, size rejection, and generation reconciliation pass.

- [ ] **Step 5: Commit**

```powershell
git add src-tauri/src/download/resolver.rs src-tauri/src/download/resolver_ipc.rs src-tauri/src/download/storage.rs src-tauri/src/download/mod.rs src-tauri/tests/fixtures/download src-tauri/tests/download_resolver.rs
git commit -m "feat: resolve and reconcile download manifests"
```

### Task 8: Managed Browser Resolver Child And Desktop Service

**Files:**
- Create: `src-tauri/src/download/browser_adapter.rs`
- Create: `src-tauri/src/download/service.rs`
- Modify: `src-tauri/src/collector/cdp.rs`
- Modify: `src-tauri/src/collector/session.rs`
- Modify: `src-tauri/src/collector/browser_runtime.rs`
- Modify: `src-tauri/src/main.rs`
- Create: `src-tauri/tests/download_resolver_child.rs`
- Modify: `src-tauri/tests/cdp_system_browser.rs`

**Interfaces:**
- Produces: `--download-resolver` executable mode
- Produces: `DownloadService` owned by `AppState`
- Produces: background-owned `BrowserOperationLease`
- Extends: CDP Network enable, response event routing and `Network.getResponseBody`

- [ ] **Step 1: Write failing child-process and CDP routing tests**

Use an in-process fixture page whose detail endpoint returns sanitized JSON. Assert the resolver child emits one manifest frame, media request bodies are not downloaded by the browser, cancellation closes the browser, and the browser lease remains held until child exit.

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_resolver_child -- --test-threads=1`

Expected: failure because the child mode and Network API do not exist.

- [ ] **Step 3: Implement Network routing and resolver child mode**

Add CDP methods for Network enable/disable and response-body retrieval without changing synchronization's Fetch policy. Dispatch `--download-resolver` before Tauri initialization, mirror collector process identity and cleanup rules with download-specific filenames, and keep pipe writes framed and sanitized.

- [ ] **Step 4: Implement desktop `DownloadService` orchestration**

The service owns queue, persistence handle, cancellation tokens, active resolver child, browser lease and worker tasks. Tauri command functions only call service methods.

- [ ] **Step 5: Run native browser and service tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_resolver_child --test cdp_system_browser -- --test-threads=1`

Expected: fixture tests pass; ignored real-browser cases remain ignored unless explicitly selected.

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/download/browser_adapter.rs src-tauri/src/download/service.rs src-tauri/src/collector/cdp.rs src-tauri/src/collector/session.rs src-tauri/src/collector/browser_runtime.rs src-tauri/src/main.rs src-tauri/tests/download_resolver_child.rs src-tauri/tests/cdp_system_browser.rs
git commit -m "feat: run authenticated media resolver child"
```

### Task 9: Tauri Commands, Dialog Capability, Frontend Module, And Full Verification

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/capabilities/default.json`
- Create: `app/src/download/types.ts`
- Create: `app/src/download/api.ts`
- Create: `app/src/download/settings.ts`
- Create: `app/src/download/selection.ts`
- Create: `app/src/download/DownloadStartDialog.svelte`
- Create: `app/src/download/DownloadTaskCenter.svelte`
- Modify: `app/src/App.svelte`
- Modify: `app/src/styles.css`
- Modify: `app/src/types.ts`
- Modify: `app/src/app-api.ts`
- Create: `tests/app/download-selection.test.ts`
- Create: `tests/app/download-settings.test.ts`
- Modify: `dist/**`
- Modify: `dist/build-manifest.json`

**Interfaces:**
- Produces commands: `download_create_job`, `download_list_jobs`, `download_get_job`, `download_pause_job`, `download_resume_job`, `download_cancel_job`, `download_retry_failed`, `download_reveal_path`
- Produces events: `download_job_updated`, `download_item_updated`
- Adds: `tauri-plugin-dialog` with directory-selection-only capability

- [ ] **Step 1: Write failing frontend pure-logic tests**

Cover checked IDs, filtered request, all-history request, duplicate selection, quality normalization, default directory normalization, and independent ask flags.

```ts
expect(buildDownloadSelection({ mode: "filtered", query: "猫" })).toEqual({
  kind: "filtered",
  query: "猫",
});
```

- [ ] **Step 2: Run frontend tests and verify failure**

Run: `npm test -- --run tests/app/download-selection.test.ts tests/app/download-settings.test.ts`

Expected: tests fail because download frontend modules do not exist.

- [ ] **Step 3: Register commands, dialog plugin and restricted reveal helper**

Add `tauri-plugin-dialog = "2"`, initialize it in the builder, grant only the folder-open permission required by the main window, and register namespaced commands. `download_reveal_path` accepts IDs only and resolves paths in the backend before invoking Explorer.

- [ ] **Step 4: Implement frontend modules and minimal App integration**

Keep selection/request/settings logic in pure TypeScript. `App.svelte` imports the two download components, adds selection checkboxes and three toolbar actions, and listens for namespaced events by refreshing backend state rather than treating event payloads as authoritative.

- [ ] **Step 5: Run frontend tests and type checking**

Run: `npm test`

Expected: all Vitest tests pass.

Run: `npm run typecheck`

Expected: TypeScript exits successfully.

- [ ] **Step 6: Regenerate committed frontend output**

Run: `npm run app:build`

Expected: Vite build succeeds and `dist/build-manifest.json` matches all source and output hashes.

- [ ] **Step 7: Run full Rust verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all default Rust tests pass.

Run: `cargo tauri build`

Expected: the Windows x64 package builds from committed `dist` without adding Python, Go, Node, FFmpeg, Chromium, or another sidecar runtime.

- [ ] **Step 8: Run explicit real-browser acceptance**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test download_resolver_child -- --ignored --test-threads=1`

Expected: with an explicitly prepared test profile, a public normal video resolves and downloads; image and live-photo fixtures remain covered deterministically if live examples are unavailable.

- [ ] **Step 9: Review final scope and commit**

Run: `git diff --check` and `git status --short`.

Confirm only download-module integration, dependency, test, source and regenerated `dist` files are included.

```powershell
git add src-tauri app tests dist package-lock.json
git commit -m "feat: add modular batch download workflow"
```

### Task 9 Review Revision

- [ ] Add a failing service behavior test proving `create_job` schedules the queued job and runs the existing resolver → manifest reconciliation → at-most-three concurrent asset worker pipeline, with durable post-commit state/events and working pause/resume/cancel semantics.
- [ ] Add one Windows-only `#[ignore]` authenticated acceptance test that requires an explicit isolated browser profile and public normal-video URL, never falls back to the normal user profile, and performs both real resolution and real asset download. The ignored command must enumerate this case; missing configuration must produce an explicit environment-blocked result.
- [ ] Add a focused frontend test for disposal before asynchronous event-listener registration resolves, then use a subscription helper that immediately unregisters late listeners and prevents refresh after disposal.

Revision acceptance requires fresh passing frontend tests/typecheck/build, default Rust tests, Windows Tauri packaging, and an ignored acceptance run that executes exactly the explicit fixture (passing only when the configured profile/URL are available; otherwise reported as an external environment block rather than as zero tests).

#### Second Review Acceptance

- [ ] Every non-cancellation failure in item resolution, manifest reconciliation, transfer-request loading, or asset persistence converges to a durable item failure with its stable error code and concise Chinese message. If that item mutation cannot commit, the job is durably interrupted with a stable failure reason and the scheduler exits without spinning.
- [ ] Scheduler database errors are never treated as an empty queue. Running/queued lookup uses bounded retry and then an explicit recoverable job state or a reliable wake path; deterministic fault injection proves a transient lookup failure cannot permanently strand queued work and repeated failures cannot busy-loop.
- [ ] Focused fault-injection tests cover representative resolver, reconcile/request-loading, StartAsset, and CompleteAsset/FailAsset persistence failures, and assert no job remains `running` with an item stuck in `resolving` or `downloading`.

The report must supersede the original zero-test acceptance note, preserve concrete RED→GREEN evidence, and repeat the full frontend, Rust, package, and explicit ignored acceptance commands.

#### Third Review Reliability Revision

- [ ] Keep the existing single scheduler worker alive across database-query failures. Retry with a bounded, capped delay while the process is alive, reset the delay after a successful database operation, and release the scheduler only after a successful empty-queue observation. Deterministic tests must prove recovery after more than the initial retry budget and a bounded call rate during a persistent failure, without a new user action.
- [ ] Make asset persistence batch-safe. `StartAsset`, `CompleteAsset`, and `FailAsset` update only their individual asset rows while the job remains active; after every transfer future in the item batch returns, one atomic finalization mutation stores the first stable error code and Chinese message, moves every remaining queued/downloading asset to a terminal state, derives the item, and only then derives the job. A four-asset/three-concurrent-transfer test must cover mixed success/failure and assert no active assets remain.
- [ ] Run the existing offline `recover_download_jobs` path during production startup after database/path preparation but before constructing the resolver/worker runtime. Startup recovery must not schedule, resolve, or transfer anything. Interrupted work may access the network only after an explicit manual resume/start action; focused tests must assert zero adapter calls before that action.
- [ ] Wrap the direct emergency interruption fallback in one `IMMEDIATE` transaction covering job, item, and asset updates. Commit only after all statements succeed; retry the whole transaction a bounded number of times. Fault-injection and trigger-backed rollback tests must prove both the actor-fallback path and absence of half-interrupted state.
- [ ] Catch asynchronous listener-registration rejection. If the second Tauri listener fails, immediately unregister the first; disposal and registration failure must not leak listeners or produce an unhandled rejection.

Third-review acceptance requires isolated RED→GREEN evidence for every item, followed by fresh `npm test`, `npm run typecheck`, `npm run app:build`, `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo tauri build`, and the explicit ignored resolver-child acceptance command. The report and final handoff must state the actual one-case environment-blocked result when the isolated authenticated profile is unavailable.

#### Final Scheduler Handoff Revision

- [ ] Keep scheduler ownership while `HasRunnableJob` is failing. Retry the handoff query inside the same worker after a 25 ms delay that grows exponentially to a 400 ms cap; only a successful result may reset that handoff delay or permit the worker to exit.
- [ ] Preserve independent capped backoff state for `QueryRunning`, `StartNext`, and `HasRunnableJob`. A successful `QueryRunning` call must not reset repeated `StartNext` failures, and a successful empty `StartNext` result must not erase a pending handoff failure delay.
- [ ] Add public deterministic fault injection at `StartNext` and focused tests proving: one `HasRunnableJob` failure sleeps then recovers without an external wake; persistent handoff failure has a bounded call rate without mutating the completed job; persistent `StartNext` failure has a bounded call rate and leaves the queued job recoverable.

Final scheduler acceptance requires focused `download_service` RED→GREEN evidence and the complete frontend, Rust, Tauri package, and explicit one-case ignored acceptance matrix.

#### Final Branch Review Remediation

- [ ] Replace the fixture-only browser-response assumption with a tolerant upstream adapter for the recognized `/aweme/v1/web/aweme/detail/` response shape. Extract `aweme_detail`, accept unrelated upstream fields, verify the requested aweme ID, map video/gallery/live-photo/music sources to the strict internal manifest, and continue observing later candidate responses when an earlier JSON candidate is unrelated or invalid. Add a sanitized nested response fixture based on MIT-licensed protocol references, without copying their implementation.
- [ ] Close the Windows containment race for temporary-file creation and finalization. Create files relative to a verified parent directory handle or prevent parent move/delete for the entire guarded lifetime, and verify the same parent identity before final rename. Tests must cover reparse rejection and attempted directory replacement/move.
- [ ] Implement cross-job asset reuse keyed by destination root, aweme ID, stable asset key and quality. Persist SHA-256 for completed files, reopen and validate the actual target before retaining it, mark valid assets skipped, and redownload only missing or corrupt assets. Existing-file finalization must never turn a valid duplicate into a rename failure.
- [ ] Reuse compatible paused/interrupted partial assets across restart or manifest generation. Preserve partial ownership, downloaded length and validator only when stable identity and quality match; add a real byte-level Range continuation test that proves the retained prefix is not downloaded again.
- [ ] Treat expired source timestamps and HTTP 401/403 as one bounded forced re-resolution per item. Revalidate aweme identity and stable asset type before retrying and forbid refresh loops.
- [ ] Add an explicit manual start/resume path for queued and interrupted jobs. Production startup recovery remains offline, while the application exit lifecycle must stop new scheduling, cooperatively cancel active work and durably pause it before teardown.
- [ ] Add a backend preview command returning the deduplicated candidate count and known content-type distribution. Extend job detail with asset rows and aggregate current resolver/transfer, speed and cancelled-asset information, then render those backend-authoritative values in the confirmation dialog and task center.
- [ ] Scope the 2 TiB write quota to one job execution instead of the singleton worker lifetime.
- [ ] Document branch ancestry without rewriting history: the download branch and the active stage-three branch diverge from `3bd1e24`; the active stage-three branch has 21 account-scoping commits while the download branch has 33 download commits. Integration therefore requires an explicit future rebase/cherry-pick decision after both branches stabilize.

The final remediation must use deterministic RED→GREEN tests, record the GitHub protocol references and licenses, regenerate committed frontend output, and repeat the complete frontend/Rust/Tauri/ignored-browser verification matrix. The ignored browser case remains an external environment gate when the explicit isolated authenticated profile and public URL are not configured.

## Final Review Gate

After Task 9, compare every acceptance criterion in `docs/superpowers/specs/2026-07-12-douyin-batch-download-design.md` against a passing test or an explicit manual acceptance result. Do not claim the feature complete if the real authenticated resolver cannot be verified; report it as the remaining blocked acceptance item while keeping all fixture-backed functionality and commits intact.
