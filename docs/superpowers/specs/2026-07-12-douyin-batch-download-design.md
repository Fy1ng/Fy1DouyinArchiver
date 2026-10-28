# Douyin Batch Download Module Design

## Objective

Add a modular, manually triggered batch-download capability to the existing Windows x64 Tauri application. Users can download selected history entries, all entries matching the current filters, or the complete history archive. The feature preserves original media assets for normal videos, image posts, and live-photo posts, supports persistent resumable jobs, and remains isolated from the main branch's ongoing third-stage collector and synchronization work.

The installed application remains a Rust/Tauri application with no Python, Go, Node, FFmpeg, proxy-certificate, or embedded-browser runtime added to the production package. Node remains optional frontend development tooling under the existing project rules.

## Confirmed Product Requirements

- Downloads are initiated manually. History synchronization never starts downloads automatically.
- The user can create a job from checked entries, the complete current filtered result, or all history.
- A job may contain normal videos, image posts, live-photo posts, and currently unknown content types.
- Image posts and live-photo posts preserve their original assets. The application does not combine them into a generated video.
- Settings store a default download directory and whether each new job should ask for a directory.
- Settings store a default quality profile and whether each new job should ask for a quality profile.
- A repeated job skips assets that were previously downloaded completely and still pass file validation. Failed or missing assets are retried individually.
- Job, item, and asset progress survives application exit or crashes. On the next launch the user may resume interrupted work manually.
- The download feature is implemented as an independent module with narrow adapters to history data and browser media resolution.

## Scope

### Included

- Manual batch selection from the history interface.
- Persistent download jobs, item snapshots, asset records, progress, retry state, and recovery.
- Media resolution through the existing authenticated system-browser environment.
- Original video, image, live-photo video, and soundtrack downloads when those assets are exposed by Douyin.
- Configurable default destination and quality, with optional per-job prompts.
- Pause, resume, cancel, retry-failed, open-folder, skip-complete, and interrupted-job recovery workflows.
- Streaming downloads, HTTP Range continuation where supported, bounded concurrency, temporary files, validation, and atomic completion.
- Sanitized diagnostics and stable Chinese-facing error messages.
- Additive database migrations and frontend/backend modules that minimize conflicts with third-stage work.

### Excluded

- Automatic downloading after history synchronization.
- Re-encoding, transcoding, slideshow generation, image-to-video conversion, or soundtrack merging.
- Live-stream recording.
- Downloading content that the authenticated browser cannot access.
- Maintaining a standalone Douyin `A_Bogus`, `X-Bogus`, or equivalent signature implementation as the primary resolver.
- Installing a system proxy or root certificate for traffic interception.
- Bundling yt-dlp, Python, Go, Node, FFmpeg, Chromium, or another sidecar runtime.
- Refactoring the third-stage collector, synchronization engine, or current history schema for unrelated reasons.

## Chosen Technical Direction

The module uses a hybrid design: the authenticated system browser resolves media metadata and Rust performs all durable queueing and file transfer work.

Three approaches were evaluated:

1. Reuse the authenticated browser and CDP. This matches the current architecture, avoids maintaining volatile signing algorithms, and adds no production runtime. This is the selected approach.
2. Call Douyin Web APIs directly from Rust. This appears simpler but requires continuous maintenance of cookies, fingerprints, and signing parameters and can fail without protocol changes in the application.
3. Embed an existing downloader such as yt-dlp or a Python/Go Douyin project. This offers broad format support but conflicts with the project's Rust-only production runtime and introduces licensing, packaging, and update concerns.

Existing projects remain design references rather than drop-in dependencies:

- `yt-dlp/yt-dlp` informs format enumeration and quality ranking.
- `Evil0ctal/Douyin_TikTok_Download_API` informs current media response shapes and resolver error categories.
- `Johnserf-Seed/TikTokDownload` and `JoeanAmier/TikTokDownloader` inform batch and archive workflows.
- `putyy/res-downloader` demonstrates traffic-based resource discovery, but its system-proxy model is deliberately not adopted.
- Old Tauri downloaders using `iesdouyin.com/web/api/v2` are not reused because those endpoints now require unavailable encrypted data, and repositories without a usable license are reference-only.

