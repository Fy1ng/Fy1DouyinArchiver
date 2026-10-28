# Douyin Home Login And Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Project rules prohibit implementation subagents.

**Goal:** Make account login start from the Douyin home page, safely persist successful identities, and recover one expired session invisibly before retrying the original history sync once.

**Architecture:** The native Rust collector gains separate visible `login`, hidden `recover-login`, and history-only `collect` modes. Tauri owns durable login attempts and sync orchestration, validates non-flat phase envelopes, linearizes cancellation at `consuming`, and writes login markers only after the controlled browser exits normally and releases its profile. Page scripts remain self-contained JavaScript embedded with `include_str!`; all database state transitions use immediate transactions.

**Tech Stack:** Rust 2021, Tokio, Tauri 2, rusqlite/SQLite, native CDP over tokio-tungstenite, Windows Job Objects, Svelte 5, TypeScript, Vitest.

## Global Constraints

- Windows x64 only; controlled system Microsoft Edge and Google Chrome profiles only.
- First login and manual relogin navigate to `https://www.douyin.com/`; history collection keeps `https://www.douyin.com/user/self?showTab=record`.
- Do not detect, solve, or bypass QR codes, sliders, SMS, passwords, or CAPTCHA pages.
- Stable identity requires `uid` or `secUid`; nickname and avatar never participate in matching.
- Hidden recovery may click exact text `一键登录` once and may click the unique `保存` button only in the verified save-login modal.
- Visible login keeps the existing ten-minute user deadline; hidden recovery has one absolute 30-second deadline plus a cleanup-only two-second allowance.
- `CollectorSuccessV3`, history hashes, schema v3 history content, and public Tauri command signatures remain unchanged.
- Every phase attempt ID is UUID v4 encoded as 32 lowercase hexadecimal characters without hyphens.
- Unknown image resources are allowed. Only rules proven by redacted real-page fixtures may block home-feed cover images.
- Collector children never delete or overwrite the task-level cancel file.
- Every production behavior follows RED -> GREEN -> relevant regression -> focused commit.
- Preserve unrelated user changes, especially the existing uncommitted `AGENTS.md` change.

---

### Task 1: Strict Home Session Probe And Identity Contracts

**Files:**
- Create: `src-tauri/collector-scripts/session-probe.js`
- Create: `src-tauri/src/collector/session_probe.rs`
- Modify: `src-tauri/src/collector/contracts.rs`
- Modify: `src-tauri/src/collector/mod.rs`
- Test: `tests/collector/session-probe.test.mjs`
- Test: `src-tauri/tests/native_collector.rs`
- Test fixture: `src-tauri/tests/fixtures/home-session-sources.json`

**Interfaces:**
- Produces: `SessionProbeV1::{Authenticated, Unauthenticated, Indeterminate, Conflicting}`.
- Produces: `ObservedHomeIdentityV1 { page_url, uid, sec_uid, nickname, avatar_url, sources }`.
- Produces: `probe_from_page_value(&Value) -> Result<SessionProbeV1, String>` and `is_valid_douyin_home_url(&str) -> bool`.
- Preserves: `AccountIdentityV1.history_url` is populated only from the Rust constant `DOUYIN_HISTORY_URL`.

- [ ] **Step 1: Add failing JavaScript fixture tests**

  Test the allowed state paths, explicit `isLogin === false`, missing booleans, split identities without a shared ID, conflicting IDs, and strict home URL rejection. The assertions must include:

  ```js
  expect(runProbe({ url: "https://www.douyin.com/", SSR_RENDER_DATA: authenticated }))
    .toMatchObject({ kind: "authenticated", identity: { secUid: "sec-a" } });
  expect(runProbe({ url: "https://www.douyin.com/", SSR_RENDER_DATA: loggedOut }))
    .toEqual({ kind: "unauthenticated", source: "ssr_render_data" });
  expect(runProbe({ url: "https://m.douyin.com/", SSR_RENDER_DATA: authenticated }).kind)
    .toBe("indeterminate");
  expect(runProbe({ url: "https://www.douyin.com/video/1", SSR_RENDER_DATA: authenticated }).kind)
    .toBe("indeterminate");
  ```

