# Node-Free Runtime And Packaging Design

## Objective

Remove Node.js from the installed application and the formal packaging path while preserving the current Svelte UI, schema v2 database, user data, sync behavior, settings, deletion and pagination workflows, login flow, and Edge/Chrome CDP collector behavior.

The repository may retain `package.json`, Svelte sources, Vite, Vitest, and other Node-based frontend development tools. They are optional developer tooling only. A clean Windows x64 machine with Rust, Cargo, `cargo-tauri`, the Windows build toolchain, and a system Edge or Chrome installation must be able to run `cargo tauri build` without Node or npm.

## Scope

### Included

- Port the TypeScript CDP collector and its production behavior to Rust.
- Run the collector as a process-isolated mode of the existing application executable.
- Remove the bundled Node runtime and NCC collector bundle from Tauri resources.
- Package the committed frontend `dist/` directly without running Vite.
- Track the packageable frontend files in Git while continuing to exclude legacy collector and Node runtime outputs.
- Preserve existing Tauri command signatures and frontend behavior.
- Preserve schema v2, existing SQLite data, browser profiles, browser selection, and login validation state.
- Preserve structured progress, result, diagnostic, cancellation, and browser PID files.
- Make Rust-only packaging reject stale or missing committed frontend assets.

### Excluded

- Rewriting the Svelte UI as vanilla JavaScript or a native Rust GUI.
- Removing JavaScript execution from the WebView or from CDP `Runtime.evaluate` calls.
- Supporting browsers other than system Edge and Chrome.
- Supporting platforms other than Windows x64 in this migration.
- Migrating or deleting the existing user database or browser profiles.

## Architecture

The application remains a Tauri desktop application with a Svelte/WebView frontend and Rust backend. The installed program contains no Node runtime and starts no JavaScript sidecar process.

The main executable supports three startup modes:

1. Normal launch initializes Tauri and shows the desktop UI.
2. `--collector collect ...` runs the Rust collector and exits without initializing Tauri.
3. `--collector login ...` launches the selected system browser for login verification and exits without initializing Tauri.

Tauri starts collector modes through `std::env::current_exe()`. This preserves process isolation without introducing a second packaged binary. A collector crash, forced cancellation, or browser failure must not terminate the desktop process.

The collector continues to exchange task data through the existing files under `app_data_dir`: result, progress, diagnostic, cancellation, and browser process identity. This keeps startup recovery, forced cancellation, and atomic SQLite import behavior compatible with the current implementation.

## Rust Collector Components

Production collector code lives under `src-tauri/src/collector/` with these ownership boundaries:

- `contracts.rs`: schema v2 result, record, range, progress, diagnostic, browser selection, and login validation structures.
- `cli.rs`: strict parsing for `collect` and `login` modes, including camelCase and snake_case compatibility where current external files require it.
- `browser_discovery.rs`: Windows registry and standard-path discovery for Edge and Chrome with Edge-first automatic selection.
- `browser_state.rs`: isolated profile paths, active browser locking, canonical executable paths, and login validation markers.
- `browser_process.rs`: browser launch arguments, Windows Job Object ownership, process identity files, cooperative close, validated recovery, and forced termination.
- `cdp_transport.rs`: browser and page WebSocket connections, monotonically increasing request IDs, response routing, lossless critical-event delivery, disconnect propagation, and request timeouts.
- `cdp_page.rs`: Target creation, Page/Runtime/Fetch domain setup, navigation, evaluation, resource blocking, and Target closure.
- `snapshot.rs`: embedded page JavaScript and React Fiber atomic snapshot decoding.
- `normalization.rs`: safe string IDs, canonical same-origin URLs, millisecond history time, Shanghai dates, content types, likes, and watch progress.
- `hashing.rs`: identity and field SHA-256 hashes using the existing fixed-array contracts.
- `range.rs`: frozen day range and bottom-range termination rules.
- `sync_engine.rs`: continuous scrolling, snapshots, final stability windows, retries, limits, sparse and empty history handling, and cancellation checks.
- `diagnostics.rs`: atomic result, progress, and diagnostic writes with sensitive values omitted.
- `runner.rs`: login and collection orchestration and cleanup.

