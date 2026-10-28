# Tray, Onboarding, and Background Scheduler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Project rules forbid implementation subagents; execute every implementation task inline in this session.

**Goal:** Deliver stage 3.3 as a low-resource Windows resident application with persistent sync jobs, history scheduling, tray lifecycle, first-run onboarding, current-user autostart, deterministic recovery, and uninstall-safe desktop state.

**Architecture:** Keep SQLite and durable JSON files as the recovery sources of truth, while small in-memory indexes, condition variables, and Tauri state coordinate the live process. Split the current oversized `main.rs` and `lib.rs` responsibilities into focused settings, queue, scheduler, admission, executor, recovery, desktop, and uninstall modules; retain the Rust-native CDP collector and expose only the history executor in 3.3. The Svelte frontend renders backend state and consumes Tauri events, but it never owns scheduling timers or queue truth.

**Tech Stack:** Rust 2021, Tauri 2, Svelte 5, TypeScript 5, SQLite/rusqlite, native CDP collector, Windows HKCU registry, Windows named mutex/events, NSIS, Vitest, Cargo test.

## Global Constraints

- The sole requirements baseline is `docs/superpowers/specs/2026-08-11-tray-onboarding-background-scheduler-design.md`.
- Production runtime and formal packaging remain Node-free; `npm` is only an optional frontend development tool.
- Windows x64 and current-user installation are the only supported desktop target in this stage; do not request elevation, write HKLM, create a service, or create a scheduled task.
- `settings.json` becomes portable schema v2; `<system_app_data>/device-settings.json` uses device schema v1.
- SQLite becomes schema v5 without clearing supported multi-account schema v3/v4 accounts, videos, or history records. Pre-multi-account v0-v2 fixtures retain the stage 3.1-approved development reset semantics when they first pass through schema v3; the v5 migration itself must not introduce an additional reset.
- Register only `HistorySyncExecutor` in 3.3. Likes, favorites, and remote-folder capabilities must remain present but unavailable with reason `collector_not_implemented`.
- All queued and running jobs must contain strict payload identity; all payload, execution-equivalence, and automatic-config hashes use canonical UTF-8 JSON and lowercase SHA-256.
- The only legal GUI startup order is uninstall journal/fault recovery, bootstrap/data root resolution, schema migration, shutdown/queue recovery, tray/window decision, then scheduler start.
- The main Tauri window starts with `visible: false`; manual launch shows it only after startup recovery, while silent autostart keeps it hidden only when onboarding and tray gates pass.
- All data-root maintenance, settings changes, login, enqueue/dequeue, importer completion, cancellation, and exit transitions pass through one `OperationAdmission` state machine.
- Frontend source changes must be accompanied by regenerated, Git-tracked `dist/index.html`, `dist/assets/**`, and `dist/build-manifest.json`.
- Do not delete legacy sync/recovery code until replacement tests pass and the production call path uses the replacement.
- Do not claim likes, favorites, or remote-folder collection is available in 3.3.
- Do not claim stage 3.3 release-ready until the stage 3.2 real-installation gates and this plan's manual installer/resource gates pass.

## File And Module Map

**Portable and device settings**

- Modify `src-tauri/src/settings_store.rs`: portable schema v2 contracts, strict v1 migration, account-default updates, revision snapshots.
- Create `src-tauri/src/device_settings.rs`: device schema v1, atomic store, revision CAS, onboarding compatibility decision.
- Create `src-tauri/src/autostart.rs`: HKCU Run command construction, readback, compensation, startup repair.
- Test in `src-tauri/tests/settings_storage.rs` and new `src-tauri/tests/device_settings.rs`.

**Database and sync domain**

- Create `src-tauri/migrations/005_sync_queue.sql`: schema v5 tables, indexes, and immutability triggers.
- Create `src-tauri/src/schema.rs`: startup-only schema v5 migration, reconstructible post-migration cleanup work, and structural assertions; ordinary runtime code only receives `assert_schema_v5`.
- Create `src-tauri/src/sync/mod.rs`: sync-domain exports.
- Create `src-tauri/src/sync/contracts.rs`: strict IPC, payload, range, capability, result-envelope, issue, and status types.
- Create `src-tauri/src/sync/hashing.rs`: canonical payload, execution-equivalence, automatic-config, job-identity, and cursor hashes.
- Create `src-tauri/src/sync/capabilities.rs`: fixed four-source capability registry.
- Create `src-tauri/src/sync/store.rs`: job, batch, issue, registry-revision, cursor, and transition persistence.
- Create `src-tauri/src/sync/enqueue.rs`: all task-key enqueue, promotion, replacement, attachment, conflict, bulk, and limit decisions.
- Create `src-tauri/src/sync/scheduler.rs`: due calculation, source-instance expansion, pause reconciliation, state views, and worker wake decisions.
- Create `src-tauri/src/sync/admission.rs`: operation modes, browser lease, owned operation registry, continuation tokens, update gates.
- Create `src-tauri/src/sync/worker.rs`: persistent queue index, dequeue loop, executor launch, cancellation, and wake handling.
- Create `src-tauri/src/sync/executor.rs`: executor/importer traits, registry, and test fake seam in Task 4; history registration is added in Task 10.
- Create `src-tauri/src/sync/history.rs`: history payload-to-collector adapter and atomic history importer.
- Test in new `src-tauri/tests/storage_v5.rs`, `sync_contracts.rs`, `sync_queue.rs`, `sync_scheduler.rs`, and `sync_lifecycle.rs`.

**Startup, desktop, and installer lifecycle**

- Create `src-tauri/src/startup_recovery.rs`: shutdown intent/fault contracts and deterministic queue recovery.
- Create `src-tauri/src/desktop_lifecycle.rs`: startup decision, desktop intents, controlled exit, tray/window decision functions.
- Create `src-tauri/src/notifications.rs`: notification policy and failure-streak deduplication.
- Create `src-tauri/src/uninstall.rs`: uninstall manifest, journal, guard, rollback, repair takeover, and startup gate.
- Modify `src-tauri/src/process_lock.rs`: activation event and uninstall mutex support.
- Modify `src-tauri/src/main.rs`: early mode split, one startup coordinator, AppState ownership, commands, event wiring.
- Modify `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/nsis/installer-hooks.nsh`, and `src-tauri/build.rs`.
- Create `src-tauri/nsis/installer-template.nsi`: pinned Tauri 2.11.4 NSIS template with pre-`/UPDATE` recovery routing and compile-time package-manifest generation.
- Create `src-tauri/build_support/uninstall_manifest.rs` and tests `desktop_lifecycle.rs`, `startup_recovery.rs`, `uninstall_lifecycle.rs`.
- Create `src-tauri/build_support/package_identity.rs`: deterministic source-input build ID embedded into the EXE, uninstall manifest, and journal.

**Frontend**

- Modify `app/src/types.ts`: v2 settings, queue, batch, capability, scheduler, device, and onboarding types.
- Modify `app/src/app-api.ts`: new commands, events, and demo-state adapters.
- Create `app/src/scheduler.ts`: event-driven refresh/polling policy and stable error labels.
- Create `app/src/onboarding.ts`: four-step state reducer, validation, and request builders.
- Modify `app/src/App.svelte`: onboarding overlay, scheduler settings, queued/partial states, desktop behavior, and visibility-aware refresh.
- Modify `app/src/App.test.ts` and create `app/src/onboarding.test.ts`, `app/src/scheduler.test.ts`.

---

### Task 1: Upgrade Portable Settings To Schema V2

**Files:**
- Modify: `src-tauri/src/settings_store.rs`
- Modify: `src-tauri/tests/settings_storage.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Produces: `PortableSettingsV2`, `GlobalSettingsV2`, `AccountSettingsV2`, `HistoryRangeV1`, `ArchiveListRangeV1`, `SourceScheduleV1`, `FavoriteSourceSettingsV1`.
- Produces: `load_portable_settings(root: &Path) -> Result<PortableSettingsV2, String>` with strict v1-to-v2 migration.
- Produces: `update_account_defaults(root: &Path, value: AccountSettingsV2) -> Result<PortableSettingsV2, String>`.
- Preserves: existing `get_app_settings`, `update_global_settings`, `update_account_settings`, and legacy frontend import command names.

- [ ] **Step 1: Write failing schema-v2 contract and migration tests**

Add exact tests covering:

```rust
#[test]
fn portable_v1_is_atomically_migrated_to_v2_without_enabling_background_sync() {
    // Write a valid schemaVersion=1 file with bottom mode, 4_800 scroll speed,
    // hidden browser, custom display fields, defaults, and one account override.
    // Load it through load_portable_settings.
    // Assert schemaVersion=2, history.range=bottom, all schedules disabled,
    // intervalMinutes=360, closeBehavior=exit, startupSyncDelayMinutes=10,
    // and all original display/browser/scroll fields preserved.
}