- [ ] **Step 2: Run the JavaScript test and verify RED**

  Run: `npx vitest run tests/collector/session-probe.test.mjs`

  Expected: FAIL because `session-probe.js` does not exist.

- [ ] **Step 3: Implement the self-contained probe script**

  Read only the confirmed versioned paths. Return a compact object with `kind`, evidence source enums, and normalized public identity fields. Do not traverse arbitrary object graphs, read cookies/storage, or include page text.

- [ ] **Step 4: Run the JavaScript test and verify GREEN**

  Run: `npx vitest run tests/collector/session-probe.test.mjs`

  Expected: all session-probe tests PASS.

- [ ] **Step 5: Add failing Rust contract and URL tests**

  Add deserialization tests that reject unknown fields and verify HTTPS, exact host, default port, root path, no credentials, and no fragment. Verify identity merge behavior:

  ```rust
  assert!(is_valid_douyin_home_url("https://www.douyin.com/?enter=home"));
  assert!(!is_valid_douyin_home_url("https://user@www.douyin.com/"));
  assert_eq!(probe.kind(), SessionProbeKind::Conflicting);
  ```

- [ ] **Step 6: Run the Rust test and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test native_collector session_probe -- --nocapture`

  Expected: FAIL because Rust probe contracts and conversion functions are missing.

- [ ] **Step 7: Implement Rust conversion and canonical history ownership**

  Add `DOUYIN_HOME_URL` and `DOUYIN_HISTORY_URL` constants. `ObservedHomeIdentityV1` must preserve stable IDs and evidence sources; `AccountIdentityV1` must never accept `historyUrl` from page output.

- [ ] **Step 8: Run focused and baseline tests**

  Run:

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test native_collector
  cargo test --manifest-path src-tauri/Cargo.toml --test account_login
  npm test
  ```

  Expected: PASS.

- [ ] **Step 9: Commit**

  ```powershell
  git add src-tauri/collector-scripts/session-probe.js src-tauri/src/collector/session_probe.rs src-tauri/src/collector/contracts.rs src-tauri/src/collector/mod.rs src-tauri/tests/native_collector.rs src-tauri/tests/fixtures/home-session-sources.json tests/collector/session-probe.test.mjs
  git commit -m "feat: add strict home session probe"
  ```

### Task 2: Durable Login Attempts Without Pending Accounts

**Files:**
- Create: `src-tauri/src/login_attempt.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/accounts.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/collector/contracts.rs`
- Modify: `src-tauri/src/collector/file_store.rs`
- Test: `src-tauri/tests/account_login.rs`
- Test: `src-tauri/tests/accounts.rs`
- Test: `src-tauri/tests/runtime.rs`

**Interfaces:**
- Produces: `LoginAttemptEnvelopeV1<T> { envelope_version, login_attempt_id, target_account_id, profile_key, payload }`.
- Produces: `LoginAttemptStateV1 { schema_version, revision, login_attempt_id, profile_key, target_account_id, lifecycle, observed_identity_hash, marker_validation_id, updated_at }`.
- Produces: `create_new_login_attempt`, `create_relogin_attempt`, `compare_and_swap_login_attempt`, `recover_login_attempts`.
- Removes new writes of `pending_login`; old pending rows remain startup-cleanup input only.

- [ ] **Step 1: Add failing tests for account invisibility and attempt CAS**

  Verify a new attempt creates files/profile identity but no `accounts` row, failed login leaves no account, revision mismatch rejects updates, and a relogin attempt is bound to the existing account/profile.

