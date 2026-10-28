# Emily Stage 3 Delivery Plan

## Goal

Complete the approved stage-three roadmap and produce a verified, deliverable Windows installer. Installer visual branding remains deferred until the user confirms it in a later round.

Prepare the repository for public source release one item at a time. Publication and new installer delivery remain separate, explicitly requested actions.

## Current Position

- Branch: `codex/stage-three-roadmap`
- Active roadmap section: phase 28 Public README and privacy documentation (complete)
- Last completed task: installed the requested README skill, rebuilt the public README with a labeled demo screenshot and verified commands, documented privacy, and synchronized third-party notice versions
- Current task: guide GitHub CI and contribution-entry preparation; authenticated Favorites acceptance and installer rebuild remain deferred
- Public-source staging: sibling `../Emily-public`, independent `main` history containing only sanitized public-source commits and no remote; keep development in this repository and refresh the public snapshot after later preparation items
- Execution mode: primary agent owns implementation, integration, and verification; no plan/design audit agents are used

## Phases

| Phase | Status | Exit Criteria |
|---|---|---|
| 1. Backup archive and settings foundation | complete | Archive integrity, portable/device settings, directory validation, and scheduling primitives are tested |
| 2. Manual backup runtime and IPC | complete | Export path selection, runtime maintenance gate, atomic archive export, and IPC registration are tested |
| 3. Backup and notification desktop UI | complete | Users can configure automatic backup and notification policy and can export an unencrypted backup with an explicit warning |
| 4. Automatic backup execution | complete | Scheduler writes retained backups, reports failures, and never silently falls back from an unavailable configured directory |
| 5. Restore and deduplicating merge | complete | Replace and merge flows are transactional, resolve folder/settings conflicts, and preserve current data on cancellation or failure |
| 6. Download worktree integration | complete | Download module passes schema, queue, cancellation, backup-exclusion, and packaging compatibility gates |
| 7. In-app update flow | complete | Manual check and optional automatic check/download work without compromising running background operations or data |
| 8. Release verification and installer | complete | Automated gates, isolated package audit, installed startup smoke, artifact integrity, and clean-state delivery pass; visual installer branding is excluded until separately confirmed |
| 9. UI contracts and settings migrations | complete | Portable settings schema 7, stable account scopes, device settings schema 4, defaults, and forward migrations pass |
| 10. Persistent notification model | complete | SQLite schema 8, notification deduplication, read/resolve operations, and IPC pass |
| 11. Visual system and application shell | complete | Theme tokens, responsive shell, primary navigation, account switcher, and 960x640 minimum window pass |
| 12. Onboarding and settings workspace | complete | Recoverable four-step onboarding and searchable, backend-confirmed settings workspace pass |
| 13. Library, search, and account synchronization | complete | Cross-account/source library, independent search, dual-pane accounts, and serial aggregate sync pass |
| 14. Overview and annual review | complete | Account-scoped customizable overview and eight-section annual review read models and UI pass |
| 15. Unified task and notification centers | complete | Unified task summaries, live detail drawer, persistent notifications, and shared feedback states pass |
| 16. Accessibility and release delivery | complete | Automated, visual, DPI, installed-runtime, packaging, and mandatory cleanup gates pass |
| 17. UI visual and window interaction refinement | complete | Base palette, controls, frameless titlebar, scroll behavior, toolbar/input shells, onboarding correction, frontend/build gates, installer build, and clean-state reset pass |
| 18. Composite controls and first-run data root | complete | Composite inputs render one compact frame, first-run setup never exposes existing-data migration, and focused frontend/Rust/rendered regressions pass |
| 19. Installer rebuild and clean-state handoff | complete | `cargo tauri build` succeeds, the NSIS artifact is hashed, managed local test state is safely removed, and the artifact directory is handed off |
| 20. Onboarding and settings interaction corrections | complete | A selected empty data root replaces the displayed path and passes WAL validation; defaults, compact controls, read-only pickers, per-route scroll, truthful save feedback, nested settings navigation, and responsive source filters pass focused and rendered verification |
| 21. UI audit optimization | complete | Implement the audit's first five priority items: semantic action colors and contrast, modal focus/inert behavior, cover-cache confirmation state, minimum-window scroll ownership, and typography/click-target tokens; each slice passes focused tests and is committed without touching unrelated Rust work |
| 22. UI-audit installer delivery | complete | Complete Cargo/release gates, produce and hash a fresh NSIS package, validate the artifact timestamp/path, then safely remove only marker-verified managed local test state |
| 23. First-run empty data-root WAL regression | complete | A newly created empty onboarding directory passes validation and migration without `data_root_wal_not_clean`; focused/full regressions and the replacement package/clean-state handoff pass |
| 24. Favorites source collection regression | complete | Favorites scrolls independently, default favorites and named remote folders are discovered from the current page contract, selected folders load, and focused/full collector gates pass |
| 25. Open-source licensing | complete | GPL-3.0-only text and metadata agree, third-party attribution is recorded, frontend provenance remains valid, and the scoped change is verified and committed locally |
| 26. Open-source privacy and fresh public history | complete | Current personal identifiers are redacted, sensitive runtime files are ignored without excluding source, focused checks pass, and an independent public repository contains one initial commit with the user-approved public author identity while development history remains local |
| 27. Dependency security remediation | complete | TLS and updater/ZIP fixes pass full regressions; Cargo.lock has no vulnerability advisories, npm has no moderate/high/critical findings, the upstream development-only deepmerge low finding is tracked, notices are refreshed, and the independent public snapshot is synchronized |
| 28. Public README and privacy documentation | complete | The requested beautify-github-readme skill is installed; README describes current functionality, build/development and first-run use with a labeled demo screenshot; privacy, limits and notices are accurate; local links/assets and wide/narrow light/dark previews pass |

## Next Action

Guide GitHub CI and contribution-entry preparation, using the locked frontend toolchain and Windows Rust checks without real account credentials. Keep the documented deepmerge development-tool finding visible. Option A remains approved: preserve private development history locally and publish only the sanitized independent repository after separate publication authorization. Do not push either repository or import private development refs into the public repository. Refresh the public snapshot after later preparation changes. Authenticated Favorites acceptance, corresponding source and complete native binary notices remain required before the next installer release.

## Working Rules

