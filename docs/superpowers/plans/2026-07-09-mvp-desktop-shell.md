# MVP Desktop Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the stage-two MVP desktop shell for manual Douyin history sync, local history viewing, search, and sync result visibility.

**Architecture:** Use Svelte + Vite for the desktop UI and Tauri 2 for the native shell. The UI calls Tauri commands for `list_history`, `start_sync`, and `get_sync_status`; during plain Vite development it falls back to a local demo adapter so the interface remains buildable without Rust. The Rust command layer reads/writes the same SQLite file under `data/history.sqlite` and launches the existing Node prototype command for manual sync.

**Tech Stack:** Tauri 2, Rust, Svelte, Vite, TypeScript, SQLite, existing Playwright prototype CLI.

## Global Constraints

- Stage two remains focused on viewing history only.
- Do not implement likes, favorites, favorite folders, tray, autostart, background scheduler, or analytics in this milestone.
- Do not invent exact `watched_at`; keep `page_group_label` and `collected_at` as separate time fields.
- Reuse the existing Playwright profile and prototype sync command.
- Preserve JSON output for debugging.
- Repeated syncs must not duplicate local history records.
- UI should be a quiet desktop tool, not a marketing page.
- Rustup and Visual Studio 2022 Build Tools C++ workload are installed on the current machine.

---

## File Structure

- Create `app/`: Svelte + Vite UI source.
- Create `app/src/app-api.ts`: Tauri IPC wrapper with dev fallback.
- Create `app/src/App.svelte`: main desktop UI.
- Create `app/src/styles.css`: restrained app styling.
- Create `src-tauri/`: Tauri 2 Rust shell and command skeleton.
- Modify `package.json`: add Vite/Tauri scripts and dependencies.
- Modify `docs/douyin-history-archive-roadmap.md`: record MVP-1 progress and Rust prerequisite.

---

### Task 1: Svelte/Vite App Shell

**Files:**
- Create: `app/index.html`
- Create: `app/src/main.ts`
- Create: `app/src/App.svelte`
- Create: `app/src/app-api.ts`
- Create: `app/src/types.ts`
- Create: `app/src/styles.css`
- Create: `vite.config.ts`
- Modify: `package.json`

**Interfaces:**
- Produces: `listHistory(input: ListHistoryInput): Promise<ListHistoryResult>`
- Produces: `startSync(): Promise<SyncResult>`
- Produces: `getSyncStatus(): Promise<SyncStatus>`

**Verification:**
- `npm run app:build` must pass.
- The UI must show a sync button, search input, latest sync summary, and history list.

---