- [ ] **Step 2: Run focused tests and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test account_login durable_login_attempt -- --nocapture`

  Expected: FAIL because `LoginAttemptStateV1` and its state machine do not exist.

- [ ] **Step 3: Implement atomic login attempt state**

  Use same-directory temporary files and the existing atomic replacement helper. CAS compares `revision`, lifecycle, attempt ID, profile key, and optional target account ID before writing `revision + 1`.

- [ ] **Step 4: Replace pending account creation in commands**

  `begin_account_login` creates only an attempt and controlled profile key. `resume_account_login` reads an existing `login_expired` or `identity_conflict` account and creates a target-bound attempt. Keep the public returned `AccountView` behavior by returning only after successful finalization.

- [ ] **Step 5: Run focused tests and verify GREEN**

  Run:

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test account_login
  cargo test --manifest-path src-tauri/Cargo.toml --test accounts
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime
  ```

  Expected: PASS.

- [ ] **Step 6: Commit**

  ```powershell
  git add src-tauri/src/login_attempt.rs src-tauri/src/lib.rs src-tauri/src/accounts.rs src-tauri/src/main.rs src-tauri/src/collector/contracts.rs src-tauri/src/collector/file_store.rs src-tauri/tests/account_login.rs src-tauri/tests/accounts.rs src-tauri/tests/runtime.rs
  git commit -m "refactor: persist login attempts before accounts"
  ```

### Task 3: Safe CDP Login Actions

**Files:**
- Create: `src-tauri/collector-scripts/login-action-target.js`
- Create: `src-tauri/src/collector/login_actions.rs`
- Modify: `src-tauri/src/collector/cdp.rs`
- Modify: `src-tauri/src/collector/mod.rs`
- Test: `tests/collector/login-action-target.test.mjs`
- Test: `src-tauri/tests/cdp_transport.rs`

**Interfaces:**
- Produces: `LoginAction::{OneClickLogin, SaveLogin}` and `ActionTarget { x, y, token }`.
- Produces: `CdpPage::dispatch_left_click(x: f64, y: f64, deadline: Deadline) -> Result<(), String>`.
- Produces: `find_and_click_login_action(page, action, deadline) -> Result<ActionOutcome, String>`.

- [ ] **Step 1: Add failing DOM fixture tests**

  Cover exact text, similar text, hidden/disabled buttons, wrong containers, multiple candidates, overlays, and modal disappearance. Verify no target is returned unless `elementFromPoint` hits the button or its descendant.

- [ ] **Step 2: Run JavaScript test and verify RED**

  Run: `npx vitest run tests/collector/login-action-target.test.mjs`

  Expected: FAIL because the action script is missing.

- [ ] **Step 3: Implement target discovery script**

  The script returns coordinates plus a short-lived structural token. Before input dispatch, evaluate the script again and require matching URL, action, text, container, visibility, and coordinates.

- [ ] **Step 4: Add failing CDP input tests**

  Extend the test WebSocket server to assert `Input.dispatchMouseEvent` sends `mousePressed` then `mouseReleased`, both with left button and matching coordinates.

- [ ] **Step 5: Run Rust test and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test cdp_transport dispatch_left_click -- --nocapture`

  Expected: FAIL because CDP input dispatch is missing.

- [ ] **Step 6: Implement safe input dispatch and outcomes**

  `一键登录` is allowed once per recovery attempt. `保存` waits up to five seconds for the optional modal; absence is success, but a discovered modal must be clicked and disappear before success.

- [ ] **Step 7: Run focused tests and commit**

  Run:

  ```powershell
  npx vitest run tests/collector/login-action-target.test.mjs
  cargo test --manifest-path src-tauri/Cargo.toml --test cdp_transport
  ```

  Expected: PASS.

  Commit:

  ```powershell
  git add src-tauri/collector-scripts/login-action-target.js src-tauri/src/collector/login_actions.rs src-tauri/src/collector/cdp.rs src-tauri/src/collector/mod.rs tests/collector/login-action-target.test.mjs src-tauri/tests/cdp_transport.rs
  git commit -m "feat: add guarded CDP login actions"
  ```

### Task 4: Lossless Login Resource Policy

**Files:**
- Create: `src-tauri/src/collector/resource_policy.rs`
- Create: `src-tauri/tests/fixtures/home-resource-policy.json`
- Modify: `src-tauri/src/collector/cdp.rs`
- Modify: `src-tauri/src/collector/mod.rs`
- Test: `src-tauri/tests/cdp_transport.rs`
- Test: `src-tauri/tests/native_collector.rs`

**Interfaces:**
- Produces: `classify_login_resource(stage, resource_type, url) -> ResourceDecision`.
- Produces: `FetchTracker` with `Seen -> Queued -> Decided` lifecycle and terminal counters.
- Produces: `CdpPage::enable_login_resource_policy` and `shutdown_resource_policy`.

- [ ] **Step 1: Add failing pure policy tests**

  Assert all `Media` fail; known fixture feed covers fail; QR, CAPTCHA, avatar, modal, `data:`, `blob:`, malformed, redirected, and unknown images continue. No broad `douyinpic.com` domain rejection is allowed.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test native_collector login_resource_policy -- --nocapture`

  Expected: FAIL because the policy module is missing.