#[test]
fn portable_v2_rejects_unknown_fields_and_every_documented_range_boundary() {
    // Assert days 0/366, interval 14/43_201, recentItems 0/5_001,
    // recordLimit 0/50_001, recentDays 0/366, unsafe account keys,
    // and more than 1_000 remote-folder overrides fail with stable codes.
}
```

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test settings_storage portable_v1_is_atomically_migrated_to_v2_without_enabling_background_sync -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --test settings_storage portable_v2_rejects_unknown_fields_and_every_documented_range_boundary -- --exact
```

Expected: compile failure because schema-v2 types and fields do not exist.

- [ ] **Step 3: Implement strict v2 contracts and migration**

Use these defaults and bounds in one validator:

```rust
pub const DEFAULT_INTERVAL_MINUTES: u32 = 360;
pub const DEFAULT_STARTUP_SYNC_DELAY_MINUTES: u32 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum HistoryRangeV1 {
    Days { days: u32 },
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ArchiveListRangeV1 {
    All,
    RecentItems { count: u32 },
    RecordLimit { limit: u32 },
    RecentDays { days: u32 },
}
```

Deserialize by first reading `schemaVersion` from a strict small version envelope, parse v1 or v2 into typed structs, convert v1 in memory, validate v2, then atomically replace the original file only after successful serialization. Preserve the original bytes on any failure. Keep `revision` unchanged during the schema-only migration; subsequent user writes increment it once.

- [ ] **Step 4: Update settings views and compatibility wrappers**

Change `AppSettingsView.account` to `AccountSettingsV2`, add `accountDefaults`, and map the old `SyncOptions` compatibility wrapper from `account.history.range`. `update_account_defaults` must share the same RMW lock and overflow check as account/global updates.

- [ ] **Step 5: Run settings and existing storage regressions**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test settings_storage
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3
```

Expected: all tests pass; v1 files are migrated once and invalid files are untouched.

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/settings_store.rs src-tauri/src/main.rs src-tauri/tests/settings_storage.rs
git commit -m "feat: upgrade portable settings to v2"
```

### Task 2: Add Device Settings And Onboarding Compatibility

**Files:**
- Create: `src-tauri/src/device_settings.rs`
- Create: `src-tauri/tests/device_settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/collector/browser_state.rs`

**Interfaces:**
- Produces: `DeviceSettingsStore::open(system_app_data: PathBuf) -> Result<Self, String>`.
- Produces one public write API: `compare_and_swap(expected_revision: u64, update: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>) -> Result<DeviceSettingsV1, String>`.
- Produces: `initialize_onboarding_state(connection: &Connection, portable: &PortableSettingsV2) -> Result<DeviceSettingsV1, String>`.
- Produces: `DeviceAccountSettingsV1 { browser_preference }` keyed by lowercase simple UUID `profile_key`.

- [ ] **Step 1: Write failing atomic-store and compatibility tests**

Cover missing-file defaults, malformed JSON preservation, unknown fields, onboarding step 0..3, revision overflow, concurrent CAS conflict, upgrade-user detection from accounts/watch records/completed jobs, portable revision detection, new-install detection, and notification-key hashing that never writes raw remote-folder IDs.

- [ ] **Step 2: Confirm RED**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test device_settings
```

Expected: module and types are missing.

- [ ] **Step 3: Implement the store and strict contracts**

Define:

```rust
pub struct DeviceSettingsStore {
    path: PathBuf,
    rmw: Mutex<()>,
}

pub fn compare_and_swap(
    &self,
    expected_revision: u64,
    f: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>,
) -> Result<DeviceSettingsV1, String>;
```

Write through a same-directory temporary file and `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` via the existing atomic-file helper. Return `device_settings_revision_conflict`, `device_settings_revision_overflow`, or `invalid_device_settings` without replacing the original file. Internal notification/tray/recovery mutations may use a private `update_internal` method under the same RMW lock; it is not exported from the module or reachable from IPC.

- [ ] **Step 4: Add durable profile generations**

Store one lowercase simple UUID in `<profile_key>/<browser>/profile-generation.json`. Require browser selection and login validation markers to carry the same generation. Add typed readers that return `browser_profile_changed` when the payload identity, selection, marker, path, or generation differs.

- [ ] **Step 5: Put the store in AppState**

Initialize the store from the system app-data directory, not the movable data root. Run the one-time onboarding compatibility decision only after schema initialization and persist the result before tray/scheduler startup.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test device_settings
cargo test --manifest-path src-tauri/Cargo.toml --test browser_native
```

Then:

```powershell
git add src-tauri/src/device_settings.rs src-tauri/src/lib.rs src-tauri/src/main.rs src-tauri/src/collector/browser_state.rs src-tauri/tests/device_settings.rs src-tauri/tests/browser_native.rs
git commit -m "feat: add durable device settings"
```

### Task 3: Implement Current-User Autostart Transactions

**Files:**
- Create: `src-tauri/src/autostart.rs`
- Create: `src-tauri/tests/autostart.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `AutostartStateV1 { enabled, command, executable_matches }`.
- Produces: `expected_run_command(exe: &Path) -> Result<String, String>`.
- Produces: `set_autostart_transactional(store, desktop_settings_tx, exe, enabled, silent) -> Result<DeviceSettingsV1, String>`.
- Uses HKCU `Software\Microsoft\Windows\CurrentVersion\Run`, value `LifeArchiveProject.Emily`.

- [ ] **Step 1: Write failing command, compensation, and repair tests**

Assert the command is exactly `"<absolute exe>" --autostart`, paths with spaces remain quoted, disabling removes only the fixed Emily value, registry write/readback failure leaves device settings unchanged, device write failure restores the prior registry value, and startup repair rewrites a stale EXE path only when device settings say enabled.

- [ ] **Step 2: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test autostart` and expect missing-module failures.

- [ ] **Step 3: Implement registry abstraction and real backend**

Use a small trait for deterministic tests:

```rust
pub trait RunRegistry: Send + Sync {
    fn read_value(&self) -> Result<Option<String>, String>;
    fn write_value(&self, value: &str) -> Result<(), String>;
    fn delete_value(&self) -> Result<(), String>;
}
```

The production implementation uses `winreg` under HKCU. Every mutation must read back and compare the exact string. Return `autostart_registry_update_failed` for write/readback failure and `autostart_state_uncertain` only when compensation cannot restore and verify the original value.

- [ ] **Step 4: Add a serialized cross-resource transaction**

Use a dedicated `desktop_settings_tx: Mutex<()>`: read old registry/device state, update and verify registry, write device settings, compensate registry if the device write fails. Do not hold the device-store RMW mutex while calling registry APIs.

- [ ] **Step 5: Run deterministic and ignored real-registry tests**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test autostart
cargo test --manifest-path src-tauri/Cargo.toml --test autostart real_hkcu_round_trip -- --ignored --exact --test-threads=1
```

The ignored test must use a unique temporary value name and remove it in a drop guard.

- [ ] **Step 6: Commit**

```powershell
git add src-tauri/src/autostart.rs src-tauri/src/lib.rs src-tauri/src/main.rs src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tests/autostart.rs
git commit -m "feat: add current user autostart"
```

### Task 4: Freeze Sync Contracts, Capabilities, And Canonical Hashes

**Files:**
- Create: `src-tauri/src/sync/mod.rs`
- Create: `src-tauri/src/sync/contracts.rs`
- Create: `src-tauri/src/sync/hashing.rs`
- Create: `src-tauri/src/sync/capabilities.rs`
- Create: `src-tauri/src/sync/executor.rs`
- Create: `src-tauri/tests/sync_contracts.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces all schema-v1 payload, range-freeze, browser identity, equivalence, automatic-config, enqueue, job, batch, capability, execution-envelope, and source-result types from design sections 5.2 and 5.4.
- Produces stable `TaskKeyV1` and `SyncJobSnapshotV1` identity types before hashing and validation functions consume them.
- Produces `canonical_payload`, `canonical_equivalence`, `canonical_automatic_config`, `job_identity_sha256`, and strict source/instance mapping functions.
- Produces `CapabilityRegistry::stage_3_3()` returning all four sources with only history available.
- Produces stable `SyncExecutor`, `SyncResultImporter`, and `ExecutorRegistry` interfaces plus an in-memory fake executor/importer used by Task 9 worker tests; no production source is registered yet.