Code shared by the desktop process and collector modes is exposed from `src-tauri/src/lib.rs`. `src-tauri/src/main.rs` dispatches collector CLI modes before constructing the Tauri application.

## CDP Behavior

The Rust implementation directly controls system Edge or Chrome through CDP. It does not use Playwright, Chromium, Node, or an embedded JavaScript runtime.

Browser startup preserves the current behavior:

- Dedicated profile directories under `app_data_dir/profiles/edge` and `profiles/chrome`.
- Edge-first discovery for an unlocked automatic selection.
- Browser locking after successful login validation.
- Random loopback debugging port and `DevToolsActivePort` discovery.
- Hidden synchronization through `--headless=new`, `--no-startup-window`, and `--disable-direct-composition`.
- Image and media request blocking by default through the CDP Fetch domain.
- Startup connection retries within one shared startup deadline.
- Browser PID, operating-system creation time, canonical executable path, browser ID, profile path, and DevTools endpoint identity recording before collection begins.
- Normal closure through CDP followed by Windows Job Object cleanup or validated process termination when necessary.

The collector uses the same CDP connection model as the current production collector: one browser WebSocket from `DevToolsActivePort` for Target creation and browser closure, followed by one direct page-target WebSocket for Page, Runtime, and Fetch domains. It does not mix direct target WebSockets with flattened `Target.attachToTarget` session routing.

Each WebSocket has one writer task and one reader task. Each outgoing command receives a monotonically increasing numeric request ID and a one-shot response channel. A request timeout removes its pending entry; a late response is logged without sensitive payloads and discarded. A WebSocket close or browser exit immediately fails every pending request with a stable `cdp_target_closed` or `cdp_connection_failed` error.

Critical events are not sent through a lossy broadcast queue. `Fetch.requestPaused`, Target closure, disconnect, and navigation lifecycle events use dedicated routing or are handled directly by the reader-side dispatcher. A paused Fetch request is converted into an immediate writer command without making the reader wait for that command's response. Non-critical observational events may use bounded broadcast channels, but subscriber lag must be reported explicitly. Queue overflow or an unrouteable critical event fails the CDP session instead of silently continuing. Event subscriptions are installed before enabling the corresponding domain or starting navigation.

Page-side collector scripts remain raw JavaScript files committed under `src-tauri/collector-scripts/` and compiled into the application with `include_str!`. They must be self-contained and may not depend on TypeScript transforms or generated helper functions.

## Sync Data Flow

1. The Svelte UI invokes the unchanged `start_sync` Tauri command.
2. The desktop process initializes schema v2 and inserts a `sync_jobs` row.
3. The desktop process spawns `current_exe --collector collect` with task paths and sync settings.
4. The collector resolves the locked system browser, starts it, connects over CDP, and collects normalized React state snapshots.
5. The collector atomically writes progress during collection and writes one schema v2 result on success.
6. The desktop process waits for collector exit, deserializes the result with unknown-field rejection, and independently validates task identity, range, URLs, time units, and hashes.
7. One SQLite transaction upserts videos and watch events and marks the job successful.
8. Failure rolls back content changes and marks the job failed in a short transaction. Cancellation marks it cancelled.
9. The UI refreshes the existing history list through unchanged commands.

## Cancellation And Recovery

The desktop process retains the running collector child handle and task ID in `AppState`. `cancel_sync` writes the cancellation file first. The Rust collector checks cancellation between CDP operations and during scrolling, closes its CDP session and browser, deletes incomplete temporary results, writes a cancelled diagnostic, and exits.

The collector places the browser process in a Windows Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Normal collector exit closes the browser cooperatively; collector crash or forced collector termination closes the last job handle and terminates the browser process tree. The desktop process retains the collector child handle and terminates the collector after the existing two-second grace period.

Startup recovery cannot rely on a live Job Object handle. Recovery may terminate a recorded browser only when all of the following match: PID, operating-system process creation time, canonical executable path, expected Edge/Chrome executable name, browser ID, canonical profile path, and the complete parsed `--user-data-dir` argument. Validation uses Windows process APIs rather than PowerShell output parsing. If any identity field cannot be queried or does not match, recovery removes the stale record but does not terminate the process. PID reuse must never result in an unrelated process being killed.