- [ ] **Step 3: Implement the pure classifier**

  Normalize origin and path only. Query signatures are ignored. On classifier error return `Continue`.

- [ ] **Step 4: Add failing Fetch lifecycle tests**

  Simulate duplicates, queue saturation, shutdown during pause, late events, disable failure, and socket close. Normal completion must assert:

  ```rust
  assert_eq!(stats.paused, stats.decided);
  assert_eq!(stats.duplicates, 0);
  assert_eq!(stats.pending, 0);
  ```

- [ ] **Step 5: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test cdp_transport fetch_tracker -- --nocapture`

  Expected: FAIL because current CDP code globally fails paused requests and has no tracker.

- [ ] **Step 6: Implement high-priority Fetch routing and shutdown**

  Stop accepting normal events, immediately continue late pauses, drain and acknowledge queued decisions, call `Fetch.disable`, then stop the handler. Duplicate request IDs, queue overflow, or connection failure abort the session and close the target/browser.

- [ ] **Step 7: Run focused tests and commit**

  Run:

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test native_collector
  cargo test --manifest-path src-tauri/Cargo.toml --test cdp_transport
  ```

  Expected: PASS.

  Commit the policy, fixture, CDP changes, and tests with `feat: add lossless login resource policy`.

### Task 5: Deadline-Aware CDP And Home Login Completion

**Files:**
- Create: `src-tauri/src/collector/deadline.rs`
- Modify: `src-tauri/src/collector/cdp.rs`
- Modify: `src-tauri/src/collector/session.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/src/collector/cli.rs`
- Modify: `src-tauri/src/collector/browser_state.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/native_collector.rs`
- Test: `src-tauri/tests/account_login.rs`
- Test: `src-tauri/tests/cdp_transport.rs`
- Test: `src-tauri/tests/runtime.rs`

**Interfaces:**
- Produces: `Deadline::from_unix_ms`, `remaining`, and `run`.
- Changes visible login CLI to `--home-url`; adds `--deadline-at-ms` only for bounded internal stages.
- Produces `LoginValidationV2` with `validation_id`; V1 remains readable.
- Collector login output is written only after `Browser.close`, normal process exit, and profile lock release.

- [ ] **Step 1: Add failing deadline tests**

  Verify remaining budget is used by browser startup, navigation, evaluation, optional modal wait, close, and profile release. A slow inner call may not receive a fresh 60-second timeout.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test cdp_transport deadline -- --nocapture`

  Expected: FAIL because CDP calls use fixed 60-second timeouts.

- [ ] **Step 3: Implement deadline propagation**

  Thread `Deadline` through connect, call, navigate, evaluate, click, resource shutdown, and close. Preserve the visible login ten-minute deadline; hidden recovery will pass a 30-second absolute deadline later.

- [ ] **Step 4: Add failing home login tests**

  Assert `build_login_invocation` passes the home URL, collector never navigates to history, page-provided `historyUrl` is ignored, result appears only after normal browser exit, and V2 marker deletion requires matching `validationId`.

- [ ] **Step 5: Run and verify RED**

  Run:

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime login_invocation
  cargo test --manifest-path src-tauri/Cargo.toml --test account_login home_login
  ```

  Expected: FAIL under the current history-page login flow.