- [ ] **Step 1: Write failing strict-serde and golden-vector tests**

Include fixture JSON for every source and range. Assert unknown fields at the envelope, tagged enum, nested payload, range, record, issue, and result levels fail. Assert JSON tags and SQLite mappings exactly match the design. Add stable expected lowercase hashes for history, likes, favorites with sorted overrides, and remote-folder automatic config.

- [ ] **Step 2: Add range and cross-field RED cases**

Test mixed/illegal `EffectiveRangeEquivalenceV1` fields, missing explicit nulls, duplicate or unsorted overrides, RecentDays cutoff mismatch, unsafe SHA-256, history/likes/favorites nonempty instance, remote-folder empty/mismatched instance, automatic archive mode other than Incremental, and FullReconcile without `All` plus matching confirmation hash.

Also freeze the 3.4-only archive result contract without registering an executor: exact record/URL/count bounds, continuous `sourcePosition`, identity/field hash inputs, `notRequested | success`, the three legal folder-discovery boolean/length combinations, and strict serialization for Complete/Incomplete stop reasons. Mark these as contract tests, not evidence that archive collection or import works.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test sync_contracts` and expect missing-module failures.

- [ ] **Step 4: Implement typed canonical projections**

Do not hash `serde_json::Value` maps. Serialize typed structs with declaration-order fields, explicit `Option` nulls, no whitespace, and tag-first internally tagged enums. Build favorites override equivalence as a bytewise-ID-sorted vector and reject noncanonical input before hashing.

Use these public entry points:

```rust
pub fn freeze_task_payload(input: SyncTaskPayloadV1) -> Result<FrozenTaskPayloadV1, String>;
pub fn automatic_config_hash(input: &AutomaticSyncConfigV1) -> Result<String, String>;
pub fn validate_execution_envelope(
    envelope: &SyncExecutionEnvelopeV1,
    job: &SyncJobSnapshotV1,
) -> Result<(), String>;
```

- [ ] **Step 5: Implement the fixed capability registry**

History supports `days` and `bottom`; likes, favorites, and remote-folder return `available=false`, `unavailableReason="collector_not_implemented"`, and their frozen supported range metadata. `supportsRemoteFolders` remains false until favorites and remote-folder executors are both registered.

Define executor traits and a registry in `sync/executor.rs` using only Task 4 contracts. Registry construction rejects duplicate sources. The test fake records start/cancel/import calls and can deterministically return success or a chosen stable launch error; Task 9 consumes this seam without depending on the history implementation.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test sync_contracts
cargo test --manifest-path src-tauri/Cargo.toml --lib sync::
```

Then commit:

```powershell
git add src-tauri/src/sync src-tauri/src/lib.rs src-tauri/tests/sync_contracts.rs
git commit -m "feat: freeze sync contracts and hashes"
```

### Task 5: Migrate SQLite To Schema V5

**Files:**
- Create: `src-tauri/migrations/005_sync_queue.sql`
- Create: `src-tauri/src/schema.rs`
- Create: `src-tauri/tests/storage_v5.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/accounts.rs`
- Modify: existing callers of `initialize_schema_v4`

**Interfaces:**
- Produces: `migrate_schema_to_v5_at_startup(connection: &mut Connection) -> Result<SchemaV5RecoveryWork, String>`; it is not exported to ordinary command adapters.
- Produces: `SchemaV5RecoveryWork { interrupted_v4_jobs: Vec<LegacyRunningJobIdentity> }` for immediate post-commit cleanup and `rebuild_schema_v5_cleanup_work(connection, paths)` for idempotent cleanup after restart.
- Produces: `assert_schema_v5(connection: &Connection) -> Result<(), String>` for ordinary IPC paths; only the startup owner may migrate.
- Preserves every schema v3/v4 sync job ID and every `watch_records.sync_job_id` reference.

- [ ] **Step 1: Write the v0-v4 migration matrix as failing tests**

For v3 and v4 fixtures, assert user_version 5 with no account/video/history loss, original sync job IDs and `watch_records.sync_job_id` references preserved, v4 terminal payload fields null, v4 running jobs failed with `schema_upgrade_interrupted_running`, registry rows initialized, original SQLite sequences preserved, and `foreign_key_check` empty. For v0-v2 fixtures, explicitly assert the existing stage 3.1 transition to schema v3 performs the approved development reset, then produces a valid empty v5 database without inventing a synthetic account or profile identity.

- [ ] **Step 2: Add rollback and invariant tests**

Inject invalid v4 source rows and sequence exhaustion. Assert a v4-to-v5 transaction failure preserves v4 schema/user_version/data and restores the original foreign-key setting. In the end-to-end v0-v3 matrix, if a later stage fails after an earlier committed migration, assert the database remains at the last fully valid committed version and can retry; do not claim rollback to the original pre-multi-account version. Verify all task-key, single-running, target/job/issue immutability, direct-delete, terminal-append, rebind, unlink, and composite-FK triggers emit the exact design error strings.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v5` and expect `migrate_schema_to_v5_at_startup` to be missing.

- [ ] **Step 4: Implement the migration orchestration**

Dispatch by source version: v0-v2 call the existing approved schema-v3 reset and v3-to-v4 migration before v5; v3 preserves all multi-account data through v4 then v5; v4 runs only v5; v5 performs assertions only. For the v4-to-v5 step, save the original `PRAGMA foreign_keys`, disable it before the immediate transaction, insert a bound frozen UTC timestamp into a temporary migration-context table, execute `005_sync_queue.sql`, restore `sqlite_sequence`, assert `sqlite_master`, inspect every `PRAGMA foreign_key_list`, execute `foreign_key_check`, set `user_version=5`, and commit. Restore the original foreign-key state and assert it on both success and error paths.

The SQL must drop the original `sync_jobs` directly and rename `sync_jobs_v5_new` to `sync_jobs`; it must not rename the original table first.

- [ ] **Step 5: Make startup the only migration owner**

Insert `remote_folder_registry_state(account_id, revision=1)` in the same transaction that creates a new account. The current Tauri setup path temporarily owns `migrate_schema_to_v5_at_startup` until Task 13 moves it into `recover_desktop_startup_once`; every ordinary command and background service calls only `assert_schema_v5` and returns `schema_not_initialized` rather than migrating. Immediately after a migration commit, the setup owner consumes `SchemaV5RecoveryWork`. On every schema-v5 startup it also reconstructs pending cleanup from rows whose error code is `schema_upgrade_interrupted_running` plus their existing orchestration/collector/browser identity files. Cleanup is idempotent; success removes the exact identity/task files, so the next reconstruction is empty. Cleanup failure blocks queue recovery and worker startup. Add a test where first cleanup fails, process state is recreated, second startup reconstructs and completes cleanup, and queue recovery has not run before success.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v5
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3
cargo test --manifest-path src-tauri/Cargo.toml --test accounts
```

Then:

```powershell
git add src-tauri/migrations/005_sync_queue.sql src-tauri/src/schema.rs src-tauri/src/lib.rs src-tauri/src/accounts.rs src-tauri/src/main.rs src-tauri/tests/storage_v5.rs src-tauri/tests/storage_v3.rs src-tauri/tests/accounts.rs
git commit -m "feat: migrate storage to sync queue schema v5"
```

### Task 6: Build Persistent Queue And Batch Storage Primitives

**Files:**
- Create: `src-tauri/src/sync/store.rs`
- Create: `src-tauri/tests/sync_queue.rs`
- Modify: `src-tauri/src/sync/mod.rs`

**Interfaces:**
- Consumes: `TaskKeyV1` and `SyncJobSnapshotV1` from Task 4.
- Produces: `QueueSortKeyV1`, `QueueStore`, and transactional job/batch/issue state transitions.
- Produces: `list_batch_entries(batch_id, cursor, limit)` with an opaque cursor bound to batch and `entry_seq`.