At application startup, interrupted jobs use this deterministic recovery order:

1. A job with a complete valid result is imported through the normal atomic validation path; a cancellation file created after atomic result completion does not override an already completed success.
2. A job with a valid cancelled diagnostic, or a cancellation file without a complete valid success result, becomes `cancelled`.
3. Every other job left in `running` becomes `failed` with the interrupted-job error.
4. Validated stale browser processes are cleaned up, identity files are removed, incomplete result files are deleted, and complete unclaimed temporary results older than 24 hours are deleted.

The result, diagnostic, cancellation, and process-identity writes use temporary files in the destination directory followed by an atomic replace. Recovery tests cover every combination used by this decision order.

## Frontend And Packaging

The current Svelte source and optional Node development toolchain remain in the repository. The formal application build consumes committed files from `dist/`. The packageable contents of `dist/` are version-controlled so that a clean checkout contains everything required by `cargo tauri build`.

The repository ignore rules are changed as part of this migration. The blanket `dist/` ignore rule is removed. Generated packageable frontend files and `dist/build-manifest.json` are committed. Legacy `dist/runtime/`, `dist/collector/`, source maps, and temporary files remain ignored by Git but must also be deleted by the frontend build and rejected by the Rust packaging gate if present, because `frontendDist` would otherwise package them. A clean checkout test must confirm that `git ls-files dist` contains the complete packageable frontend and no legacy runtime payload.

`src-tauri/tauri.conf.json` changes as follows:

- Remove `beforeDevCommand`.
- Remove `beforeBuildCommand`.
- Remove `devUrl`.
- Keep `frontendDist` pointing to `../dist`.
- Remove `runtime/` and `collector/` bundle resources.

`cargo tauri dev` loads the committed `dist/` without starting Vite. Frontend developers may still run the existing Vite workflow when changing Svelte code, but `npm run app:build` must regenerate the frontend and manifest together. A frontend source change and its regenerated `dist/` are committed in the same change.

The frontend build atomically writes `dist/build-manifest.json` with this logical content:

- A manifest schema version and the fixed build command.
- Exact Node, npm, Vite, Svelte, TypeScript, and relevant plugin versions.
- A sorted list of build inputs with repository-relative paths and SHA-256 hashes. Inputs include Svelte, TypeScript, CSS, `app/index.html`, Vite configuration, relevant `tsconfig` files, `package.json`, and `package-lock.json`.
- A sorted list of every packageable `dist/` output other than the manifest itself, with `dist`-relative path, byte length, and SHA-256 hash.

Repository text files used as manifest inputs have deterministic line endings enforced through `.gitattributes`; hashes are calculated from checked-out bytes. The generator rejects absolute paths, `..` traversal, duplicate paths, symlinks that resolve outside the repository, and outputs outside `dist/`.

Rust `build.rs` validates the manifest during every relevant Cargo build. It prints `cargo:rerun-if-changed` for the manifest and every declared input and output, recomputes every hash and output length, rejects missing or unexpected packageable `dist/` files, and rejects unsupported manifest schemas or build-tool versions. This detects stale, modified, incomplete, or mixed frontend output without invoking Node during Rust packaging.

The formal packaging command is:

```powershell
cargo tauri build
```

It must not invoke Node, npm, Vite, NCC, TSX, or a runtime download script.

## Error Handling

Stable collector error codes remain compatible with the current UI. They include browser discovery and lock failures, missing login validation, profile conflicts, CDP startup and connection failures, closed Targets, navigation and evaluation failures, unsupported page structure, virtualized-list detection, timeout, record limit, cancellation, and result-contract violations.

The collector writes sanitized diagnostics only. Cookies, local storage, authorization headers, complete profile contents, and arbitrary page payloads must never be written. Rust errors retain an internal source chain for logs while exposing only stable codes and Chinese user-facing messages to the UI.