No GPL source is copied into the application. Code from repositories without a clear license is not copied. Any adopted algorithmic idea is independently implemented against this specification and the current platform behavior.

## Module Boundaries

Production Rust code lives under `src-tauri/src/download/` with the following ownership boundaries:

- `contracts.rs`: strict Tauri command payloads, job/item/asset views, quality profiles, progress events, and stable state enums.
- `catalog.rs`: the `DownloadCatalog` interface and history-data adapter.
- `resolver.rs`: the `MediaResolver` interface, resolved media manifests, quality selection, and expiring-source rules.
- `browser_adapter.rs`: the thin adapter between `MediaResolver` and the current system-browser/CDP facilities.
- `service.rs`: job creation, deduplication, control commands, recovery, and orchestration.
- `queue.rs`: resolver and download scheduling, concurrency limits, pause and cancellation propagation.
- `worker.rs`: HTTP streaming, Range continuation, retries, validation, and atomic file completion.
- `storage.rs`: download-table access, recovery queries, and the download persistence actor. The SQL migration belongs to the module, while migration ordering remains centrally coordinated.
- `paths.rs`: deterministic directory layout, sanitization, containment checks, and collision handling.
- `errors.rs`: internal error chains, stable error codes, and user-facing Chinese messages.
- `mod.rs`: the module's intentionally small public surface.

Frontend code lives under `app/src/download/` and owns its API wrapper, types, task state, start dialog, selection controls, and task center. Existing history components receive only narrow integration props or callbacks. Download state and rendering do not become part of the history query implementation.

All commands, events, settings, CSS classes, and persistent keys owned by this feature use a `download_` or `download.*` namespace. The module does not import frontend state from the synchronization feature.

The required integration boundaries are:

- `DownloadCatalog`: expands selection conditions into stable candidate snapshots.
- `MediaResolver`: resolves a candidate into original downloadable assets through the authenticated browser environment.
- the central database migration registry, which assigns and applies the next schema migration
- the main-process command/event registration and shared browser-operation coordinator

Changes to the third-stage history schema, filter implementation, browser lifecycle, or CDP types should require changes only in these adapters.

## Core Contracts

`DownloadCandidate` is a task-time snapshot and contains only stable archive metadata:

- `aweme_id`
- canonical Douyin URL
- normalized content type
- optional title, author, and publish time
- the history/filter context required for a user-facing summary

`ResolvedMediaManifest` is an in-memory resolution result:

- `aweme_id` and resolved content type
- zero or more `ResolvedAsset` entries
- the actual quality selected for video-like assets
- optional expected byte length, MIME type, dimensions, bitrate, codec, and source expiry hints
- request metadata required for the immediate download, held in memory only

Each `ResolvedAsset` has a deterministic stable key independent of the temporary URL. Examples include `video:primary`, `image:0001`, `live-image:0001`, `live-video:0001`, and `audio:music`. The key, asset type, ordinal, selected quality, and destination path identify durable progress. The source URL does not.

Resolver and downloader contracts reject unknown fields at persistence and Tauri boundaries. Browser response parsing is tolerant of unrelated upstream fields but strict about the fields used to create a downloadable asset.

## Selection And Job Creation

The history interface supports three explicit actions:

1. Download checked entries.
2. Download the complete result represented by the current filters, not only the visible page.
3. Download all history.

The frontend submits the selected IDs or the complete serialized filter contract. It does not enumerate every page client-side. `DownloadCatalog` expands the request in the backend, takes immutable snapshots, and deduplicates candidates by decimal-string `aweme_id` before creating a job.

Large filtered or all-history jobs show a confirmation dialog containing the deduplicated work count, selected destination, quality profile, and known content-type distribution. Job creation remains atomic: either the job and all candidate snapshots are committed or no job is created.

History records may later be updated or deleted without invalidating an existing job because the job owns its candidate snapshots. There is no foreign key from download tables to `videos` or `watch_records`.

When multiple selected watch records refer to the same `aweme_id`, the snapshot uses the record with the greatest `history_time_ms`; ties use ascending `source_position` and then descending watch-record ID. Checked duplicate watch records still produce exactly one download item. The canonical URL and presentation metadata come from that deterministically selected record.