- [ ] **Step 6: Implement visible home login**

  Navigate home, wait for authenticated `SessionProbeV1`, process the optional save modal, close normally, verify profile release, and write only a `LoginAttemptEnvelopeV1<ObservedHomeIdentityV1>`. Tauri validates output and owns marker/database finalization.

- [ ] **Step 7: Implement recoverable marker finalization**

  Within `BEGIN IMMEDIATE`, recheck stable IDs, write V2 marker with the attempt validation ID, then insert/update account and commit. Marker failure rolls back. Commit failure removes only the exact new V2 marker. Startup uses attempt state, marker validation ID, and database identity to finish or clean an interrupted finalization.

- [ ] **Step 8: Run regression and commit**

  Run all four focused test binaries plus `cargo test --manifest-path src-tauri/Cargo.toml`.

  Expected: PASS.

  Commit with `feat: complete account login from Douyin home`.

### Task 6: Collection Session Expiry Detection

**Files:**
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/src/collector/errors.rs`
- Modify: `src-tauri/src/collector/contracts.rs`
- Test: `src-tauri/tests/native_collector.rs`
- Test: `src-tauri/tests/cdp_system_browser.rs`

**Interfaces:**
- `collect` returns stable `session_expired` only for `SessionProbeV1::Unauthenticated`.
- Indeterminate/conflicting probes preserve their technical errors and never masquerade as expiry.

- [ ] **Step 1: Add failing readiness race tests**

  Use a Rust fixture page that exposes delayed history, explicit logout, missing state, and conflicting state. Race snapshot readiness, page closure, cancellation, timeout, and session probe.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test native_collector collection_session -- --nocapture`

  Expected: FAIL because `wait_ready` does not read a session probe.

- [ ] **Step 3: Implement parallel session probing**

  During history readiness waits, explicit unauthenticated ends immediately with `session_expired`. Authenticated continues. Indeterminate does not overwrite navigation, script, or history readiness errors.

- [ ] **Step 4: Run tests and commit**

  Run native collector tests and the ignored Edge fixture once in visible mode.

  Commit with `feat: detect expired sessions during collection`.

### Task 7: Phase Envelopes, Paths, And Durable Orchestration

**Files:**
- Create: `src-tauri/src/sync_orchestration.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/collector/contracts.rs`
- Modify: `src-tauri/src/collector/cli.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Test: `src-tauri/tests/runtime.rs`
- Test: `src-tauri/tests/native_collector.rs`

**Interfaces:**
- Produces non-flat `SyncPhaseEnvelopeV1<T> { envelope_version, account_id, sync_job_id, phase, phase_attempt_id, payload }`.
- Produces `SyncOrchestrationV1 { schema_version, revision, account_id, sync_job_id, phase, phase_attempt_id, lifecycle, login_recovery_count, cancel_requested_at, options_hash, deadline_at_ms, updated_at }`.
- Produces task paths and phase paths containing account, job, phase, and attempt identity.
- Produces `cas_orchestration(expected, next)` that compares revision plus phase, attempt, and lifecycle.

- [ ] **Step 1: Add failing envelope/path tests**

  Reject raw payloads, unknown fields, incorrect account/job/phase/attempt, invalid UUID text, and payload field collisions. Verify serializing/deserializing the nested `CollectorSuccessV3` preserves its exact bytes and hashes.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test runtime phase_envelope -- --nocapture`

  Expected: FAIL because paths and envelopes are task-level only.

- [ ] **Step 3: Implement path and envelope contracts**

  The task cancel path remains `(account_id, sync_job_id)` only. Result, progress, diagnostic, browser PID, collector identity, and log paths include the phase attempt identity. Collector code may read but never remove the task cancel path.