- [ ] **Step 1: Write failing queue-sequence and transition tests**

Cover immediate-transaction queue sequence reservation, i64 overflow, manual FIFO, automatic ordering by scheduled time/account/source/instance/queue sequence, global single-running, queued/running CAS, terminal snapshots, job issues, and started-vs-never-started cancellation anchors.

- [ ] **Step 2: Write failing batch status and pagination tests**

Cover the complete status matrix, shared job/issue entry sequence, first 200 entries, active-batch continuation after initially empty pages, cursor tamper/cross-batch use, live-job preference, snapshot fallback after account deletion, and stable issue ordering.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test sync_queue` and expect missing storage APIs.

- [ ] **Step 4: Implement queue storage with transaction-only mutations**

Expose methods that accept `&Transaction` for compound operations and convenience wrappers that open `Immediate` transactions. Every job status update must also refresh all linked `job_view_snapshot_json` values and recalculate affected batch statuses before commit. Do not mutate the in-memory queue until the SQLite transaction commits.

- [ ] **Step 5: Implement stable cursors**

Encode `{schemaVersion:1,batchId,lastEntrySeq}` plus a domain-separated checksum as canonical JSON, then base64url-encode the complete token. Recompute the checksum and bind the decoded batch ID to the command argument before querying. Reject malformed, cross-batch, stale-schema, or modified cursors as `invalid_batch_cursor`. Return a reusable cursor for active batches even when `hasMore=false`; do not add a new mutable secret or device file outside the approved design.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test sync_queue
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v5
```

Then commit:

```powershell
git add src-tauri/src/sync/store.rs src-tauri/src/sync/mod.rs src-tauri/tests/sync_queue.rs
git commit -m "feat: add persistent sync queue storage"
```

### Task 7: Implement Enqueue Decisions, Bulk Expansion, And Limits

**Files:**
- Create: `src-tauri/src/sync/enqueue.rs`
- Create: `src-tauri/tests/sync_enqueue.rs`
- Modify: `src-tauri/src/sync/mod.rs`
- Modify: `src-tauri/src/accounts.rs`

**Interfaces:**
- Produces: `EnqueueService::enqueue(request, settings_snapshot, device_snapshot, capabilities)`.
- Produces: `decide_enqueue_for_task_key_tx(transaction, decision_input) -> TaskDecisionV1` used by single, bulk, scheduler, and future discovery paths.
- Consumes `FrozenTaskPayloadV1`, `AutomaticSyncConfigV1`, `QueueStore`, and capability/source-instance expansion.

- [ ] **Step 1: Write failing request-matrix tests**

Test every legal and illegal combination of `one`, `currentAccountAvailableSources`, and `allActiveAccounts`; source/instance/payload cross-checks; no-active/no-available behavior; implicit Incremental archive payloads; FullReconcile confirmation; unavailable source rejection; and bulk independence from automatic enabled flags.

For both explicit and implicit payloads, reject `RecentDays` with `time_range_not_supported` unless the selected capability declares a reliable remote time field. The failed request must create neither job nor batch.

- [ ] **Step 2: Write failing deduplication and conflict tests**

Cover equivalent attachment, equivalent automatic-to-manual promotion with a new persistent manual `queue_seq`, non-equivalent automatic replacement, running/manual conflict, cross-batch reuse, exact task-key isolation, immutable manual snapshots, and automatic config reconciliation.

- [ ] **Step 3: Write failing all-or-none limit tests**

Cover active batches=16, batch attachments=4, task targets 4,999+1, 4,999+2 aggregate-only, existing 5,000 plus one aggregate entry, initial 5,001 preflight, remote-folder schedule instances=2,000, and deterministic candidate ordering independent of map/DB traversal order.

- [ ] **Step 4: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test sync_enqueue` and expect missing enqueue service failures.

- [ ] **Step 5: Implement one transactional decision engine**

Reserve `sync_batch_targets` before an association or issue. On a uniqueness race, roll back that task-key decision and reread database truth. Use a single transaction for the entire initial bulk expansion; independent task-key conflicts become persisted entries, while storage failure rolls back the whole request. Return only the first 200 stable entries plus counts and cursor.

Update account archive/delete paths to cancel queued jobs and refresh every affected batch snapshot/status in the same transaction. Reject account mutations while a job for that account is running; deleting the account may null live job links through foreign keys but must preserve immutable batch snapshots and issues.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test sync_enqueue
cargo test --manifest-path src-tauri/Cargo.toml --test sync_queue
```

Then commit:

```powershell
git add src-tauri/src/sync/enqueue.rs src-tauri/src/sync/mod.rs src-tauri/src/accounts.rs src-tauri/tests/sync_enqueue.rs
git commit -m "feat: add sync enqueue decisions"
```

### Task 8: Add Scheduler Calculation, Registry Paging, And Automatic Reconciliation

**Files:**
- Create: `src-tauri/src/sync/scheduler.rs`
- Create: `src-tauri/tests/sync_scheduler.rs`
- Modify: `src-tauri/src/sync/mod.rs`
- Modify: `src-tauri/src/settings_store.rs`
- Modify: `src-tauri/src/device_settings.rs`

**Interfaces:**
- Produces: `SchedulerStateV1`, `SchedulerAccountStateV1`, and `SchedulerSourceStateV1`.
- Produces: `calculate_next_due(last_terminal, app_ready_at, interval_minutes) -> DateTime<Utc>`.
- Produces: `expand_source_instances(...)`, `reconcile_automatic_jobs(...)`, and `list_scheduler_instances(...)`.

- [ ] **Step 1: Write failing pure due-time tests**

Assert no history uses `app_ready_at + startup_delay`; started success/partial/failed/cancelled use `finished_at + interval`; cancelled-before-start is ignored; manual completion anchors automatic due time; changed interval recomputes immediately; multiple missed periods enqueue only once.

- [ ] **Step 2: Write failing expansion and revision-cursor tests**

Assert all four capabilities are returned, unavailable sources never enqueue, Unified and PerFolder both traverse all known unremoved registry IDs, override/default resolution is exact, favorites-disabled messaging state is exposed, and any portable or registry revision change invalidates the next page with `scheduler_instances_changed`.

- [ ] **Step 3: Write failing automatic reconciliation tests**

Without starting a worker, verify a changed source-specific automatic config cancels only that key's queued automatic job as `automatic_settings_changed`, a disabled source does not create a replacement, a valid changed snapshot creates at most one replacement, manual queued/running jobs remain frozen, and unrelated display/notification settings do not change `config_sha256`.

- [ ] **Step 4: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test sync_scheduler` and expect missing scheduler APIs.

- [ ] **Step 5: Implement pure calculation and bounded database queries**

Keep the scheduler iteration deterministic and query only aggregate state plus due candidates. Do not materialize all remote folders unless the settings UI requests a page. Use one SQLite read transaction plus portable-settings double-read for each page.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test sync_scheduler
cargo test --manifest-path src-tauri/Cargo.toml --test sync_enqueue
```

Then commit:

```powershell
git add src-tauri/src/sync/scheduler.rs src-tauri/src/sync/mod.rs src-tauri/src/settings_store.rs src-tauri/src/device_settings.rs src-tauri/tests/sync_scheduler.rs
git commit -m "feat: add background sync scheduling"
```

### Task 9: Introduce Operation Admission And The Persistent Worker

**Files:**
- Create: `src-tauri/src/sync/admission.rs`
- Create: `src-tauri/src/sync/worker.rs`
- Create: `src-tauri/tests/sync_lifecycle.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/runtime_paths.rs`
- Modify: `src-tauri/src/data_root.rs`

**Interfaces:**
- Produces: `OperationAdmissionMode::{Open,MaintenancePending,MaintenanceRunning,ExitPending,Exiting}`.
- Produces: `BrowserOperationLease`, `OwnedAdmissionOperation`, `OperationContinuationToken`, source/device/browser update gates, pause pending, and an owned-operation registry.
- Produces: `SyncWorkerHandle { wake, stop, join }` and a serial dequeue loop.

- [ ] **Step 1: Write failing admission state-machine tests**

Cover login-vs-sync browser lease exclusion, source/browser update gates, maintenance immediate refusal, placeholder registration before pending, continuation permissions, maintenance timeout reopening, exit-pending finalizer behavior, abort race with the last owned operation, and no lock held while waiting on path leases.

- [ ] **Step 2: Write failing queue worker tests**