## Media Resolution

Resolution follows this order for every candidate:

1. Inspect media metadata already available in the current authenticated page state, including video play addresses, download addresses, bitrate variants, image lists, live-photo components, and music when present.
2. If the history state is incomplete, open or reuse an isolated detail page through the browser adapter and observe the page's successful detail data through CDP.
3. Normalize the response into a `ResolvedMediaManifest` and discard the raw response.

The resolver does not make an unsigned direct API call its primary path. It relies on the authenticated browser to perform the platform-specific navigation, challenges, cookies, and signed requests that the page already knows how to perform.

The browser resolver runs as a managed mode of the existing executable: `current_exe --download-resolver`. The desktop background service starts one resolver child for the active job and exchanges length-delimited manifests and control messages through anonymous process pipes. Sensitive request metadata may cross only this in-memory pipe; it is never written to task files. The child owns its browser process, CDP connections, and browser PID record and exits after the current resolution batch or explicit cancellation.

The desktop background service, not the short-lived Tauri command handler, owns the shared browser-operation lease for the complete resolver-child lifetime. Synchronization and media resolution may not operate concurrently against the same browser profile. Starting either operation while the other owns the lease returns a stable busy state. File transfers that already have in-memory request metadata may continue after the resolver exits and do not hold the browser lease. A later expiry retry starts a new resolver child and reacquires the lease.

The browser adapter extends CDP with the `Network` response-observation and response-body capabilities required to identify detail JSON, dedicated non-lossy routing for required Network events, and explicit resource policy modes. History synchronization keeps its current media-blocking policy; download resolution uses a separate detail-page policy that permits metadata responses without downloading media bodies in the browser. Resolver-child cancellation, crash cleanup, PID identity validation, and stale-process recovery follow the collector's existing process-isolation model with download-specific records.

Media URLs, cookies, authorization headers, signing parameters, and full raw responses remain in memory. They are not stored in SQLite, progress events, logs, diagnostics, crash artifacts, or task export data.

Temporary media sources are treated as expiring capabilities:

- A persisted or resumed asset is resolved again before transfer unless an active in-memory source is still valid.
- HTTP 401 or 403, an explicit expiry response, or a media redirect to an invalid response causes one forced re-resolution.
- After re-resolution, the asset returns to the normal bounded retry policy.
- A re-resolved source must still match the expected asset type and candidate identity.

## Content Types And Original Assets

Normal video posts create one primary video asset. If multiple formats are available, the quality selector chooses one according to the job's quality profile.

Image posts create one asset per original image in page order. A soundtrack is stored separately when the work exposes an independently downloadable music asset. The application does not render a slideshow or merge audio.

Live-photo posts create paired original image and short-video assets in page order. Pairing information remains visible in stable keys and filenames. A post-level soundtrack is stored separately when available.

Unknown content types are not silently discarded. The resolver attempts to recognize supported asset structures. If it finds safe image, video, or audio assets, it records the resolved type and downloads them. If no supported original asset contract can be established, the item fails with `download_unsupported_content` while the rest of the job continues.

## Quality Profiles

Settings provide one default quality profile, and a separate setting controls whether every new job asks for a profile. The start dialog may override the profile for that job without changing the stored default.

The initial profiles are:

- `highest`: choose the largest available resolution, then the highest bitrate. Prefer H.264 over H.265 only when resolution and effective bitrate are equivalent; otherwise preserve the higher-quality source.
- `balanced`: choose the highest format at or below 1080p, preferring H.264 for broad Windows playback compatibility. If every format exceeds 1080p, choose the lowest format above the cap.
- `space_saver`: choose the highest format at or below 720p. If every format exceeds 720p, choose the lowest available format.

Images use the highest-resolution original candidate exposed by the page for every profile. Audio uses the best original standalone asset exposed by the work. Quality fallback is deterministic: when the exact preferred tier is unavailable, select the nearest lower tier, then the lowest higher tier. The actual selected dimensions, bitrate, codec, and profile are recorded without persisting the temporary source URL.

## Persistence Model