Collector mode catches recoverable errors at the top level, writes diagnostics when task paths are available, attempts browser cleanup, and exits non-zero. Panics are treated as collector process failures by the desktop process and cannot crash the Tauri UI.

## Testing Strategy

Existing TypeScript collector tests are ported to Rust before the corresponding TypeScript production module is removed. Rust tests cover:

- Browser discovery priority, explicit selection, canonical path validation, and lock invalidation.
- Browser selection and login marker serialization.
- `DevToolsActivePort`, CDP request routing, disconnect propagation, retries, timeouts, and partial-connection cleanup.
- Navigation, Runtime evaluation, Fetch blocking, and Target closure.
- React snapshots for list growth, delayed fields, alternate Fibers, empty history, and virtualized-list rejection.
- ID, URL, history time, date, content type, like count, progress, and hash contracts.
- Range termination, sparse history, empty history, final stable windows, retries, hard timeout, record limit, and cancellation.
- Atomic result and diagnostic files.
- SQLite schema v2 validation and import behavior.
- Self-spawn collector invocation, two-second forced cancellation, browser PID validation, and startup recovery.

A Windows-only Rust integration fixture starts an in-process local HTTP server, serves embedded fixture assets, and launches the installed system browser with a temporary profile. It verifies Target creation, browser/page WebSocket connection, navigation, React snapshot extraction, Fetch handling, continuous scrolling, cancellation, Job Object cleanup, and validated restart recovery. The fixture must not invoke Node, npm, Python, PowerShell, or another external test server.

The Svelte/Vitest tests remain optional frontend-development checks. Their presence does not affect the Rust-only packaging gate.

## Acceptance Criteria

- Current UI, settings, pagination, deletion, like formats, login flow, synchronization options, schema v2 data, and browser profiles continue to work.
- `cargo test` passes all native collector, database, runtime, and recovery tests.
- The Windows CDP fixture passes against system Edge and can be forced to Chrome for acceptance.
- Three paired TypeScript-oracle and Rust-collector real-page runs over the same fixed range produce the same complete `(awemeId, sourcePosition, historyTimeMs)` identity sequence and identity hash. Stable normalized fields match exactly. Volatile presentation fields such as like count, title, author name, and cover URL are validated by contract but are not required to remain byte-identical across runs performed at different times.
- Visible synchronization, hidden synchronization, startup cancellation, scrolling cancellation, and restart recovery work.
- On the same acceptance machine, account, browser, and frozen seven-day range, the median duration of three Rust runs is no more than 125% of the median duration of three TypeScript-oracle runs, no run reaches the hard timeout or record limit, and no bottom-page popular content is collected.
- A clean Windows x64 environment without Node or npm successfully runs `cargo tauri build` using committed frontend assets.
- In that clean environment, `Get-Command node`, `Get-Command npm`, and `Get-Command npx` all fail before the build, proving that the successful packaging path cannot use an accidentally installed Node toolchain.
- The installed application synchronizes without Node or npm installed.
- Source, Tauri resources, executable payloads, and NSIS contents contain no bundled `node.exe`, npm, NCC output, `index.cjs`, `chrome-remote-interface`, Playwright, Chromium, or Headless Shell.
- The formal packaging path performs no network download of a Node runtime.
- A clean checkout contains all manifest-declared frontend inputs and outputs, and modifying, deleting, or adding an undeclared packageable `dist/` file causes `cargo build` and `cargo tauri build` to fail before bundling.

## Migration And Removal Order

The TypeScript collector remains available as a development oracle until the Rust implementation passes fixture and real-page sequence comparison. Database schema and user data are not cleared.

After parity gates pass, production invocation switches to the Rust collector mode. Only then are Node runtime preparation, collector bundling, production TypeScript collector sources, Tauri Node resource paths, and the blanket `dist/` ignore rule removed or replaced. Packageable frontend outputs are added to Git in the same migration commit that enables the Rust-only packaging gate. Svelte sources, frontend-only tests, and optional frontend development dependencies remain.

The branch is not eligible to merge until a Rust-only NSIS package passes installation, login, visible sync, hidden sync, cancellation, restart recovery, and package-content inspection on a machine without Node.