Assert manual priority, automatic stable ordering, paused automatic retention, login-held browser lease leaves jobs queued, queued-to-running final validation, executor-start failure to failed, exact single running, wake on settings/account/terminal change, and a maximum 30-second idle wait.

- [ ] **Step 3: Write failing pause and source-update linearization tests**

Verify `automatic_pause_pending` is set under admission before the device-settings write; no automatic dequeue crosses that barrier; failed writes clear pending and wake the worker; running automatic and all manual work continue; resume revalidates config and never duplicates a key. Add controllable per-task-key barriers for source settings: set `source_settings_update_pending`, atomically write portable settings, cancel/rebuild affected queued automatic work, then clear the gate. Prove an old automatic cannot become running after the source is disabled or its range changes, including the race where dequeue has selected the row but has not completed final validation.

- [ ] **Step 4: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test sync_lifecycle` and expect missing admission/worker APIs.

- [ ] **Step 5: Implement admission without nested waits**

Keep one mutex-protected state and return owned handles that finalize through a single idempotent path. Clone cancellation tokens and completion handles under the lock, then release it before waiting. A continuation token authorizes only progress, importer, batch-child association, and terminal DB writes for its bound job/task identity.

Route portable source-setting updates, autostart updates, browser-preference changes, and profile resets through bounded owned operations. Autostart uses `desktop_settings_tx`; browser preference/reset additionally holds the account browser-update gate and rejects active jobs, login, or browser leases with `account_browser_operation_in_progress`.

- [ ] **Step 6: Implement worker recovery index and lifecycle lease ownership**

Build the in-memory queue from validated SQLite queued rows after startup recovery. Move `BrowserOperationGuard` ownership into `RunningSync`/`RunningLogin`; never release it at command return. Acquire browser lease before the queued-to-running CAS and retain the continuation token until importer/cancellation finalization.

- [ ] **Step 7: Route data-root migration through maintenance**

Replace direct runtime maintenance entry with the design sequence: gate, bounded wait for running/continuations, transactionally cancel all queued including newly created children, then acquire `RuntimePaths` maintenance and migrate. On timeout, restore Open and leave queued rows untouched.

- [ ] **Step 8: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test sync_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml --test data_root
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_paths
```

Then commit:

```powershell
git add src-tauri/src/sync/admission.rs src-tauri/src/sync/worker.rs src-tauri/src/sync/mod.rs src-tauri/src/main.rs src-tauri/src/runtime_paths.rs src-tauri/src/data_root.rs src-tauri/tests/sync_lifecycle.rs src-tauri/tests/data_root.rs src-tauri/tests/runtime_paths.rs
git commit -m "feat: coordinate sync operations and worker"
```

### Task 10: Adapt The History Collector To The Queue And Atomic Importer

**Files:**
- Modify: `src-tauri/src/sync/executor.rs`
- Create: `src-tauri/src/sync/history.rs`
- Create: `src-tauri/tests/history_executor.rs`
- Modify: `src-tauri/src/collector/cli.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/src/collector/contracts.rs`
- Modify: `src-tauri/src/collector/sync_engine.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `SyncExecutor`, `SyncResultImporter`, and `ExecutorRegistry` from Task 4.
- Produces: `HistorySyncExecutor` and `HistoryResultImporter`, then registers only history in the production registry.
- Changes result root to strict `SyncExecutionEnvelopeV1` with nested `SourceExecutionResultV1::History(CollectorSuccessV3)`.

- [ ] **Step 1: Write failing invocation and envelope tests**

Assert a history queued payload generates the exact collector CLI range, frozen cutoff, speed, visibility, browser/profile identity, job ID, source, instance, and payload hash. Assert raw legacy collector result roots, mismatched envelope identity, wrong source tag, unknown fields, and tampered payload hashes are rejected.

- [ ] **Step 2: Write failing importer/CAS race tests**

Cover valid history import, duplicate watch-event upsert, cancelled-before-import zero writes, importer-first success, result transaction rollback, job/batch snapshot update, warning vs incomplete issue semantics, and a short failure transaction only when the job remains running.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test history_executor` and expect missing executor/envelope APIs.

- [ ] **Step 4: Emit the strict execution envelope**

Pass account/job/source/instance/task-payload hash to collector mode. The collector writes only a complete envelope via atomic replacement. Cancellation removes incomplete result files and writes a cancelled diagnostic. Keep all existing history record/hash validation inside the nested history result.

- [ ] **Step 5: Implement executor startup and transactional importer**

The worker marks queued->running, creates orchestration, starts `current_exe --collector`, and records the exact process identity. The importer opens an immediate transaction, rereads the running job, revalidates payload/equivalence/envelope, imports history, writes issues/counts/terminal status, refreshes all batch snapshots/status, and commits through `WHERE id=? AND status='running'`.

Retain the reviewed login-recovery boundary: one hidden recovery attempt and one collection retry for an expired session, no CAPTCHA automation, exact account/profile identity validation, and final `login_expired`/`login_recovery_failed` state only after the bounded retry is exhausted.

- [ ] **Step 6: Replace start_sync with a compatibility enqueue wrapper**

`start_sync(account_id, options)` builds a history `scope=one` request and returns queued/running status. `get_sync_status` uses `SyncStatusV2`; `cancel_sync` handles queued and running jobs.

- [ ] **Step 7: Verify collector and storage regressions**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test history_executor
cargo test --manifest-path src-tauri/Cargo.toml --test native_collector
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_transport
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v2
```

- [ ] **Step 8: Commit**

```powershell
git add src-tauri/src/sync/executor.rs src-tauri/src/sync/history.rs src-tauri/src/sync/mod.rs src-tauri/src/collector src-tauri/src/main.rs src-tauri/src/lib.rs src-tauri/tests/history_executor.rs src-tauri/tests/native_collector.rs src-tauri/tests/storage_v2.rs
git commit -m "feat: run history sync through persistent queue"
```

### Task 11: Implement Shutdown Intent And Startup Queue Recovery

**Files:**
- Create: `src-tauri/src/startup_recovery.rs`
- Create: `src-tauri/tests/startup_recovery.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/sync/worker.rs`
- Modify: `src-tauri/src/sync/store.rs`

**Interfaces:**
- Produces: strict `ShutdownIntentV1`, `StartupRecoveryFaultV1`, phase, and disposition types.
- Produces: `recover_sync_state_once(context) -> Result<RecoveredQueueStateV1, StartupRecoveryGateError>`.
- Produces: `resolve_startup_recovery_fault(disposition)` IPC backend.

- [ ] **Step 1: Write failing shutdown-marker and fault tests**

Cover valid markers, wrong application/root, PID still alive with matching creation time, PID reuse, malformed marker, orphan invalid marker, fault-only recovery, raw hash mismatch, and every Detected -> MarkerArchived -> RecoveryApplied crash point.

- [ ] **Step 2: Write failing recovery disposition tests**

For PreserveQueued and CancelQueued, cover running with valid Complete/Incomplete result, running without result, valid queued manual, stale automatic config, unavailable capability, archived account, changed browser/profile, and started-vs-never-started schedule anchors. Assert no running row exists before worker start.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test startup_recovery` and expect missing recovery APIs.

- [ ] **Step 4: Implement phase-persisted idempotent recovery**

Always check fault, invalid marker, normal marker, then ordinary crash. Create and fsync the fault before renaming an invalid marker. Use the original importer for valid complete result files. Use conditional CAS for failed/cancelled fallback and update batch snapshots/status in the same transaction.

- [ ] **Step 5: Enforce one recoverable startup state machine**