The project gains a central ordered migration registry and a general `initialize_schema` entry point. Every database caller uses it instead of requiring an exact schema version of two. The registry applies committed migrations sequentially in one controlled initialization path and then validates the latest application database version. Collector result schema version two remains a separate protocol constant and is not coupled to the SQLite `user_version`.

The download module contributes one additive SQL migration to that registry. On this branch it is migration 003. If the third-stage main branch consumes version 003 first, the download migration is renumbered to the next unused version during rebase and the registry test is updated in the same commit. The migration creates new tables and does not rebuild or alter current history, video, watch-record, synchronization, or collector tables.

### `download_jobs`

One row represents one manual user action. It stores:

- primary key and creation/completion timestamps
- selection scope and a sanitized filter summary
- destination root selected for this job
- requested quality profile
- state: `queued`, `running`, `paused`, `interrupted`, `completed`, `completed_with_errors`, or `cancelled`
- aggregate candidate, resolved, completed, skipped, failed, and byte counts
- stable job-level error code when the complete job cannot proceed

### `download_items`

One row represents one candidate snapshot within a job. It stores:

- job ID and decimal-string `aweme_id`, unique within the job
- canonical URL, normalized content type, title, author, and publish time snapshot
- deterministic relative work directory
- state: `queued`, `resolving`, `downloading`, `paused`, `interrupted`, `completed`, `skipped`, `failed`, `superseded`, or `cancelled`
- resolution attempt count, stable error code, and sanitized message
- manifest generation, timestamps, and completed/total required asset counts

### `download_assets`

One row represents one original asset within a job item. It stores:

- item ID and stable asset key, unique within the item
- asset type, ordinal, requested profile, and actual quality metadata
- relative final path and relative `.part` path
- expected size when known, downloaded bytes, ETag and Last-Modified when safe and available
- validation metadata needed to recognize an already complete file
- manifest generation and whether the asset is required by that generation
- state: `queued`, `downloading`, `paused`, `interrupted`, `completed`, `skipped`, `failed`, `superseded`, or `cancelled`
- retry count, last stable error code, and timestamps

The tables contain no source URL, Cookie, authorization header, browser storage, or signed query value.

All download writes go through one persistence actor owned by the desktop background service. Workers send typed state transitions over a bounded channel; the actor serializes short SQLite transactions, derives aggregate counters inside the same transaction, and emits frontend events only after commit. Application database connections consistently enable WAL, foreign keys, and a bounded busy timeout so synchronization imports, history reads, and download writes wait predictably rather than failing immediately with `database is locked`.

When a new job includes a previously completed work, the service searches prior successful asset records for the same `aweme_id`, stable asset key, quality identity, and destination. It then checks the actual file. An item is skipped only when every required asset exists and passes validation. If one asset is absent or invalid, only that asset is downloaded again.

## Destination And Naming

Settings store:

- `download.default_directory`
- `download.ask_directory_each_job`
- `download.default_quality`
- `download.ask_quality_each_job`

If the corresponding ask setting is enabled, the start dialog requires confirmation and permits a one-job override. If it is disabled, the resolved default remains visible before a large filtered or all-history job is confirmed. A job override never modifies the stored default.

The default directory layout is:

```text
download root/
  author/
    publish-time_title_awemeId/
      video.mp4
      audio/
        music.m4a
      images/
        001.jpg
        002.jpg
      live/
        001.jpg
        001.mp4
```

Only directories required by the resolved manifest are created. Normal video posts do not receive empty `images`, `live`, or `audio` directories.

Path components remove Windows-invalid characters and trailing dots or spaces, collapse unsafe whitespace, reserve space for the extension and ID, and use deterministic length limits. Missing author, title, or publish time uses a stable localized placeholder. The work directory always includes `awemeId`, preventing title collisions.

Every joined path is normalized and checked to remain under the selected root before any filesystem operation. Symlink and reparse-point behavior is checked again at file creation and atomic rename boundaries. A user selecting a different root is requesting another archive copy; success in a previous root does not skip files in the new root.

## Queue, Concurrency, And State Transitions

The initial version permits one active job and any number of queued or terminal jobs. This removes cross-job browser, destination, and partial-file races while retaining up to three concurrent asset transfers inside the active job. The resolver queue processes one work at a time per browser profile. Concurrency is deliberately not user-configurable in the first release.