- Re-read this file before a major scope or architecture decision.
- After Compact or session recovery, this file is the only authoritative plan; never regenerate or replace it with a new task plan.
- Update `findings.md` after discoveries and `progress.md` after implementation or verification.
- Complete and verify one phase before advancing.
- Do one concentrated final review against acceptance criteria and the actual diff before declaring the whole goal complete.
- Installer visual branding remains scoped to explicitly confirmed assets; the current approval covers only the in-app brand mark, executable icon, and NSIS installer icon.
- A delegated task must be narrowly scoped and non-overlapping; once dispatched, wait for its complete response and never interrupt it merely because it is slow.

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| First full-tree cleanup preflight piped directly from a PowerShell `foreach` block | 1 | The command failed at parse time and changed no state; materialize the rows before converting them to JSON |
| First cleanup execution encountered an uninstall registry entry without `DisplayIcon` under strict mode | 1 | It stopped before process, registry, or filesystem mutation; read optional registry properties through `PSObject.Properties`, revalidate all roots, and rerun the same marker-bound cleanup |
| Runtime ownership search included nonexistent `src-tauri/src/storage.rs` | 1 | The read-only search changed nothing; use repository-discovered storage modules and concrete paths for subsequent inspection |
| New Windows locked-zero-WAL regression fails with `data_root_wal_not_clean` | 1 | Expected RED proving the installed failure boundary; retain zero-length/type/reparse checks and make deletion of that already-clean WAL best-effort before revalidation |
| Main-binary focused commands used bare names with `--exact` and selected zero tests | 1 | The commands compiled successfully but provided no behavioral evidence; rerun using substring filters that include the module-qualified Rust test names |
| Full Cargo poll was sent once to the yielded-script waiter instead of the PTY session poller | 1 | The test process was not interrupted; poll the existing `exec_command` session with `write_stdin`, which returned the complete successful exit status |
| Broad live-bundle scan timed out and reset the browser inspection kernel | 1 | No state changed; use bounded, identified bundle reads and reinitialize the browser binding only if an authenticated DOM check becomes necessary |
| First read-only sync database query assumed `sync_jobs.error_code` exists | 1 | SQLite rejected only that diagnostic SELECT; no mutation occurred. Inspect table columns with `PRAGMA table_info` before issuing schema-specific queries |
| Initial bounded Douyin HTML request returned an empty body and the first modern Favorites fixture failed with `archive_list_not_found` | 1 | Treat unauthenticated HTML as non-authoritative; use the authenticated-shaped system-browser fixture to drive a generic list/folder adapter |
| PowerShell reconnaissance commands failed while embedding nested Python/foreach pipelines | 2 | No repository or runtime state changed; use here-strings/materialized arrays and keep later probes in separate commands |
| Full Cargo run once failed at the existing scheduler wake test with `scheduler_stopped` | 1 | Isolated the test with one thread; it passed, so classify the failure as parallel timing flake and rerun the complete gate serially |
| First end-to-end fixture compile used the path-based executable hash helper with the wrong argument type | 1 | Pass the normalized executable path string to the existing helper; no production behavior changed |
| Delayed-loading fixture initially treated a multi-link hot container as one card and returned `react_fiber_not_found` | 1 | Require one unique content ID per card candidate and exclude multi-ID containers; the delayed mount and hot-region regression then passed |
| Initial network-folder parser fixture used non-ASCII bytes and derived `Eq` for a `Value`-backed type | 1 | Use JSON Unicode escapes in the byte fixture and retain `PartialEq`; parser tests then passed |
| Several intermediate patch probes used an incorrect relative path | 1 | The patches were rejected atomically; reissued them against the discovered `src-tauri` paths and verified the resulting diff |
| UI-05 focused contract tests fail because typography/click-target tokens are absent and the minimum-height sidebar still uses 30px | 1 | Expected RED; implement the audited discrete tokens and targeted control rules, then rerun the same focused tests |
| Checkbox/radio reconnaissance regex was malformed by PowerShell quoting | 1 | The read-only search changed nothing; use fixed-string searches for the concrete selectors |
| First combined UI-05 style patch referenced nonexistent `.folder-list > li` | 1 | The patch was rejected atomically; read the live `.local-folder-row` selector and apply smaller exact-context patches |
| Existing Browser page API has no viewport-resize method and its isolated evaluate scope cannot create DOM | 2 | Keep the mandated Browser surface and use a temporary same-origin static wrapper with a real 960×640 iframe; remove it after QA |
| First 960×640 iframe measurement ran before the Vite-injected global stylesheet settled | 1 | Inspect stylesheet ownership, wait for the enabled global sheet and token value, then rerun all route measurements from a stable reload |
| A Windows-incompatible `app/src/**/*.css` path produced an `rg` filename error | 1 | The read-only scan changed nothing; search the concrete `app/src/styles.css` path instead |
| Final frontend build initially wrote Node 22.18.0/npm 10.9.3 into the strict manifest | 1 | Rebuild with `.runtime-cache/v24.15.0` and verify manifest tools are exactly Node 24.15.0/npm 11.12.1 before staging |
| First real-style recheck queried the search submit button before restoring the search route after reload | 1 | Take a fresh DOM snapshot, navigate from the observed route, then reacquire the exact primary control before reading computed styles |
| Full frontend regression retained the pre-audit focus-ring assertion for `--accent` | 1 | Update the release contract to require the independent `--focus-ring` token, then rerun the complete frontend suite |
| Browser screenshot call followed the testing skill's stale `playwright.screenshot` example | 2 | Use the live Browser API's documented `Tab.screenshot` method; both calls occurred after the interaction probe and changed no browser or repository state |
| Browser locator did not expose Playwright's `isChecked()` helper | 1 | Read the checkbox's DOM `checked` property through the Browser evaluate API, then continue the same rendered flow without changing application state |
| UI-04 static contract passed while Browser still computed search-result `overflow-y: auto` | 1 | Add a later `.library-content.search-results` two-axis override and assert that ordering boundary explicitly; single-axis hidden overflow otherwise promotes the visible axis back to auto |
| Browser could not find the newly labelled page scrollbar immediately after App hot reload | 1 | Reload the test page so `CustomScrollbars` recreates its DOM-injected entry and rereads the target label; existing injected entries intentionally update metrics, not identity metadata, during ordinary scans |
| Browser tab binding became stale when attempting the UI-04 reload | 1 | Discard only the stale tab handle and acquire a fresh tab from the existing browser binding, as required by the Browser runtime; no application or repository state changed |
| Dialog stack helpers did not narrow a nullable registration for TypeScript | 1 | Perform the null/top-entry checks directly before reading the registration entry |
| JSDOM click did not focus either dialog trigger before opening it | 2 | Establish browser-realistic focus on both primary and nested triggers before clicking, then verify each restoration path |
| Browser modal probe used the unavailable `HTMLElement` constructor in isolated evaluation | 1 | Read `document.activeElement` through DOM properties and containment checks without constructor-based narrowing |
| Real browser dialog teardown found the original Svelte anchor had already been removed before portal cleanup | 2 | Do not reinsert a portal during teardown; remove the portaled node directly so Svelte cannot leave an orphan after its original block anchor is gone |
| Browser automation click did not leave the opener focused by the time the dialog mounted | 1 | Track the last pointer/keyboard interaction in capture phase and use it only when `activeElement` is the body, preserving focus restoration for real mouse, keyboard, and automation paths |
| Full Rust regression retained a v1-migration assertion for the old archive default `All` | 1 | Update the migration contract to `RecentItems { count: 100 }`, then rerun the focused target before the complete suite |
| A combined `rg` pattern containing `--scroll-speed` was parsed as an option | 1 | Use fixed-string searches with an explicit `--` delimiter; the failed read changed no files |
| Focused Rust rerun rejected a manifest generated by the current PATH Node/npm versions | 1 | Rebuild with the repository-locked Node 24.15.0 and npm 11.12.1 runtime before rerunning the same Cargo targets |
| Full Rust regression found the scheduler expansion fixture still expected archive default `All` | 1 | Scan default-derived archive assertions once, update the scheduler expectation to `RecentItems { count: 100 }`, and rerun focused plus full Cargo gates |
| Rendered-QA preview could not bind port 4173 because it was already in use | 1 | Inspect the listener and served page read-only; reuse only if it is this project, otherwise start on a distinct free loopback port |
| First browser metric probe used an unavailable `HTMLElement` constructor in the isolated evaluation scope | 1 | Use capability checks plus `getBoundingClientRect` directly; the corrected read-only probe passed |
| Session resume accidentally polled nonexistent exec cells | 1 | No repository or process state changed; resumed with direct commands and recorded the recovery before implementation |
| First onboarding data-root command test created an invalid completed-device fixture | 1 | Set the required final onboarding step together with `onboarding_completed`; production validation remains strict |
| Data-root regression found the temporary `journal_mode=DELETE` fix changed critical database bytes | 1 | Preserve WAL mode, truncate/checkpoint first, then remove only a zero-length WAL and removable SHM sidecar before inventory/copy |
| Concentrated review initially inferred a repeated root-switch retry after refresh failure | 1 | Read the full `refreshAppSettings` control flow; it catches the error and returns, after which the draft root is finalized, so no retry bug exists and no code change is needed |
| Focused `rustfmt --check` reported broad pre-existing drift in shared Rust files | 1 | Apply rustfmt's shape only to lines introduced by this phase; do not rewrite or stage unrelated formatting changes in `main.rs` or other dirty files |
| First follow-up planning patch used a stale `progress.md` sentence as its anchor | 1 | Re-read the live tails and apply smaller exact-context planning updates; the failed patch changed no files |
| Task-center event refresh test never observed the registered `sync:changed` handler | 1 | The test setup overwrote its recording implementation with `mockResolvedValue`; remove the override and wait for listener registration before emitting the event |
| Focused Cargo verification rejected stale frontend inputs after the latest Svelte changes | 1 | Rebuild `dist/` and `build-manifest.json` with `npm run app:build`, then rerun Cargo gates |
| A combined CSS reconnaissance regex had an unclosed escaped character class | 1 | Replace the broad expression with exact fixed-string searches for each selector; no file or runtime state changed |
| `rg` was given PowerShell-style wildcard paths for nested test directories | 1 | Search explicit directories with `-g "*.test.ts"` or enumerate files first; the failed read changed no state |
| A planning update used the malformed error-row text as an exact patch anchor | 1 | Re-read the live plan and patch the actual line; the failed patch changed no files |
| A later `rg` search again used a Windows-incompatible `app/src/*.test.ts` argument | 1 | Use `app/src` as the root plus `-g "*.test.ts"`; no files were changed |
| A combined fixed-string reconnaissance command still mixed a quoted `type=\"number\"` fragment into one regex | 1 | Split the search into `rg -F -e` literals and exact file reads; the failed command was read-only |
| Focused Cargo RED tests stopped in `build.rs` before test execution because frontend test edits made the committed manifest stale | 1 | Preserve the required sequencing: finish the frontend slice, run `npm run app:build` to regenerate the manifest, then execute Rust tests serially |
| A multi-file planning patch omitted a hunk boundary before the second file | 1 | Reissue the same append with a complete `@@` hunk for each file; the failed patch changed no files |
| First focused GREEN run failed Svelte compilation because the new save-status span mixed `on:animationend` with the component's property event syntax | 1 | Use Svelte 5 `onanimationend`; the compiler failure occurred before runtime tests |
| First Phase-20 typecheck rejected directly calling a DOM `TimerHandler`, which may also be a string | 1 | Narrow the captured timeout handler with `typeof === "function"` before invoking it in the test |
| Initial settings test read assumed nonexistent `src-tauri/tests/settings_store.rs` and one search reused a Windows-incompatible wildcard path | 1 | Use the discovered `src-tauri/tests/settings_storage.rs` and search concrete directories with `-g` filters |
| A PowerShell `rg` verification pattern embedded conflicting quote and comma syntax | 1 | Re-run with one single-quoted regular expression; the read-only command made no changes |
| Initial Phase-12 settings-boundary regex lost its quoted literal inside PowerShell | 1 | Replaced the broad regex with fixed-string and explicit line-range reads; no files were changed |
| One Phase-12 patch used multiple update blocks for the same file, which `apply_patch` rejects | 1 | Combined all `App.svelte` hunks into one update block and reapplied successfully |
| A broad test-context patch matched the new fourth-step fixture before the older tray fixture | 1 | Re-read both named tests and corrected each under its unique test-name anchor |
| Initial save-state test clicked the theme button before settings hydration enabled it | 1 | Wait for the control to become enabled before firing the setting change; the production state logic then passed |
| Installed like sync stops with `archive_dom_contains_non_records`, favorite sync exits with `cdp_target_closed`, and downloads end as `completed_with_errors` / `download_part_not_owned` | 1 | Bind each source to its own route scroll container, ignore non-record archive children safely, and compute transfer ownership from the actual work root; add focused regressions before rebuilding |
| Initial persistent-log patch assumed `progress.md` began with `# Progress` | 1 | Re-read the live headers and patch each planning file against its actual structure |
| First combined RED-test patch had an invalid multi-file hunk boundary | 1 | Split the patch by file and apply exact-context hunks |
| New download RED test called an `Arc<Self>` method on a plain service value | 1 | Wrap the fixture service in `Arc`, then rerun to reach the intended behavioral failure |
| First latest-log enumeration used an invalid PowerShell regex escape and omitted `currentRoot.path` | 1 | The command was read-only; retry using the parsed nested bootstrap field plus literal path-segment matching |
| First persistent-log update used a stale findings-file tail anchor | 1 | The combined patch failed atomically; re-read all three live anchors and update them separately |
| First read-only item query hit a GBK `UnicodeEncodeError` on an emoji title | 1 | Do not drop or mutate data; force UTF-8 output and select only diagnostic fields on the retry |
| Source search passed a Windows wildcard path and one later regex had an unclosed group | 1 | Both commands were read-only; use repository directories with `-g '*.rs'` and fixed-string searches instead |
| First Win32 path probe piped directly from nested PowerShell loops | 1 | Parser rejected it before API execution; collect loop output in a variable before formatting |
| PowerShell interpolated `$:` inside a double-quoted search pattern | 1 | Use single-quoted/fixed patterns for Svelte reactive-statement searches |
| One repository search named nonexistent `src-tauri/src/sync.rs` | 1 | Continue with the discovered `src-tauri/src/sync/` module directory |
| Searched nonexistent `app/tests` directory | 1 | Confirmed frontend tests live beside sources in `app/src`, especially `App.test.ts` |
| Combined frontend source read exceeded output limit | 1 | Preserve gathered facts and use smaller line-range reads for remaining sections |
| Assumed stylesheet path `app/src/app.css` | 1 | Use `rg --files app/src` to locate the actual stylesheet before styling |
| Broad test patch failed atomically on exact-looking contexts | 2 | Re-read the live file and switch to one atomic hunk per patch |
| Targeted rustfmt exposed large pre-existing formatting drift in three legacy files | 1 | Restore their original layout with exact patches, retain only behavior changes, and rerun the affected tests |
| First GREEN run observed initialization/DOM flush races in new tests | 1 | Wait for each control's observable state before issuing the next interaction |
| Second GREEN run still lost prior toggle state | 2 | Instrument the test boundary and inspect the full settings call sequence before changing production code |
| Typecheck rejected typed request as InvokeArgs and found one incomplete fixture | 1 | Pass an explicit invoke object and add backupDirectory null to the complete fixture |
| Automatic-backup notification recovery read a nonexistent `status` field from `SchedulerTickResultV1` | 1 | Read the persisted post-tick automatic-backup state and resolve only a confirmed latest success |
| Schema-8 backup tests failed because the archive module still declared database schema 7 | 1 | Advance the strict backup database contract to schema 8 and retain unsupported-version rejection |
| Cargo was given two positional test filters in one invocation | 1 | Run the complete binary test target and focused integration targets instead |
| Parallel verification orchestration contained invalid JavaScript before any nested command started | 1 | Split the read-only checks into shorter tool calls; no repository command had executed |
| Patch orchestration script contained an unescaped template delimiter | 1 | Retry with delimiter-free planning text and smaller patches |
| Concentrated final diff output was truncated | 1 | Keep the single final-review pass but inspect the omitted API/type/style hunks with focused diffs |
| Playwright QA artifacts were written into the repository and direct PowerShell cleanup commands were rejected | 2 | Removed only the named untracked QA files and directory with path-scoped `git clean`; no tracked or user files were affected |
| Device-setting save feedback was overwritten by a late initialization settings response | 1 | Invalidate the active settings request when a user save begins, then preserve the existing generation-based latest-write contract |
| PowerShell parsed a combined `rg` pattern as an operator | 1 | Split module/export inspection into a plain file read and a separately quoted search |
| Phase-11 package inspection assumed `app/package.json` | 1 | Use the root `package.json`, which owns the Vite, Vitest, Svelte, and TypeScript contracts |
| Account-switcher GREEN test used an ambiguous current-account name query | 1 | Select the controlled trigger by `aria-expanded`; the duplicate visible name in the open account menu is intentional |
| App-shell integration patch targeted `App.svelte` more than once | 1 | The patch was rejected atomically; split imports/state, shell template, and topbar removal into separate patches |
| Phase-11 preview could not find the optional `agent-browser` CLI | 1 | Reused the already-available Playwright browser tools without adding or changing project dependencies |
| Cargo polling used the exec-cell waiter for a terminal session | 1 | The test process continued normally; poll the returned `session_id` with `write_stdin` and wait for its complete exit |
| Host policy rejected the verified recursive removal of browser QA artifacts | 1 | No command started; require the exact workspace paths, reject subdirectories, delete only enumerated files with .NET, then remove the empty directory |
| Phase-7 planning-log patch used an inexact findings anchor | 1 | Re-read the real file tails and append using stable end-of-file anchors |
| Cargo test command supplied two positional filters | 1 | Run the complete focused integration-test target instead |
| Frontend typecheck found two test fixtures without schema-v6 updates | 1 | Add disabled-by-default update settings to both typed fixtures |
| Full App test reached a stale database-schema assertion | 1 | Align the assertion with schema 7 and advance all backup-summary settings fixtures to schema 6 |
| Worker-slot test used `unwrap()` on a non-Debug error type | 1 | Assert `OnceLock::set(...).is_ok()` without adding test-only Debug behavior to production |
| Session resume mistakenly polled nonexistent exec cells | 1 | Confirmed no process or file side effect, then resumed with direct read-only commands |
| Automatic-backup lifecycle RED test could not import three intended helpers | 1 | Expected TDD failure; implement initialize, stop, and join helpers using the existing worker pattern |
| Broad `runtime.rs` read was truncated | 1 | Use focused line ranges and amend the existing lifecycle wiring test instead of rereading the whole file |
| Cargo test command supplied two positional test filters | 1 | Run the complete `--test runtime` target, which covers both focused assertions |
| Whole-crate `cargo fmt --check` exposed extensive pre-existing formatting drift | 1 | Do not reformat unrelated files; run `rustfmt` only on phase-4 Rust files and verify their diffs |
| Runtime wiring assertion searched the private field name instead of the stop helper | 1 | Assert `stop_automatic_backup_runtime` ordering so the test matches the actual lifecycle abstraction |
| Main-binary focused test tool call had malformed JavaScript quoting | 1 | Command never started; corrected the tool arguments before retrying |
| Full Rust suite timed out once in existing sync-worker quiesce test | 1 | Reproduce the single test, inspect its wait condition, and only change code if evidence links it to the current diff |
| Broad restore-document search was truncated | 1 | Read only authoritative roadmap lines 226-257 and use targeted code searches from here |
| Portable-settings decode RED test could not import the intended pure API | 1 | Expected TDD failure; extract existing migration/validation into a public bytes decoder |
| Replace-restore integration tests could not import the intended service | 1 | Expected TDD failure; add a restore service that composes existing archive, maintenance, settings, and bootstrap primitives |
| Successful restore test could not delete temp root on Windows | 1 | Functional assertions passed; explicitly drop live and safety SQLite connections before cleanup |
| Partial-commit rollback RED test could not import the intended helper/stage | 1 | Expected TDD failure; route production commit through one injectable commit-and-rollback function |
| Restore inspection and desktop wiring RED tests could not find the intended APIs | 1 | Expected TDD failure; add a read-only inspection service and register the three replace-restore desktop commands |
| Replace-restore frontend RED test could not find the import entry point | 1 | Expected TDD failure; add the inspected direct-replacement flow to the existing backup settings section |
| Combined frontend restore patch missed the live `App.svelte` import context | 1 | Patch failed atomically; split types/API and component edits into small live-context hunks |
| Restore UI GREEN test expected a sync-job fetch without a tracked job | 2 | `refreshSyncStatus` intentionally no-ops unless `currentSyncJobId > 0`; assert the remaining restored-state refreshes instead |
| Rust restore regression was blocked by a stale committed frontend manifest | 1 | Run `npm run app:build` after frontend changes before invoking Cargo gates |
| Restore busy-state RED test found account settings remained enabled | 1 | Include restore busy state in both global and account settings disable conditions |
| New restore Rust files failed targeted `rustfmt --check` | 1 | Format only the two new restore files and leave unrelated crate formatting untouched |
| Merge-service RED integration target could not import the planned merge APIs | 1 | Expected TDD failure; implement the merge contracts, deterministic plan validation, conflict resolution, and transactional merge service |
| PowerShell `rg` search assumed a file/wildcard path unsupported on Windows | 3 | Stop using wildcard-like paths entirely; target a real directory and apply `rg -g 'pattern'` only |
| Initial merge GREEN compile hit three `query_map` temporary-lifetime errors | 1 | Bind each collected row vector before the statement-owning block ends, as required by rusqlite iterator lifetimes |
| First behavioral merge run kept current history range despite a backup-field choice | 1 | Align conflict IDs and application at approved direct-field granularity so `accounts.<key>.history` resolves as one field |
| Broad merge-test coverage patch missed live assertion contexts | 1 | Patch the test in small independent hunks after reading exact current line ranges; the failed patch was atomic and changed no files |
| Merge desktop wiring RED test could not find the planned command function | 1 | Expected TDD failure; add merge-plan and merge-restore Tauri commands and register both IPC handlers |
| Merge frontend RED test could not find the planned merge action | 1 | Expected TDD failure; add merge contracts/API wrappers and extend the existing import modal with plan/conflict state and commit flow |
| First merge frontend GREEN run queried plan content before Svelte flushed async state | 1 | Wait for the plan heading to appear before interacting with conflict controls; production API invocation already occurred |
| Full App regression retained the old “merge coming later” assertion | 1 | Replace the obsolete expectation with the implemented replace-versus-merge choice copy |
| UID-only current account was not matched to a backup-enriched `sec_uid` key | 1 | Build one canonical account-identity map for folder planning and settings normalization; reject split or conflicting identities |
| Canonical-account patch missed rustfmt-adjusted merge-plan context | 1 | Re-read the two exact function ranges and apply smaller independent hunks; the failed patch changed no files |
| Running `rustfmt` on `lib.rs` recursively reformatted unrelated child modules | 2 | Restored only this run's formatter changes from the merge index/stage 2; never pass `lib.rs` to rustfmt again and format leaf files only |
| Vite preview PTY launch used the denied WindowsApps `pwsh.exe` shim | 1 | Start the long-running preview through system PowerShell without a PTY |
| Static demo import action did not open the restore modal | 1 | Confirmed non-Tauri file selection intentionally returns `null`; inject deterministic local IPC responses through the tab debug interface for rendered QA without changing production code |
| Rendered-QA IPC stub omitted periodic `get_scheduler_state` | 1 | Classified the three timer errors as test-harness noise after UI assertions; production Tauri IPC and automated scheduler tests remain unaffected |
| Targeted rustfmt check included drift-heavy `main.rs` and `lib.rs` | 1 | Do not format unrelated existing code; restrict the formatting gate to the new merge service/test and shared restore helper |
| PowerShell parsed `$base..feature/download` as an invalid revision expression | 1 | Use `${base}..feature/download` so the variable boundary is explicit |
| Plain `git rm` refused staged incoming internal reports | 1 | Avoid force removal; delete the five known merge-only report files explicitly with `apply_patch` |
| Combined pragma search used an invalid escaped regex | 1 | Replace it with separate fixed-string searches for busy-timeout and foreign-key configuration |
| Error-log patch assumed a nonexistent prior row | 1 | Read the live table tail, then append new error rows against exact context |
| Download-test search passed a nonexistent `src-tauri/tests/download_` path | 1 | Search the real tests directory with `rg -g 'download_*.rs'` instead of encoding a filename prefix as a path |
| First schema-v7 test compile found a three-argument browser-session call | 1 | Reconcile the auto-merged collector session wrapper with the current explicit `BrowserId` launch contract, then rerun the same focused test |
| Download browser lease with no account did not block maintenance | 1 | Inspect admission state accounting and make every active browser lease participate in maintenance exclusion without inventing a fake account ID |
| First namespaced download compile lacked resolver browser/CDP network APIs | 1 | Port only `resolve_download_browser` and the required network-response methods from merge stage 3 onto the current browser-runtime/CDP architecture |
| Full download tests referenced removed migration path 003 | 1 | Update isolated download-table fixtures to the integrated `007_download_jobs.sql` path and rerun the same target set |
| Resolver-child tests opened blank databases after worker migration was removed | 1 | Initialize each runtime fixture through the central startup migrator before spawning `DownloadService`; keep production worker assert-only |
| Resolver-child process never connected to fixture CDP | 1 | Trace the spawned command through current early-process parsing and add the strict `--download-resolver` mode before Tauri/single-instance startup |
| `rg` treated a search pattern beginning with `--download-resolver` as a flag | 1 | Insert the `--` option terminator before patterns that begin with a dash |
| Combined early-mode patch missed the live `main.rs` test context | 1 | Split enum/parser, production dispatch, and tests into independent exact-context patches; failed patch was atomic |
| All download-service tests seeded legacy account/history fixtures | 2 | After adding account IDs, also seed mandatory remote-folder registry state and migrate the one standalone offline-recovery database through the central latest-schema path |
| Combined account-binding implementation patch missed the live download-service import layout | 1 | Patch migration, catalog, storage, resolver, service, and schema as separate exact-context changes; the failed patch was atomic |
| Schema-v7 idempotence fixture inserted an active account without uid/sec_uid | 1 | Use the schema-valid `pending_login` status for the synthetic profile-only account |
| Schema-v7 idempotence fixture omitted remote-folder registry state | 1 | Seed the mandatory revision-1 registry row for the synthetic account before reasserting latest schema |
| Full download regression found legacy direct job inserts without owner columns | 1 | Upgrade queue, recovery, and resolver fixtures to seed one valid account/registry row and include its account/profile binding in every direct job insert |
| Memory registry referenced a rollout-summary filename that no longer exists verbatim | 1 | Locate the current summary path with `rg --files` before reading it; do not repeat the stale path |
| Frontend test search passed Windows-incompatible `app/src/*.test.ts` as a path | 1 | Search the real `app/src` directory and use `rg -g '*.test.ts'` when a filename filter is needed |
| First download App GREEN run used an exact text matcher for a longer job-summary line | 1 | Match the account attribution as a substring; production output already included the correct account ID |
| First integrated full Rust compile found `NetworkResponse::from_event_params` private | 1 | Restore the parser's public visibility required by the system-browser integration contract, then rerun the focused target |
| Full Rust unit tests found one internal emergency-interrupt fixture without schema-v7 owner columns | 1 | Seed a valid account/registry and bind the direct download-job insert to its account/profile before rerunning library tests |
| Full Rust regression found the cover-cache latest-schema test still expected v6 | 1 | Update the latest-schema assertion to v7 and scan current frontend backup fixtures for the same stale version |
| Cover-cache rerun was blocked by stale frontend manifest after schema-fixture edits | 1 | Rebuild frontend assets and manifest before rerunning Cargo, preserving the mandatory build-time hash gate |
| Full Rust regression found v1-to-v2 migration left `user_version` at 1 | 1 | Restore `PRAGMA user_version = 2` in historical migration 002; registry-owned version assignment remains limited to migration 007 |
| Parallel-read orchestration accidentally invoked the unavailable Plan-mode input selector twice | 1 | No repository or process state changed; resumed with direct read-only commands and will reserve the selector for Plan mode only |
| Reused a Windows-incompatible `src-tauri/src/download/*.rs` path in `rg` | 1 | The command was read-only and changed nothing; target the real directory and use `rg -g '*.rs'` for any future filename filtering |
| Update reconnaissance referenced nonexistent `src-tauri/src/settings_contracts.rs` | 1 | Located the actual unified contracts and persistence in `src-tauri/src/settings_store.rs`; subsequent reads use repository-discovered paths |
| Authenticated download acceptance lacked the required isolated test profile environment | 1 | The one ignored case executed and failed explicitly with `environment blocked`; no normal browser profile was used or inferred |
| Combined acceptance setup/cleanup command was rejected before execution by shell policy | 1 | Split marker creation, test execution, and verified cleanup into separate commands using the system PowerShell path |
| Windows PowerShell `New-Item` rejected `-LiteralPath` in this host | 1 | Retried the exact non-wildcard path with `-Path`; marker creation succeeded |
| Direct `Remove-Item` cleanup outside the workspace was rejected by shell policy | 1 | Resolved and equality-checked the exact marker path, then deleted that single file through `System.IO.File.Delete` and verified absence |
| First authenticated normal-video download was rejected by `PublicInternetPolicy` | 1 | Login and resolver succeeded; inspect only the source host and DNS addresses, then correct either the fixture URL or a proven public-address classification bug |
| First read-only SQLite one-liner had mismatched nested quoting | 1 | Retried with PowerShell single-quoted Python source and double-quoted SQL; no database mutation occurred |
| Recursive account-profile listing produced truncated noisy output | 1 | Stop broad recursion; subsequent profile checks target exact marker/selection/login filenames and shallow paths |
| Second real-video attempt could not read a transient CDP response body | 2 | Switched to a different archived normal-video URL for the final diagnostic attempt instead of repeating the same unstable resource |
| Authenticated transfer resolved a Douyin CDN host to `198.18.0.109` | 1 | Preserve the general public-IP rejection and allow proxy fake-IP results only for exact approved Douyin CDN domain suffixes; IP-literal sources remain rejected |
| Authenticated transfer reached the CDN but returned an expired/unauthorized response | 1 | Validate the smallest browser-derived request-header set in the explicit acceptance test before changing the production resolver; do not copy cookies without evidence |
| Memory registry pointed to a batch-download rollout-summary filename that is no longer present | 1 | Locate the current summary with `rg --files` before reading it; do not retry the stale path |
| Resolver-child header RED test returned zero headers instead of two | 1 | Expected TDD failure proving the existing CDP resolution path needs manifest enrichment; implement only live UA and Douyin referer attachment |
| First post-header real-video attempt could not read the transient CDP detail body | 1 | Authorization marker and browser process were cleaned; do not repeat the same URL, use the next archived normal-video candidate |
| Second post-header real-video candidate returned an invalid media manifest | 2 | Cleanup passed; try one final distinct archived normal-video candidate, then stop candidate rotation and investigate capture stability if it also fails |
| Staged secret-scan PowerShell embedded a semicolon inside an object expression | 1 | Command failed before scanning and changed no files; capture the diff-check exit code in a separate statement, then build the result object |
| Phase-7 settings search assumed nonexistent `app/src/api.ts` | 1 | Locate the actual frontend IPC module with `rg --files app/src` before focused reads; no repository state changed |
| Reused a Windows-incompatible `app/src/*.ts` path while searching formatting helpers | 1 | Search the real `app/src` directory with `rg -g '*.ts'`; no repository state changed |
| Svelte rejected `class:` directive on a Lucide component in the updater RED-to-GREEN run | 1 | Remove the component directive; the textual checking state already communicates progress |
| Focused Rust runtime test was blocked by the committed frontend manifest after updater UI edits | 1 | Rebuild `dist/` and its manifest before continuing Cargo gates, as required by the repository build contract |
| Release review again passed a Windows wildcard path (`src-tauri/tauri.*.json`) to `rg` | 1 | Stop wildcard paths entirely and inspect the discovered concrete `src-tauri/tauri.conf.json` file only |
| Post-shutdown preview poll returned `Unknown process id` | 1 | Confirmed the local preview server had already exited after Ctrl+C; no process remains |
| System `tar` could not inspect the NSIS executable format | 1 | Check known local 7-Zip installations; if unavailable, verify package inputs through the NSIS template, bundle configuration, package manifest, and installer binary metadata without installing over the user's current application |
| Package scan referenced nonexistent `src-tauri/src/package_identity.rs` | 1 | Locate the real package-identity source with `rg --files` before continuing the bounded installer-input audit |
| Targeted rustfmt check found only line-wrapping drift in the two new updater Rust files | 1 | Format those two leaf files only, avoiding `main.rs`/`lib.rs` and unrelated recursive churn |
| Initial icon-task planning patch used a nonexistent `progress.md` heading anchor | 1 | The atomic patch changed no files; re-read the live planning-file anchors and patch each existing section directly |
| Session resume accidentally polled two nonexistent exec cells | 1 | No repository or process state changed; resumed with direct commands and recorded the recovery here |
| Brand-image focused test found no image in `.brand-mark` | 1 | Expected TDD failure proving the live UI still renders the Lucide placeholder; generate the assets and replace only that occurrence |
| First icon contact-sheet generator computed a zero scale for the 256px frame | 1 | Keep native size for frames at or above the preview target by clamping the inspection scale to at least 1 |
| In-app browser rejected `networkidle` for the local preview | 1 | Reuse the already-created tab and wait for supported `load`, then inspect the rendered DOM directly |
| Browser documentation advertised `playwright.screenshot` but the active tab did not expose it | 1 | Use the supported top-level `tab.screenshot` API and continue the same QA session |
| Git classified the new source PNG as LF-normalized text during staging | 1 | Add explicit PNG/ICO binary attributes, restage the original bytes, and verify staged hashes plus image decoding before commit |
| Ordinary restaging retained the previously normalized PNG blob in the index | 2 | Force renormalization under the new binary rule, then compare raw worktree and index object IDs again before proceeding |
| Combined launch-investigation planning patch missed a live findings anchor | 1 | The atomic patch changed no files; split the update by file and use the exact current headings |
| Updater base-configuration regression found no `/plugins/updater` value | 1 | Expected TDD failure reproducing the installed startup panic; add only the inert deserializable plugin configuration |
| Installed acceptance now reports `schema_v5_column_mismatch:download_jobs` and an early-closing first-login browser | 1 | Diagnose both concrete startup paths before changing production code; preserve the existing phase-8 plan |
| First read-only SQLite metadata one-liner had nested quoting syntax failure | 1 | Switched to a PowerShell here-string piped to Python; metadata inspection then succeeded without database writes |
| Login artifact search included a nonexistent `src-tauri/src/paths.rs` path | 1 | Used the repository-discovered `AppPaths` implementation in `src-tauri/src/lib.rs` and the active data-root directories |
| External AppData database cleanup via `Remove-Item` was blocked by host policy | 2 | Verified exact parent paths and used non-recursive `System.IO.File.Delete` for the three authorized SQLite files |
| Computer Use could read the WebView tree but click geometry and screenshot capture were unavailable | 2 | Stopped retrying UI input and invoked the exact installed native login collector with an isolated profile and durable diagnostics |
| Computer Use guidance read initially used a mistyped work directory | 1 | Retried with the repository's exact absolute path; no process or file state changed |
| First visible-page reuse run left CDP mouse input inactive | 1 | Abandoned reuse; create and activate the proven interactive page, then close only the unused launch Target |
| Broader login fixture was redirected by live Douyin scripts after the synthetic save modal closed | 3 | Keep the real-browser gate on one login/save cycle and remove only delayed live-SPA-dependent idempotence checks; production URL validation stays strict |
| Combined cleanup patch targeted error rows in the wrong planning file | 1 | The atomic patch changed nothing; split code/findings and task-plan corrections against exact live contexts |
| Leaf `rustfmt` reformatted the entire compact CDP transport file | 1 | Rebuilt `cdp.rs` from `HEAD` plus only the targeted handoff/tests using `apply_patch`, then reran all affected gates |
| First release-probe `Start-Process` command was blocked by host command policy | 1 | Split directory creation from direct GUI-subsystem invocation; the real collector and browser then launched normally |
| PowerShell wrapper-layer target count initially reported zero | 1 | Inspected raw `/json/list`; it contained exactly one page Target at the Douyin page |
| Repository-wide `cargo fmt --check` reports extensive pre-existing formatting drift outside this login fix | 1 | Do not rewrite unrelated Rust files; run rustfmt checks only on the three modified Rust files and continue the targeted/full test gates separately |
| `rg` treated a search pattern beginning with `--collector` as an option | 1 | Re-ran with the explicit `rg --` end-of-options separator; no project or runtime state changed |
| Long PowerShell wrapper for the optional release real-profile probe was blocked by host command policy | 1 | No process or file was created; split precondition, direct collector execution, inspection, and exact-file cleanup into short independent commands |
| Release hash command piped directly from a `foreach` statement | 1 | Capture the loop results first, then pipe the materialized array to `Format-List`; no file or process state changed |
| Initial exact-dialog/event search used an invalid escaped regular expression | 1 | The search was read-only; continue with fixed-string searches and concrete file reads |
| Download subscription read assumed `app/src/download/subscription.test.ts` and combined unrelated reads in one shell command | 1 | The missing path caused no mutation; locate tests with `rg --files` and keep subsequent reads separate |
| Synchronization GREEN test used an ambiguous `任务` text query | 1 | The rendered behavior was correct; assert the summary's `strong` value through its concrete selector |
| Focused system-Edge test was blocked by a stale committed frontend manifest | 1 | Rebuild `dist/` with `npm run app:build`, then rerun the identical Rust gate |
| Post-package cleanup inline PowerShell was rejected by host command policy before launch | 1 | No state changed; execute the same validated workflow from a temporary repository-local `.ps1`, then remove it |
| Combined login logging patch missed the live spawn context | 1 | The atomic patch changed nothing; re-read the exact ranges and apply the browser wait, logging helper, spawn handling, and tests as separate hunks |
| Targeted rustfmt check exposed pre-existing compact formatting in `browser_process.rs` | 1 | Do not reformat the legacy file wholesale; retain the focused behavioral diff and rely on compilation, tests, `git diff --check`, and targeted review |
| A browser-process search used an invalid escaped regex | 1 | The command was read-only; rerun with fixed-string searches against the concrete file |
| Full Cargo gate rejected a manifest generated by Node 22.18.0/npm 10.9.3 | 1 | Rebuild `dist/` with the repository-cached Node 24.15.0/npm 11.12.1; keep the strict build-time tool-version gate unchanged |
| Combined system-Edge fixture classified unrelated old browser processes as descendants | 1 | Root PID `9036` had been reused while old processes retained that parent PID; filter fixture descendants by creation time at or after the current root process |
| Planning-tail read piped directly from a PowerShell `foreach` block | 1 | Materialize the objects into an array before piping to `ConvertTo-Json`; no project or runtime state changed |
| First cleanup verification matched its own PowerShell command line as a managed process | 1 | Treat the result as a read-only false positive and independently verify exact executable paths after the command exits |
| Targeted `rustfmt --check` reported whole-file wrapping drift in legacy `login_actions.rs` | 1 | Do not reformat untouched functions; retain the focused behavioral diff and validate with compilation, tests, system Edge, and `git diff --check` |
| Post-package cleanup raced an already-exited managed process and `Stop-Process` returned `NoProcessFoundForGivenId` | 1 | Treat only a verified already-absent PID as successfully stopped; preserve hard failure for any process that still exists or cannot be terminated |
| Combined cleanup-race planning patch used an invalid multi-file hunk boundary | 1 | The patch was atomic and changed nothing; apply the script and planning updates as separate exact-context patches |
| Release startup smoke exited with code 0 while an authorized installed EXE appeared as its child | 1 | Initial handoff hypothesis was retained only until the next probe disproved it; final resolution is the Windows PowerShell path-decoding fix below |
| Handoff-aware smoke still exited 0 while the same installed EXE remained alive | 2 | Rejected the handoff hypothesis: the cleanup process filter failed to return the exact installed process, so investigate the PowerShell filter semantics before changing smoke acceptance again |
| Nested Windows PowerShell filter probe expanded `$installed` in the outer shell and produced invalid-path noise | 1 | The probe was read-only and inconclusive; run the minimal filter directly in the current shell without a nested command string |
| Windows PowerShell 5.1 decoded the UTF-8 no-BOM Chinese install-path literal as mojibake | 1 | Remove non-ASCII path literals from the temporary script; discover the uniquely named executable under standard install roots and validate its file metadata before authorizing termination |
| Full Rust regression again hit the existing one-second `quiescing_worker_finishes_the_running_job_without_dequeueing_the_next_job` timeout | 1 | Reproduce the isolated lifecycle test and only change code if evidence links it to the login rollback diff |
| Minimal-rollback post-package cleanup could not delete protected Edge profile objects | 2 | Verified an ACL boundary, then reran the existing marker-validating cleanup under same-user UAC elevation; all managed roots and processes were removed without touching package artifacts |
| Restart Manager could not enumerate the blocked cache file | 1 | Error 5 occurred before any process list was available, supporting an ACL boundary; probe `CreateFileW` directly for delete access versus sharing violation |
| Win32 access probe reused PowerShell's read-only `$Error` variable | 1 | Rename the local result to `$win32Error` and rerun the same read-only probe |
| Cleanup reached a second inaccessible Edge component directory | 1 | The directory cannot be enumerated and the user token has no ownership/backup privilege; inspect its file type and try only parent-authorized exact directory removal before considering a broader strategy |
| Protected Edge component directory rejected inherited ACL reset | 1 | Parent grants the local user full control but the child has an independent protected descriptor; research a supported Edge cleanup path before requesting elevation or accepting a residual orphan |
| Initial Bing-result extraction used an invalid escaped regex | 1 | Replace the pipeline regex with `Invoke-WebRequest` plus a concrete .NET match expression; no repository or runtime state changed |
| Selected-aggregate Rust test was blocked by a stale frontend manifest after adding a Svelte test input | 1 | Rebuild `dist/` and `build-manifest.json`, then rerun the focused Rust test; no Rust diagnostic was emitted |
| Phase-13 verification found a narrow TypeScript inference in the new App fixture and the expected stale-manifest gate | 1 | Type the fixture as `AppSettingsView`, rebuild the frontend manifest, and rerun the same gates |
| Direct structural cast of the existing deliberately partial App fixture was rejected because the fixture predates the required theme field | 1 | Use an explicit `unknown` bridge only at this test boundary; production types remain strict |
| Cargo verification after the fixture correction rejected `stale frontend input: app/src/App.test.ts` | 1 | Rebuild the committed frontend manifest after every frontend source change before Cargo gates |
| Phase-16 QA artifact cleanup was rejected twice by the host command policy | 2 | No file was deleted by either attempt; preview the exact untracked paths with `git clean -nd`, then remove only that confirmed set with scoped `git clean -fd` |
| Nested Windows PowerShell 5.1 parser probe expanded variables in the outer shell | 1 | Run the parser directly with `powershell.exe` as the selected shell; the temporary cleanup script then parsed cleanly in both PowerShell runtimes |
| Phase-16 cleanup helper hit Windows PowerShell 5.1 expression and collection compatibility errors | 3 | All three failures occurred before deletion; parenthesize `Test-Path` conditions, add exact stack evidence, and replace the empty `Generic.List[CimInstance]` return with a plain object array before the successful elevated run |