Store `StartupRecoveryStateV1::{NotStarted,Blocked,Recovering,Ready}` behind a mutex/condition variable. Cache only `Ready`; a durable recovery fault moves to `Blocked` and the minimal UI may call `resolve_startup_recovery_fault` in the same process. A successful resolution transitions through Recovering, reruns the no-running assertion, rebuilds the queue, and returns a single-use `StartupContinuationPermitV1`. Task 13 consumes that permit through the same startup continuation that performs onboarding, tray/window, and worker stages, so no restart is required. Remove task recovery from `prepare_app_directories`; ordinary IPC directory preparation must never reorder or cancel queued work.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test startup_recovery
cargo test --manifest-path src-tauri/Cargo.toml --test sync_lifecycle
```

Then commit:

```powershell
git add src-tauri/src/startup_recovery.rs src-tauri/src/main.rs src-tauri/src/sync/worker.rs src-tauri/src/sync/store.rs src-tauri/tests/startup_recovery.rs
git commit -m "feat: recover persistent sync state on startup"
```

### Task 12: Add Uninstall Journal, Guard, And Package Manifest Core

**Files:**
- Create: `src-tauri/src/uninstall.rs`
- Create: `src-tauri/build_support/uninstall_manifest.rs`
- Create: `src-tauri/build_support/package_identity.rs`
- Create: `src-tauri/tests/uninstall_lifecycle.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/process_lock.rs`
- Modify: `src-tauri/build.rs`

**Interfaces:**
- Produces strict package manifest v1, uninstall session phases, cleanup/interrupted faults, product identity, protected paths, and user data disposition.
- Produces: `recover_uninstall_state_before_bootstrap(coordination_dir, executable_identity)`.
- Produces: `run_uninstall_guard(args)` early process mode and authenticated nonce/handle handshake.
- Produces: `run_package_manifest_builder(args)` non-GUI build mode invoked only by the NSIS compiler against an explicit staging file list.
- Extends a single `EarlyProcessModeV1` parser so collector, login, uninstall guard/helper, and package-manifest builder all dispatch before GUI mutex, Tauri setup, plugins, bootstrap, or SQLite.

- [ ] **Step 1: Write failing manifest validation tests**

Reject absolute paths, `..`, duplicate/case-colliding manifest paths, symlinks/reparse points, length/hash mismatch, protected-path overlap, any requested installation-tree file deletion outside the manifest, and a mutable installed manifest without the compile-time expected hash. Separate identities explicitly: the old installation build/version must match the journal, old manifest, and any still-existing old EXE metadata; a candidate repair package must match application ID, product name, and architecture but may have a newer build ID/version. Preparing/Prepared validation requires every manifest entry to exist and match but never enumerates the install directory. DestructiveCommitted takeover permits already-deleted entries and verifies every still-existing entry.

The manifest governs only package-owned files under the normalized install tree. NSIS-owned start-menu shortcuts and fixed uninstall registry values use a separate exact object whitelist, and they may be removed only after DestructiveCommitted. Installation-generated files are not errors merely because they are outside the manifest; the pinned template grants the NSIS self-removing uninstaller one documented narrow install-tree exception.

- [ ] **Step 2: Write failing journal and takeover tests**

Cover Preparing/Prepared rollback, DestructiveCommitted failure-close, Completed/RepairCompleted cleanup, Run/device compensation faults, lost guard nonce, installer PID reuse, update takeover, continue-uninstall, same-version and cross-version repair completion, old-install identity mismatch, candidate product/architecture mismatch, exact shortcut/registry whitelist enforcement, and retained/deleted data choices.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test uninstall_lifecycle` and expect missing uninstall APIs.

- [ ] **Step 4: Implement strict contracts and state transitions**

Persist the immutable identity snapshot with Preparing before changing autostart. Disable device autostart first, then delete and verify HKCU Run. Roll back in the reverse required order: restore and verify Run, then restore device settings. After DestructiveCommitted never restore autostart.

- [ ] **Step 5: Add the build-time manifest generator**

`build.rs` computes `EMILY_BUILD_ID` as lowercase SHA-256 of canonical, sorted relative-path/hash entries for `Cargo.toml`, `Cargo.lock`, `build.rs`, `build_support/**`, `src/**/*.rs`, `collector-scripts/**`, `migrations/**`, `tauri.conf.json`, and `dist/build-manifest.json`, plus application ID, product name, semantic version, and `windows-x86_64`. It emits the value through `cargo:rustc-env`; the EXE exposes the same compiled constant to the manifest builder and uninstall journal.

The manifest generator accepts an explicit response file containing sorted package-owned staging source paths and install-relative destinations plus an explicit output path. It computes byte length and SHA-256, rejects paths outside staging and reparse points, and emits canonical JSON plus its expected hash for installer metadata. `main.rs` dispatches `--package-manifest-build` before GUI initialization so the NSIS compiler invokes the already-linked Rust binary without Node or recursive Cargo. It must never enumerate an installed directory at runtime to decide ownership.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test uninstall_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml --test frontend_manifest
```

Then commit:

```powershell
git add src-tauri/src/uninstall.rs src-tauri/src/main.rs src-tauri/src/process_lock.rs src-tauri/src/lib.rs src-tauri/build_support/uninstall_manifest.rs src-tauri/build_support/package_identity.rs src-tauri/build.rs src-tauri/tests/uninstall_lifecycle.rs
git commit -m "feat: add durable uninstall recovery core"
```

### Task 13: Add Controlled Exit And The Global Desktop Startup Gate

**Files:**
- Create: `src-tauri/src/desktop_lifecycle.rs`
- Create: `src-tauri/tests/desktop_lifecycle.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/sync/admission.rs`
- Modify: `src-tauri/src/sync/worker.rs`

**Interfaces:**
- Produces: `recover_desktop_startup_once` with the fixed global startup order.
- Produces: idempotent `continue_desktop_startup_from_ready(StartupContinuationPermitV1)` used both by initial success and in-process recovery-fault resolution.
- Produces: `request_app_exit` and `abort_exit_before_marker`.
- Produces: `StartupWindowDecisionV1` and a testable startup action log.

- [ ] **Step 1: Write failing startup-order tests**

Use injected stage callbacks and assert uninstall recovery runs before bootstrap parsing, bootstrap before DB/schema, schema commit before reconstructible schema-upgrade process cleanup, that cleanup before sync queue recovery, sync recovery/no-running assertion before tray/window, and scheduler last. A schema-cleanup failure must not call queue recovery. Every failure before normal runtime must leave no worker, browser lease, DB connection, or prematurely shown window.

Add the blocked-recovery continuation matrix: `Blocked -> Recovering -> Ready -> onboarding/device decision -> tray/window -> worker` completes in the same process, and every stage after Ready executes exactly once even if the resolve command/event is repeated.

- [ ] **Step 2: Write failing exit state-machine tests**

Cover owned-operation timeout before marker, stable-root failure, marker write failure, abort race with last operation, worker stop, running cancellation, two-second process cleanup, ten-second total deadline, importer-vs-cancel CAS, final queued sweep including child jobs, marker retention on hard timeout, and second ExitRequested pass-through.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test desktop_lifecycle` and expect missing desktop lifecycle APIs.

- [ ] **Step 4: Implement one startup coordinator**

Early-dispatch collector, login, uninstall guard/helper, and package-manifest-builder modes before GUI mutex. Acquire GUI mutex, recover uninstall state using only the coordination directory, then resolve bootstrap/data root, migrate schema, reconstruct and finish any schema-upgrade process cleanup, recover sync state, assert no running rows, and issue a startup continuation permit. The shared continuation initializes device/onboarding, creates tray/window state, and finally starts the worker; initial success and in-process fault resolution call the same idempotent continuation.

If uninstall state, device settings, bootstrap, schema, or shutdown recovery is structurally invalid, start only the minimal visible recovery UI needed for the stable error. Do not open the user database before the design permits it, do not initialize tray-silent mode, and do not start scheduler or browser operations.

- [ ] **Step 5: Implement controlled exit with durable linearization**

Do not cancel queued/running or stop the worker until `shutdown-intent.json` is atomically durable. After the marker, transition to Exiting, stop dequeue, resolve running/monitor/continuations, sweep queued, verify zero active jobs, remove task files/marker, mark complete, and call `app.exit(0)`.

- [ ] **Step 6: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test desktop_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml --test startup_recovery
cargo test --manifest-path src-tauri/Cargo.toml --test sync_lifecycle
```

Then commit:

```powershell
git add src-tauri/src/desktop_lifecycle.rs src-tauri/src/main.rs src-tauri/src/lib.rs src-tauri/src/sync/admission.rs src-tauri/src/sync/worker.rs src-tauri/tests/desktop_lifecycle.rs
git commit -m "feat: coordinate desktop startup and exit"
```

### Task 14: Add Tray, Activation Event, Desktop Intents, And Notifications

**Files:**
- Create: `src-tauri/src/notifications.rs`
- Create: `src-tauri/tests/desktop_shell.rs`
- Modify: `src-tauri/src/process_lock.rs`
- Modify: `src-tauri/src/desktop_lifecycle.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Produces: `GuiActivationEvent`, `DesktopIntentState`, `take_desktop_intents(last_seen_revision)`.
- Produces tray menu decision functions and notification policy service.
- Adds commands: `request_app_exit`, `take_desktop_intents`, and `set_scheduler_paused`.