The scheduler additionally computes an asset ownership key from the normalized destination root, `aweme_id`, stable asset key, and quality identity. Only one writer may own a key. Partial filenames include the durable asset-row ID and an attempt nonce instead of using a shared `video.mp4.part` name. Before final rename, the worker verifies both ownership and the current destination state; a later queued job reuses the committed result after validation.

The job state is derived from durable item and asset states and the explicit user control state. State transitions are committed before emitting frontend events.

Pause behavior:

- Stop assigning new resolution and transfer work.
- Cancel active HTTP requests cooperatively.
- Flush downloaded byte counts and response validators.
- Retain `.part` files.
- Move active assets to `paused` and the job to `paused`.

Resume behavior:

- Revalidate the destination and available disk space.
- Re-resolve expiring sources.
- Continue a `.part` file with HTTP Range only when server validators and returned ranges are compatible.
- Delete and restart the partial file when continuation is unsafe or unsupported.

Cancel behavior:

- Stop resolver and transfer work.
- Delete incomplete `.part` files owned by the job.
- Preserve every completely validated original asset.
- Mark unfinished assets and items cancelled and mark the job cancelled.

Normal application shutdown requests a pause and persists state. A crash leaves active jobs in `running`; startup recovery performs this deterministic matrix without opening the browser or network:

1. Change every `running` job to `interrupted`.
2. Change `resolving` or `downloading` items and `downloading` assets to `interrupted`; leave queued and explicitly paused rows unchanged.
3. Validate any final file referenced by a nonterminal asset. A valid final file becomes completed; an invalid final file becomes failed with a validation code.
4. Retain contained `.part` files and reconcile their byte counts. Unsafe, missing, or unowned partial files are deleted and the asset returns to queued.
5. On explicit resume, re-resolve every interrupted item, increment its manifest generation, and reconcile by stable asset key. Newly discovered required assets are inserted, unchanged assets retain safe progress, and assets no longer present become `superseded`.
6. Mark an item complete only after the current manifest generation establishes its complete required-asset set and every required asset is validated.

## Download Worker

The worker uses a Rust HTTP client and writes bytes to a `.part` file in the final file's directory. This keeps finalization on the same filesystem so that rename is atomic.

Before transfer it verifies:

- the final and partial paths are contained by the selected root
- the source uses HTTPS
- the connection-time address policy rejects loopback, link-local, private, multicast, unspecified, IPv4-mapped forbidden IPv6, and other reserved ranges across every A/AAAA result
- the response is a successful full or compatible partial media response

Address validation is part of the actual connector/resolver used by the HTTP client, so the verified address is the address used for the connection rather than a separate preflight lookup. Every redirect is resolved and validated again. The client allows at most five redirects, at most 64 KiB of response headers, at most 20 GiB for one asset, and at most 2 TiB of newly written data in one job. Crossing a limit pauses or fails with a stable capacity error before additional bytes are committed.

On Windows, destination directories and files are opened through handles that reject or explicitly validate reparse points. Containment is rechecked against the opened parent handle at temporary-file creation and final rename, preventing a junction or symlink replacement between a path check and a write.

The worker records progress in bounded intervals instead of committing every chunk. Completion requires:

- a non-empty body
- exact expected length when the server provides a reliable total
- agreement between the response type and expected asset class
- a basic file-signature or parseable-container check sufficient to reject HTML, JSON, and challenge pages masquerading as media
- successful flush, close, and atomic rename

Network and transient server errors receive at most three automatic attempts with bounded exponential backoff. A 401, 403, or expired source first consumes the one re-resolution path and then uses the normal retry budget. Permanent path, permission, unsupported-content, validation, or security errors do not retry automatically.

Insufficient disk space, invalid destination access, expired login state, unavailable browser resolution, or a shared-browser conflict pauses the job and exposes an actionable error. The module never retries indefinitely.

## Frontend Experience

The history page gains a modular selection mode with per-row checkboxes and three actions: checked entries, current filters, and all history. Existing pagination and filtering behavior remains unchanged outside selection mode.

The start dialog displays:

- deduplicated work count
- known content-type counts
- destination root
- quality profile
- whether previous successful assets may be skipped

The task center displays job totals, completed, active, queued, skipped, failed, and cancelled counts; overall bytes and speed when known; the current resolver item; and per-work/per-asset status. It provides pause, resume, cancel, retry failed, open job root, and open work directory actions.

Backend events use stable names such as `download_job_updated` and `download_item_updated`. Events are hints for refresh, not the source of truth. The frontend can reload all state through commands after refresh, missed events, or application restart.

The initial Tauri command surface is namespaced and intentionally small:

- `download_create_job`
- `download_list_jobs`
- `download_get_job`
- `download_pause_job`
- `download_resume_job`
- `download_cancel_job`
- `download_retry_failed`
- `download_reveal_path`

Command implementations delegate to the download service. They do not contain queue, resolver, SQLite, or filesystem business logic.

## Security And Privacy

The feature is a personal archive capability, not an access-control bypass. A work that cannot be accessed by the user's authenticated browser remains unavailable.

Sensitive values are memory-only. Logs and diagnostics may contain job ID, `aweme_id`, asset type, state transition, elapsed time, byte counts, attempt count, and stable error code. They must not contain titles, author identity, cookies, local storage, authorization headers, complete media URLs, raw browser responses, or arbitrary HTML.

The HTTP worker treats resolved URLs as untrusted input despite their browser origin. HTTPS enforcement, redirect revalidation, reserved-address rejection, path containment, response validation, and bounded file sizes protect the local machine from malformed or compromised resolver data.

Directory selection uses `tauri-plugin-dialog` with the minimum required capability. Download settings continue in the existing frontend settings blob for now, but every task creation sends the chosen values to the backend, which validates and persists the resolved job snapshot. Invalid or unavailable defaults are rejected before job creation.

`download_reveal_path` accepts only a job or item ID, resolves the persisted path in the backend, verifies containment and existence, and then opens Explorer through a restricted Windows backend helper. It never accepts an arbitrary frontend path and never executes a downloaded file.

## Error Model

Stable error categories include:

- selection and catalog errors
- browser busy, browser unavailable, and login-required errors
- navigation, CDP, page-structure, and media-resolution errors
- unsupported content and missing original assets
- expired source and media authorization errors
- destination, permission, containment, filename, and disk-space errors
- network, timeout, redirect, Range, and response-status errors
- length, MIME, signature, and finalization validation errors
- persistence, recovery, pause, cancellation, and internal contract errors

Internal Rust errors retain source chains for development logs. Tauri responses and persisted rows expose only stable codes and concise Chinese messages. One failed item or asset does not stop unrelated work. A job with at least one failed item and all other work terminal becomes `completed_with_errors`.

## Testing Strategy

### Unit Tests

- Selection expansion and `aweme_id` deduplication.
- Quality selection and deterministic fallback for all profiles.
- Stable asset-key generation for video, image, live-photo, audio, and unknown structures.
- Windows path sanitization, truncation, placeholder values, collision prevention, and containment.
- Job, item, and asset state-machine transitions.
- Retry classification and re-resolution budgets.
- Sanitized error and diagnostic serialization.

### SQLite Tests

- Additive migration on a current schema database.
- Ordered central migration registry behavior, exact latest-version validation, and migration renumbering compatibility.
- Atomic job creation and unique `aweme_id` within a job.
- Progress persistence and aggregate reconciliation.
- Crash conversion from `running` to `interrupted`.
- Reuse of verified historical successes and recovery of only missing assets.
- No dependency or foreign key on current history tables.
- Persistence-actor serialization, busy-timeout behavior, aggregate recomputation, and post-commit event ordering.

### Local HTTP Integration Tests

An in-process local server covers:

- full streaming download
- valid Range continuation
- ignored or invalid Range responses
- ETag or Last-Modified changes
- redirects and redirect loops
- reserved-address rejection logic through isolated resolver tests
- transient retries, timeouts, disconnects, cancellation, and pause
- content-length mismatch
- HTML or JSON returned as media
- atomic finalization and `.part` cleanup

The worker receives an `AddressPolicy` dependency. Production uses the connection-time reserved-address policy. Tests use a policy that permits only the exact random loopback address and port owned by that test server; debug builds do not receive a general loopback bypass.