- [ ] **Step 4: Add failing orchestration CAS tests**

  Test stale revision, wrong lifecycle, wrong attempt, crash between preparing/running, and atomic replacement. Assert every successful update increments exactly one revision.

- [ ] **Step 5: Implement durable orchestration and run tests**

  Run runtime and native collector tests. Expected: PASS.

- [ ] **Step 6: Commit**

  Commit with `feat: persist sync phase orchestration`.

### Task 8: Owned Collector Processes And Recover Login CLI

**Files:**
- Create: `src-tauri/src/collector_process.rs`
- Modify: `src-tauri/src/collector/browser_process.rs`
- Modify: `src-tauri/src/collector/cli.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: `src-tauri/tests/runtime.rs`
- Test: `src-tauri/tests/browser_native.rs`
- Test: `src-tauri/tests/native_collector.rs`

**Interfaces:**
- Adds `CollectorCommand::RecoverLogin(RecoverLoginOptions)`.
- Produces `CollectorProcessIdentityV1` containing PID, Windows creation time, canonical executable, complete argument hash, phase identity, and start time.
- Produces suspended-process launch into a Tauri-owned `KILL_ON_JOB_CLOSE` Job Object before resume.

- [ ] **Step 1: Add failing CLI and process identity tests**

  Verify `recover-login` requires phase identity, expected identity path, absolute deadline, hidden mode, and scoped files. Verify PID reuse, wrong executable, wrong argument hash, and wrong attempt cannot be terminated.

- [ ] **Step 2: Run and verify RED**

  Run:

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test native_collector recover_login
  cargo test --manifest-path src-tauri/Cargo.toml --test browser_native collector_process
  ```

  Expected: FAIL because the mode and owner do not exist.

- [ ] **Step 3: Implement recover CLI and collector ownership**

  Tauri creates collector suspended, assigns its Job Object, atomically writes identity, registers current child while holding the orchestration lock, then resumes. Collector owns its browser in a nested/compatible Job Object and retains browser PID identity until Tauri consumes it.

- [ ] **Step 4: Preserve GUI/collector startup split**

  Keep `--collector` parsing before `tauri::Builder` and any single-instance initialization. Add a regression test that collector mode still starts while a GUI instance marker exists.

- [ ] **Step 5: Run tests and commit**

  Run runtime, browser-native, native-collector, and full Rust tests. Commit with `feat: own phased collector processes`.

### Task 9: Schema V4 Identity Conflict State

**Files:**
- Create: `src-tauri/migrations/004_identity_conflict.sql`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/accounts.rs`
- Test: `src-tauri/tests/storage_v3.rs`
- Test: `src-tauri/tests/accounts.rs`
- Test: `src-tauri/tests/account_login.rs`

**Interfaces:**
- Adds account status `identity_conflict` without changing history content tables.
- Produces `finish_sync_with_account_state(connection, job, outcome)` as one immediate transaction.
- Hidden recovery never fills a previously missing local ID.

- [ ] **Step 1: Add failing migration tests**

  Verify v3 -> v4 preserves all account and child rows, accepts `identity_conflict`, rejects unknown status, restores foreign keys after success/failure, rolls back version on injected failure, and passes `foreign_key_check`.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3 schema_v4 -- --nocapture`

  Expected: FAIL because schema v3 CHECK rejects the new status.

- [ ] **Step 3: Implement safe accounts-only migration**

  On one exclusive connection: set `foreign_keys=OFF` outside the transaction; `BEGIN IMMEDIATE`; rebuild/copy accounts and indexes; set `user_version=4`; run transaction `foreign_key_check`; commit; in all outcomes restore `foreign_keys=ON` and assert it again.

- [ ] **Step 4: Add failing outcome transaction tests**

  Assert `login_recovery_failed` and retry `session_expired` set account `login_expired` plus job `failed`; `account_identity_conflict` sets `identity_conflict` plus job `failed`; technical failure and cancellation leave the account unchanged.