- [ ] **Step 1: Write failing single-instance and intent tests**

Assert second manual launch signals the auto-reset event; second `--autostart` exits silently; collector, login, uninstall guard/helper, and package-manifest-builder early modes bypass GUI single-instance, Tauri/plugin initialization, and database access; intent revisions prevent pre-mount event loss and duplicate handling.

- [ ] **Step 2: Write failing tray/window decision tests**

Cover manual launch, silent autostart, onboarding incomplete, tray failure, close-to-tray, exit-on-close, system exit, first tray hint once, dynamic menu enablement, and current-account/all-account tray enqueue scopes.

- [ ] **Step 3: Write failing notification-policy tests**

Assert notifications only for final login expiry, the third consecutive automatic failure per complete task key, and first tray hide; success, partial, cancellation, another task key, or recovery resets/deduplicates the streak as specified.

- [ ] **Step 4: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test desktop_shell` and expect missing APIs.

- [ ] **Step 5: Implement Tauri tray and event wiring**

Enable `tauri = { version = "2", features = ["tray-icon"] }`, add `tauri-plugin-notification = "2"`, initialize the plugin once, and grant only its required capability. Set the main window to `visible:false`. Left-click opens/focuses; right-click displays the fixed six-item menu. On tray creation failure set `tray_available=false`, force the window visible, disable Tray close behavior, and record `tray_unavailable`.

- [ ] **Step 6: Replace direct ExitRequested cancellation**

Window close uses the close-behavior decision; real exit calls `request_app_exit`. Keep the first exit request prevented until cleanup completes, then permit the final exit event.

- [ ] **Step 7: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test desktop_shell
cargo test --manifest-path src-tauri/Cargo.toml --test desktop_lifecycle
```

Then commit:

```powershell
git add src-tauri/src/notifications.rs src-tauri/src/process_lock.rs src-tauri/src/desktop_lifecycle.rs src-tauri/src/main.rs src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/capabilities/default.json src-tauri/tauri.conf.json src-tauri/tests/desktop_shell.rs
git commit -m "feat: add tray and desktop activation"
```

### Task 15: Expose Scheduler, Batch, Device, And Onboarding IPC

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/sync/contracts.rs`
- Modify: `src-tauri/src/sync/scheduler.rs`
- Modify: `src-tauri/src/settings_store.rs`
- Modify: `src-tauri/src/device_settings.rs`
- Create: `src-tauri/tests/desktop_ipc.rs`

**Interfaces:**
- Adds commands: `enqueue_sync_tasks`, `get_sync_batch`, `list_sync_batch_entries`, `get_scheduler_state`, `list_scheduler_instances`, `update_account_defaults`, `commit_onboarding_step`, `set_autostart`, `set_scheduler_paused`, `set_browser_preference`, `take_desktop_intents`, `resolve_startup_recovery_fault`, `request_app_exit`.
- Keeps: `start_sync`, `cancel_sync`, `get_sync_status`, login/account/data-root command names.

- [ ] **Step 1: Write failing IPC serialization tests**

Assert camelCase output, strict input rejection, fixed source/range/status strings, queued position, partial issues, generic record counts, history compatibility count, batch first page/cursor, all four capabilities, and account/default/device scope separation.

Add capability-aware `RecentDays` cases: portable settings saves, implicit payload generation, and explicit enqueue payloads must all return `time_range_not_supported` when the selected source lacks reliable remote timestamps.

- [ ] **Step 2: Write failing command admission tests**

Cover maintenance/exit rejection, source-setting update gate, browser-preference relogin gate, paused scheduler, account state, unavailable source, and queued/running cancellation.

- [ ] **Step 3: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test desktop_ipc` and expect missing command adapters.

- [ ] **Step 4: Implement thin command adapters**

Commands validate transport structs, acquire the required admission operation, call domain services, release locks before blocking work, and emit `scheduler:changed`/`sync:changed` only after durable commit. Do not duplicate queue rules in Tauri command functions.

Do not expose an arbitrary device-settings document write. `commit_onboarding_step` accepts a strict tagged step request and expected portable/device revisions; it first commits the required portable account/default settings and any protected desktop/browser operation through their dedicated transaction, then advances device `onboarding_step` exactly once. Repeated identical requests are idempotent. Tray-hint and notification-streak fields remain internal-only.