### Resolver Fixtures

Sanitized fixed JSON fixtures cover normal video variants, image posts with music, live-photo pairs, missing fields, expired sources, unknown content with recognizable assets, and unsupported content. Normal automated tests do not depend on Douyin availability.

A Windows-only ignored acceptance fixture uses an explicitly selected test browser profile to verify real authenticated detail resolution. It never reads the user's normal profile automatically and does not run in the default test suite.

### Frontend Tests

- Checked-entry selection.
- Complete filtered-result submission independent of pagination.
- All-history confirmation.
- Default and per-job directory and quality behavior.
- Event-driven refresh and full reload after missed events.
- Pause, resume, cancel, retry, skipped assets, and failed-item rendering.

## Third-Stage Integration Rules

Development on this branch stays modular while the main branch advances its third stage:

- Do not copy unstable third-stage implementation into the download core.
- Do not restructure collector, synchronization, history query, or existing database modules for download convenience.
- Keep history and browser dependencies behind the two adapters.
- Keep the download migration additive and independently testable.
- Resolve merge conflicts by adapting to the final stable main-branch contracts rather than weakening module boundaries.

Before merging, update the branch from the latest main branch and revalidate:

- migration ordering
- history filter serialization and backend expansion
- Tauri command and state registration
- browser-profile locking and synchronization mutual exclusion
- settings persistence
- frontend integration points
- committed `dist` regeneration and manifest validation when frontend files change

## Acceptance Criteria

- Users can manually create jobs from checked entries, the full current filtered result, and all history.
- Candidate expansion is backend-driven and deduplicates by exact decimal-string `aweme_id`.
- Normal videos, image posts, and live-photo posts preserve their original exposed assets without generated video composition.
- The default destination and quality settings work, and each has an independent per-job prompt setting.
- Highest, balanced, and space-saver profiles select formats deterministically and record actual quality.
- A repeated job skips only fully verified assets and downloads only missing or invalid assets.
- Pause retains resumable partial files; cancel deletes partial files but preserves completed originals.
- Closing or crashing the application leaves recoverable progress, and startup performs no automatic network activity.
- Media authorization expiry triggers one re-resolution without persisting sensitive source data.
- One failed asset does not stop unrelated assets, and users can retry failed work.
- Paths cannot escape the selected root, and non-media challenge responses cannot become completed files.
- Download tables do not alter or depend on current history-table structure.
- Every database command uses the central migration registry, and the collector result protocol remains schema version two.
- One active job prevents cross-job destination races; partial files have unique owned names and final rename revalidates ownership.
- Resolver work runs in a managed child process whose lifetime owns the browser lease and whose sensitive IPC is pipe-only.
- Default tests use only fixed fixtures and local servers; the real-browser acceptance test is explicit and ignored by default.
- Rust tests, frontend tests, type checking, production frontend build, Rust packaging checks, and the Windows real-browser acceptance fixture pass before merge.
- The installed package adds no Python, Go, Node, FFmpeg, proxy certificate, Chromium, or other download sidecar runtime.

## Delivery Sequence

Implementation should preserve module isolation in this order:

1. Replace exact schema-v2 initialization with a tested central ordered migration registry, add the download migration, and keep collector protocol validation independent.
2. Add download contracts, state-machine tests, persistence actor, aggregate reconciliation, and recovery matrix.
3. Add path, quality, deterministic candidate deduplication, asset ownership, and local HTTP worker tests and implementation.
4. Add persistent single-active-job queue control and startup recovery.
5. Add the `MediaResolver` interface, manifest-generation reconciliation, and fixed resolver fixtures.
6. Add the managed resolver child mode, pipe IPC, browser lease ownership, Network-domain support, and PID recovery against the then-current main-branch browser contract.
7. Add namespaced Tauri commands and events plus the restricted directory dialog and Explorer integration.
8. Add frontend selection, start dialog, settings, and task center.
9. Run real-browser resolution acceptance, full regression tests, packaging validation, and third-stage integration checks.

The download branch is not eligible to merge until the module passes all acceptance criteria on top of the latest third-stage main branch.