- [ ] **Step 5: Implement account outcome transaction and run tests**

  Run storage, accounts, and account-login tests. Commit with `feat: add account identity conflict state`.

### Task 10: Three-Phase Sync Driver And Linearized Cancellation

**Files:**
- Create: `src-tauri/src/sync_driver.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/accounts.rs`
- Test: `src-tauri/tests/runtime.rs`

**Interfaces:**
- Phases: `collecting`, `recovering_login`, `retrying_after_login`.
- Lifecycles: `preparing`, `running`, `consuming`.
- Produces `PhaseCompletionDecision::{Consume, Cancel, FailInterrupted, StartRecovery, StartRetry}` shared by live execution and startup recovery.
- `consuming` is the cancellation linearization point.

- [ ] **Step 1: Add failing deterministic transition tests**

  Use barriers at every transition: before preparing, after preparing/before spawn, after spawn/before resume, running result arrival, before consuming CAS, after consuming CAS, and between recovery success and retry spawn. Assert no next stage starts after persistent cancellation.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --bin douyin-history-archive sync_driver -- --nocapture`

  Expected: FAIL because `RunningSync` owns one immutable child.

- [ ] **Step 3: Implement the locked phase driver**

  Start order under one mutex: verify database running and no cancel; write preparing CAS; spawn/register child; write running CAS; resume. First `session_expired` is internal. Recovery can occur once. Successful recovery must write marker and CAS retry preparing before its original 30-second deadline; actual retry spawn rechecks cancellation and uses normal sync limits.

- [ ] **Step 4: Implement cancellation rules**

  For `preparing`/`running`, persist `cancel_requested_at` and task cancel file before signaling child. For `consuming`, return `sync_finalizing` without writing cancel. Two seconds after cooperative cancellation, terminate only the identity-matched collector and browser.

- [ ] **Step 5: Run focused tests and commit**

  Run all main binary tests and runtime tests. Commit with `feat: orchestrate login recovery during sync`.

### Task 11: Deterministic Startup Recovery And Cleanup

**Files:**
- Modify: `src-tauri/src/sync_driver.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/runtime.rs`
- Test: `src-tauri/tests/browser_native.rs`

**Interfaces:**
- Startup consumes only the current persisted phase attempt.
- `consuming` current collect/retry results may complete; merely present orphan result files may not.
- Startup never resumes hidden recovery or retry.

- [ ] **Step 1: Add failing recovery matrix tests**

  Cover all three phases and lifecycles, valid/missing/corrupt orchestration, old PID, PID reuse, wrong args, result before cancel, cancel before consuming, consuming result, recovering marker, and stale prior-attempt output.

- [ ] **Step 2: Run and verify RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --bin douyin-history-archive startup_recovery -- --nocapture`

  Expected: FAIL under current task-level recovery.

- [ ] **Step 3: Implement the shared decision matrix**

  Exact order: validate DB running plus orchestration; terminate only exact current identities; consume only persisted `consuming`; otherwise honor persistent cancellation; mark remaining jobs failed/interrupted; do not run another phase; clean phase files, task cancel, and orchestration last.

- [ ] **Step 4: Run tests and commit**

  Run runtime, browser-native, and full Rust tests. Commit with `fix: make sync restart recovery deterministic`.

### Task 12: Desktop Account And Recovery UI

**Files:**
- Modify: `app/src/types.ts`
- Modify: `app/src/accounts.ts`
- Modify: `app/src/formatters.ts`
- Modify: `app/src/account-login-ui.ts`
- Modify: `app/src/App.svelte`
- Modify: `app/src/App.test.ts`
- Test: `tests/app/accounts.test.ts`
- Test: `tests/app/formatters.test.ts`

**Interfaces:**
- Adds `AccountStatus = ... | "identity_conflict"`.
- Adds phase labels for `recovering_login` and `retrying_after_login`.
- Restores visible action label `一键登录`.
- Identity-conflict accounts cannot sync but may relogin, archive, or delete.