- [ ] **Step 5: Verify and commit**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test desktop_ipc
cargo test --manifest-path src-tauri/Cargo.toml --test sync_enqueue
cargo test --manifest-path src-tauri/Cargo.toml --test sync_scheduler
```

Then commit:

```powershell
git add src-tauri/src/main.rs src-tauri/src/sync/contracts.rs src-tauri/src/sync/scheduler.rs src-tauri/src/settings_store.rs src-tauri/src/device_settings.rs src-tauri/tests/desktop_ipc.rs
git commit -m "feat: expose scheduler and queue commands"
```

### Task 16: Build The Four-Step Onboarding And Scheduler Settings UI

**Files:**
- Modify: `app/src/types.ts`
- Modify: `app/src/app-api.ts`
- Create: `app/src/onboarding.ts`
- Create: `app/src/scheduler.ts`
- Create: `app/src/onboarding.test.ts`
- Create: `app/src/scheduler.test.ts`
- Modify: `app/src/App.svelte`
- Modify: `app/src/App.test.ts`
- Modify: `app/src/styles.css`
- Regenerate: `dist/index.html`
- Regenerate: `dist/assets/**`
- Regenerate: `dist/build-manifest.json`

**Interfaces:**
- Consumes all Task 15 IPC types and events.
- Produces a four-step full-window onboarding overlay and expanded settings modal.
- Produces visibility-aware polling: 500 ms only for visible active jobs, at most 60 seconds for visible idle scheduler state, no nonessential interval while hidden.

- [ ] **Step 1: Write failing TypeScript contract and reducer tests**

Test exact Rust string unions, step 0..3 resume, account skip/default saves, capability-disabled controls, immediate-sync request creation, close/autostart/silent validation, and completion idempotency.

- [ ] **Step 2: Write failing rendered UI tests**

Cover new install vs upgraded user, each onboarding step, skipped login, unavailable likes/favorites, account defaults, scheduler pause, last/next status, queued/running/partial issue rendering, batch first and second pages, active-batch entries appended after an initially exhausted cursor, invalid cursor reload, RemoteFolder scheduler-instance second page, dual-revision invalidation with restart from page one, browser preference relogin hint, tray-unavailable close behavior, autostart rollback, and desktop intent arriving before mount.

Assert `RecentDays` is disabled when capability time support is absent and that a stale/request-forged save surfaces `time_range_not_supported` without changing local form state.

- [ ] **Step 3: Write failing refresh-policy tests**

Use fake timers and `document.visibilityState` to prove the 500 ms loop runs only for visible queued/running work, visible idle state polls no faster than 60 seconds, hidden state has no nonessential polling, and backend events trigger immediate refresh.

- [ ] **Step 4: Confirm RED**

Run:

```powershell
npm test -- --run app/src/onboarding.test.ts app/src/scheduler.test.ts app/src/App.test.ts
npm run typecheck
```

Expected: missing types/modules/UI behavior.

- [ ] **Step 5: Implement typed frontend adapters and reducers**

Keep transport construction in `app-api.ts`; keep onboarding transition/validation pure in `onboarding.ts`; keep visibility/event timer policy in `scheduler.ts`. Use backend timestamps and queue positions without frontend due-time calculation.

- [ ] **Step 6: Implement onboarding and settings UI**

Create the four confirmed steps in the existing main window. Add source-specific range/schedule controls, startup delay, close behavior, autostart/silent controls, browser preference, pause/resume, last/next sync state, partial issues, and source sync commands. Preserve the current compact desktop style; do not perform the stage 3.6 visual redesign.

- [ ] **Step 7: Add Tauri event listeners and desktop intent handshake**

Register `desktop:intent-changed`, `scheduler:changed`, and `sync:changed`; immediately call `takeDesktopIntents(lastSeenRevision)` after listeners are active. Clean up all listeners and timers on component unmount.

- [ ] **Step 8: Verify frontend and regenerate committed dist**

Run:

```powershell
npm test
npm run typecheck
npm run app:build
cargo test --manifest-path src-tauri/Cargo.toml --test frontend_manifest
```

Expected: all frontend tests pass and Cargo accepts the exact regenerated manifest/output set.

- [ ] **Step 9: Commit**

```powershell
git add app/src/types.ts app/src/app-api.ts app/src/onboarding.ts app/src/scheduler.ts app/src/onboarding.test.ts app/src/scheduler.test.ts app/src/App.svelte app/src/App.test.ts app/src/styles.css dist/index.html dist/assets dist/build-manifest.json
git commit -m "feat: add onboarding and scheduler settings"
```

### Task 17: Wire NSIS Guard, Update Takeover, And Uninstall Data Choice

**Files:**
- Create: `src-tauri/nsis/installer-template.nsi`
- Modify: `src-tauri/nsis/installer-hooks.nsh`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/uninstall.rs`
- Modify: `src-tauri/build.rs`
- Modify: `src-tauri/build_support/uninstall_manifest.rs`
- Modify: `src-tauri/tests/uninstall_lifecycle.rs`

**Interfaces:**
- Connects the Task 12 guard to all install, uninstall, and `/UPDATE` entry paths.
- Embeds the expected build-manifest hash in NSIS metadata and transfers raw manifest bytes plus authenticated nonce/handle to the guard.

- [ ] **Step 1: Write failing generated-script/package tests**

Pin the upstream template from `https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi` (`tauri-cli-v2.11.4`, SHA-256 `1114a136876d667ca7ba77218d4e966a625d1078dcba18484caae8f7bffb8626`) before applying Emily-specific edits. Assert every installer mode checks the coordination directory before update branching, non-update uninstall starts the long-lived guard, update bypass occurs only with no nonterminal journal/fault, data deletion runs only after DestructiveCommitted, guard loss aborts, and repair takeover keeps autostart disabled. A template provenance sentinel test must fail if the recorded upstream commit/hash changes without an explicit re-review.

- [ ] **Step 2: Confirm RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml --test uninstall_lifecycle nsis_contract -- --exact` and expect the old minimal hook to fail assertions.

- [ ] **Step 3: Implement the NSIS handshake**

Set `bundle.windows.nsis.template` to `nsis/installer-template.nsi`. Keep production `build.beforeBuildCommand` and `build.beforeBundleCommand` absent/null, keep `frontendDist="../dist"`, and add a config test that rejects references to `node`, `npm`, or `npx` in production build hooks. During NSIS compilation, write a response file containing only template-declared package-owned staging source/destination pairs, invoke `${MAINBINARYSRCPATH}` in `--package-manifest-build` mode with that response file and explicit outputs, then include the canonical manifest, expected hash, and compiled `EMILY_BUILD_ID` in installer metadata. Never copy a generated `.nsi` containing local absolute paths into source control. At runtime, copy the guard-capable executable to the current-user temporary coordination area before destructive work. Pass installer PID/creation identity, session nonce, named event or parent handle, raw package manifest, expected hash, and data disposition. Hold GUI and uninstall mutexes until NSIS reports completed/aborted or exits.

- [ ] **Step 4: Replace the old direct data helper path**

Only after guard commit may NSIS remove package-owned files and optionally invoke the strict managed-data helper. Default to keeping data. Preserve device settings with autostart disabled when data is retained.

- [ ] **Step 5: Verify the real Node-free package path**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test uninstall_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml
cargo tauri build
```

Run the build in a PowerShell environment where `Get-Command node`, `npm`, and `npx` all fail. Inspect the rendered NSIS script to confirm the response-file manifest command and upstream-template sentinel. Temporary response files and generated scripts under `target/` may contain build-machine staging/source paths required by NSIS; assert those paths do not appear in the tracked template, canonical package manifest, installer metadata payload, uninstall journal, or installed files. Confirm the expected hash and compiled build ID are present.

- [ ] **Step 6: Commit**

Then commit:

```powershell
git add src-tauri/nsis/installer-template.nsi src-tauri/nsis/installer-hooks.nsh src-tauri/tauri.conf.json src-tauri/src/main.rs src-tauri/src/uninstall.rs src-tauri/build.rs src-tauri/build_support/uninstall_manifest.rs src-tauri/tests/uninstall_lifecycle.rs
git commit -m "feat: guard installer lifecycle and user data"
```

### Task 18: Run Full Gates, Package, And Record Stage 3.3 Evidence

**Files:**
- Create: `docs/testing/tray-onboarding-background-scheduler-verification.md`
- Modify only if a gate reveals a defect: files owned by the failing task; create a separate focused fix commit.

**Interfaces:**
- Produces deterministic automated evidence, package scan, manual test matrix, and bounded stage statement.

- [ ] **Step 1: Run the complete automated suite in race-safe order**

Run:

```powershell
npm test
npm run typecheck
npm run app:build
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1
```

Set `EMILY_TEST_BROWSER=chrome` and rerun the ignored browser fixture for Chrome where installed.

- [ ] **Step 2: Build without Node on PATH**

Start a clean PowerShell process whose PATH contains Rust/Cargo, `cargo-tauri 2.11.4`, Windows system tools, and WebView2 prerequisites but no Node/npm/npx. Confirm all three `Get-Command` lookups fail, then run:

```powershell
cargo tauri build
```

- [ ] **Step 3: Scan the installer and installed tree**

Record installer path/size/hash. Scan source-tracked production resources, NSIS contents, and a test installation for Node, npm, npx, NCC, `index.cjs`, chrome-remote-interface, Playwright, Chromium, or Headless Shell. Any match in a production runtime path fails the gate.

- [ ] **Step 4: Execute the desktop lifecycle matrix**

On a current-user test install, verify manual show, silent autostart, second-instance activation, close-to-tray, tray restore/settings/exit, pause/resume, 15-minute two-cycle automatic history sync, manual priority, sleep recovery one-shot, login expiry notification, third-failure notification, and zero collector/browser residuals after exit.

- [ ] **Step 5: Execute installer recovery cases**

Verify clean install, same-path upgrade, Preparing/Prepared interruption rollback, DestructiveCommitted interruption failure-close, continue-uninstall, repair takeover, keep-data default, delete-data opt-in, install-directory `Data`, custom data root, and `/UPDATE` refusal when any nonterminal fault exists.

- [ ] **Step 6: Measure idle resources**

For ten idle minutes sample the Emily process plus WebView2 child tree every five seconds. Record median and p95 private working set, average/p95 CPU, scheduler wakeups, and the same-window Electron comparison. Required limits: median <=180 MiB, p95 <=250 MiB, average CPU <0.5%, p95 CPU <2%, <=3 scheduler wakeups/minute, and Emily median <=80% of Electron.

- [ ] **Step 7: Write the verification report with bounded status**

The report must contain commands, counts, package path/hash, environment, pass/fail for every automatic/manual gate, residual risks, and this exact completion boundary when valid:

> 观看历史生命周期完成，多来源设置/队列/结果契约已冻结，archive 执行器与最终多来源验收待 3.4。

If stage 3.2 real-installation gates remain open, state that they still block release.

- [ ] **Step 8: Commit the evidence**

```powershell
git add docs/testing/tray-onboarding-background-scheduler-verification.md
git commit -m "test: record stage 3.3 desktop verification"
```

## Plan Self-Review Checklist

- [ ] Every requirement in design sections 4 through 10 maps to at least one task above.
- [ ] Every 3.3 automated test item in design section 11.1 maps to a focused task or Task 18 full gate.
- [ ] Design section 11.2 remains contract/fixture-only in 3.3; no archive executor is registered.
- [ ] No task uses current UI settings at execution time in place of a frozen queued payload.
- [ ] No task lets frontend timers become scheduler truth.
- [ ] No task enters runtime-path maintenance while holding admission/queue/device/database locks.
- [ ] No task performs irreversible uninstall work before a durable DestructiveCommitted journal.
- [ ] Frontend source and committed dist are regenerated together.
- [ ] Each implementation task has a focused RED command, GREEN command, and single-purpose commit.

## Execution Rule

After this plan passes the mandatory `gpt-5.6-sol` / `xhigh` plan review, execute it inline with `superpowers:executing-plans`. Project rules prohibit implementation subagents, so do not choose the subagent-driven option from the generic skill guidance.