### Task 2: Tauri 2 Shell Skeleton

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/src/main.rs`
- Create: `src-tauri/capabilities/default.json`

**Interfaces:**
- Produces Tauri command: `list_history(query?: String, limit: usize, offset: usize)`
- Produces Tauri command: `start_sync()`
- Produces Tauri command: `get_sync_status()`

**Verification:**
- `npm run app:build` still passes.
- `npm run tauri:dev` is expected to require Rust on this machine until `rustc/cargo` are installed.

---

### Task 3: Wire Real Query And Sync

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `scripts/douyin-history-poc.ts` only if command output needs additional machine-readable status.

**Behavior:**
- `list_history` reads `data/history.sqlite`.
- `start_sync` runs `npm run prototype -- --no-prompt --db data/history.sqlite --out data/prototype/tauri-sync-<timestamp>.json`.
- `get_sync_status` returns latest `sync_jobs` row.
- Concurrent syncs are rejected with a clear error.

**Verification:**
- With Rust installed, `npm run tauri:dev` starts the desktop app.
- Manual sync writes SQLite and updates the list.

---

## Completion Status

Complete. `MVP-0 本地持久化内核` and `MVP-1 桌面应用壳` are both complete for the stage-two MVP scope.

Completed on 2026-07-09:

- Svelte/Vite app shell created under `app/`.
- Tauri 2 config and Rust command skeleton created under `src-tauri/`.
- Frontend supports opening Douyin, manual sync trigger, history list, keyword search, sync status, and error display.
- `npm run app:build` passes.
- Browser sanity check passed at `http://127.0.0.1:5173`; screenshot written to `data/prototype/mvp-desktop-shell-screenshot.png`.
- Search interaction and web-preview sync error display were checked with Playwright.
- Rustup and Visual Studio 2022 Build Tools C++ workload were installed on this machine.
- `cargo check` and `cargo test` pass for the Tauri command/query layer.
- `npm run tauri:build` builds the release executable with `--no-bundle`.
- `npm run tauri:bundle` builds an NSIS installer at `src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe`.
- The release executable starts and remains alive for at least 5 seconds.
- The sync command used by Tauri was smoke-tested with `npm run prototype -- --no-prompt --db data/history.sqlite --out data/prototype/tauri-sync-smoke.json`.
- Real desktop manual sync through Tauri IPC was verified from the release executable: clicking “同步一次” created `sync_jobs.id = 4`, inserted 23 videos and 23 watch records, and refreshed the UI from 552 to 575 local records.
- Windows release sync now explicitly calls `npm.cmd`; this fixes the Tauri-window error `program not found` from `Command::new("npm")`.
- Post-sync UI screenshot: `data/prototype/tauri-window-after-sync-success.png`.
- Search in the real Tauri shell was verified with keyword `Agent`, returning 9 matching local records.
- App close/relaunch persistence was verified; the relaunched release window reads 575 records from SQLite.
- Post-trial ABI fix: Tauri sync no longer passes `--db` to the Node prototype. Node now only collects history and writes JSON; Rust imports that JSON into SQLite with `rusqlite`. This avoids `better-sqlite3` native module failures when the installed app resolves a different Node.js version than the development shell.
- Post-trial UX fixes: release builds use the Windows GUI subsystem to avoid a black terminal window; Tauri sync passes `--headless` so the automated browser is hidden during collection; NSIS language is configured as `SimpChinese`.
- Verification after the fix: with the release executable and a PATH preferring system Node 22, clicking “同步一次” created `sync_jobs.id = 5`, inserted 128 videos and 137 watch records, and refreshed the UI from 575 to 712 local records. Screenshot: `data/prototype/tauri-window-after-abi-fix-sync.png`.
- List display now avoids raw URLs for missing titles, localizes source labels, and shows collected time as a fallback when Douyin page-group date is missing.
- Quiet-window fix: Windows child processes now use hidden creation flags. In release validation, clicking “打开抖音” opened the Douyin page without any visible console window; clicking “同步一次” created `sync_jobs.id = 8` with status `success` and no visible new Douyin/blank browser window was detected.
- White-window follow-up fix: `msedge` channel could still create a visible blank browser window in the user's installed app. The prototype now supports `--browser-executable`, and Tauri sync prefers the local Chrome executable in headless mode instead of the Edge channel. Release validation created `sync_jobs.id = 12` with status `success`, inserted 24 watch records, and no new visible browser window was detected.
- Additional white-window mitigation: headless browser launches now include offscreen/minimized window arguments (`--window-position=-32000,-32000`, `--window-size=1,1`, `--start-minimized`) so any transient browser window should be created outside the visible desktop. Release validation created `sync_jobs.id = 15` with status `success` and inserted 12 watch records.
- After Playwright Chromium/headless shell was installed, Tauri sync now defaults back to the Playwright bundled headless runtime (`--browser-channel none`). `--browser-executable` remains available only through `DOUYIN_SYNC_BROWSER_EXECUTABLE` for explicit override. Release validation created `sync_jobs.id = 16` with status `success`, inserted 114 watch records, and no new visible Chrome/Chromium/Edge window was detected.
- Windows firewall prompt mitigation: headless launches now disable Chromium background networking, extensions, Media Router/Cast features, and non-proxied WebRTC UDP. This is intended to avoid first-run Windows Security firewall prompts from local network discovery while preserving normal outbound page loading.
- Delete workflow added: the history list supports single-row deletion, selecting visible rows, deleting selected rows, and deleting the current search result set or all local history records. Deleting watch records also removes orphaned video rows while keeping sync job logs.
- Detail order changed to author, watch date, collection source, collected-at timestamp. Collected-at is displayed as `采集于 YY/MM/DD HH:mm`.

Known follow-up:

- Metadata completeness remains imperfect. At 758 local records, some items still lack author, cover, title, or Douyin page-group date. This needs extractor improvements rather than UI-only fallback.

## Self-Review

**Spec coverage:** Covers Tauri shell, manual sync, history list, keyword search, sync result visibility, and failure display. Does not cover tray/autostart/scheduler by design.

**Placeholder scan:** No `TBD` or unspecified implementation steps remain.

**Type consistency:** Frontend and Tauri command names match: `list_history`, `start_sync`, `get_sync_status`.