- [ ] **Step 1: Add failing UI unit/component tests**

  Assert status labels, action availability, exact login button copy, no transient first-expiry error, cancellation remains available during recovery/retry, and final errors map separately for login expiry and identity conflict.

- [ ] **Step 2: Run and verify RED**

  Run:

  ```powershell
  npx vitest run tests/app/accounts.test.ts tests/app/formatters.test.ts app/src/App.test.ts
  ```

  Expected: FAIL because the new status and phases are absent.

- [ ] **Step 3: Implement minimal UI state changes**

  Do not change public commands. Keep sync settings locked through all phases. Display final errors only after backend transaction completion.

- [ ] **Step 4: Run frontend checks and rebuild committed dist**

  Run:

  ```powershell
  npm test
  npm run typecheck
  npm run app:build
  ```

  Expected: PASS and `dist/build-manifest.json` matches all changed frontend inputs/outputs.

- [ ] **Step 5: Commit**

  Stage only app/tests and generated `dist` outputs. Commit with `feat: show login recovery account states`.

### Task 13: System Browser, Restart, And Packaging Gates

**Files:**
- Modify: `src-tauri/tests/cdp_system_browser.rs`
- Create: `docs/testing/home-login-recovery-verification.md`
- Modify: `docs/superpowers/specs/2026-07-14-douyin-home-login-recovery-design.md`

**Interfaces:**
- Provides reproducible Edge/Chrome commands and a redacted acceptance report.
- No production API changes.

- [ ] **Step 1: Extend Rust-only system-browser fixtures**

  Add home authentication, optional save modal, one-click recovery, CAPTCHA/QR pass-through, identity conflict, transition cancellation, and Fetch terminal counter checks. Do not use Node/Python/PowerShell test servers.

- [ ] **Step 2: Run the complete automatic gate**

  Run:

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml
  npm test
  npm run typecheck
  npm run app:build
  cargo tauri build
  ```

  Expected: all commands PASS; NSIS bundle builds without Node production resources.

- [ ] **Step 3: Run Edge and Chrome system fixtures**

  Run:

  ```powershell
  $env:EMILY_TEST_BROWSER='edge'; cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1
  $env:EMILY_TEST_BROWSER='chrome'; cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1
  ```

  Expected: PASS on installed browsers; each terminal fixture leaves no process, profile lock, debug port, or pending Fetch after two seconds.

- [ ] **Step 4: Perform quantified real-page acceptance**

  Edge and Chrome each run hidden recovery ten times. Each recovery must finish within 30 seconds. Capture HWND/taskbar enumeration and 10-fps desktop recording from phase start through two seconds after terminal state; no non-app white/browser composition surface may appear. Verify normal sync skips recovery, retry occurs once, CAPTCHA requires user action, and identity mismatch imports no history.

- [ ] **Step 5: Record evidence and update the design status**

  Write only counts, durations, error codes, redacted URL classes, hashes, and cleanup observations. Do not record account IDs, cookies, QR/CAPTCHA content, or DOM text snapshots.

- [ ] **Step 6: Commit verification artifacts**

  ```powershell
  git add src-tauri/tests/cdp_system_browser.rs docs/testing/home-login-recovery-verification.md docs/superpowers/specs/2026-07-14-douyin-home-login-recovery-design.md
  git commit -m "test: verify home login recovery workflow"
  ```

## Completion Checklist

- [ ] `git status --short` contains only the user's pre-existing `AGENTS.md` change.
- [ ] Every new behavior was observed failing before its implementation.
- [ ] Full Rust tests, frontend tests, typecheck, frontend build, and NSIS build pass.
- [ ] Edge and Chrome real-page acceptance meets the quantified thresholds.
- [ ] Source, dist, resources, and installer contain no Node runtime, Playwright, Chromium, or Headless Shell.
- [ ] No secrets, cookies, QR/CAPTCHA data, account identifiers, or unredacted diagnostics are staged.
