# 统一设置与安全数据根 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Project rules prohibit implementation subagents.

**Goal:** 将散落在前端 `localStorage` 与固定 `app_data_dir` 中的状态迁移为 Rust 管理的统一设置，并允许用户在不产生双写数据库的前提下安全切换本地数据根目录。

**Architecture:** 系统 `app_data_dir` 只保存原子写入的设备 bootstrap，bootstrap 通过路径和不可变 `rootId` 指向带同一身份 marker 的数据根；数据根保存数据库、统一设置和全部可迁移数据目录。GUI 进程先取得 Windows 单实例 mutex，`AppState` 再以路径租约管理当前根；迁移取得进程内维护租约和 bootstrap 跨进程 mutex，等待所有前台及后台租约结束，执行严格 WAL checkpoint、no-follow 复制、浏览器状态重定位、语义校验和 bootstrap 原子切换，任何失败都继续使用旧路径。

**Tech Stack:** Rust 2021、Tauri 2、rusqlite/SQLite WAL、serde/serde_json、sha2、Windows `MoveFileExW`、rfd 原生目录选择器、Svelte 5、TypeScript、Vitest。

## Global Constraints

- 仅支持 Windows x64；正式运行和 `cargo tauri build` 不依赖 Node、npm、NCC、Playwright 或随包浏览器。
- 系统 `app_data_dir` 只保留 `bootstrap.json`、迁移恢复标记和原子写临时文件，不作为新的默认数据库根目录；bootstrap 缺失但旧数据尚未收纳时必须继续打开旧根，直到收纳与指针切换在同一提交中完成。
- 新默认数据根固定为 `<app_data_dir>\Data`；旧版直接位于 `app_data_dir` 的受管数据必须一次性安全收纳到该目录。
- 数据根固定包含 `history.sqlite`、`settings.json`、`covers/`、`profiles/`、`backups/`、`diagnostics/`、`logs/`、`tmp/` 和 `downloads/`。
- 用户选择安装目录根时自动规范化为 `<安装目录>\Data`；磁盘根、UNC/网络卷、当前根的父子重叠目录、非 Unicode 路径和非空未知目录一律拒绝。
- 权限探针必须真实执行目录创建、文件写入、读取、重命名、删除及 SQLite WAL/SHM 创建，不请求管理员权限，也不自动回退到其他目录。
- 目录迁移先阻止新任务，再等待现有同步、登录及其收尾 monitor 所持后台路径租约结束；复制完成后比较文件数、总字节数、关键文件语义 SHA-256 和浏览器登录状态，最后才原子更新 bootstrap。
- 迁移失败或进程中断时旧 bootstrap 继续指向旧根；新副本在 bootstrap 切换前不得被应用打开写入。
- 旧根默认保留。应用只能在用户明确确认后，按 bootstrap 中记录的精确路径、`rootId`、布局类型、allowlist 和 no-follow 安全检查删除旧根；旧版平铺布局永远不递归删除 `app_data_dir` 本身。
- 设备绝对路径、活动账号和后续开机自启/窗口状态不得写入可跨设备备份的 `settings.json`。
- 当前阶段的 portable settings V1 只迁移现有 UI 已生效字段；喜欢、收藏、浏览器选择、周期调度、托盘、缓存、备份、通知和更新通过后续 schema 扩展，不在 V1 写入未生效占位字段。
- GUI 在 collector 参数分流后、Tauri 初始化前持有稳定名称的 current-user Windows mutex；第二个 GUI 必须退出，collector 子进程不得被该 mutex 阻止。
- 每个标准数据根都有 `.emily-data-root.json`，包含 application ID 与随机 `rootId`；bootstrap 或 marker 缺失、损坏、路径不符时失败关闭，不创建空数据库、不回退默认目录、不覆盖现有 bootstrap。
- 浏览器选择和登录标记不得长期保存数据根绝对 profile 路径；迁移后隐藏同步必须沿用原登录态，无需重新登录。
- 正常迁移的 SQLite checkpoint 必须读取 `PRAGMA wal_checkpoint(TRUNCATE)` 三元组并断言 `busy == 0`；关闭最后连接后禁止复制 SHM，WAL 只能在不存在或零字节时继续。
- 安装目录 `Data` 的升级保留和显式卸载删除使用最小 NSIS hook 验证；安装包视觉资源仍留到后续安装包美化阶段。
- 每个生产行为遵循 RED -> GREEN -> 相关回归 -> 单一提交；不得修改或提交无关文件。

---

### Task 1: Bootstrap 与统一设置契约

**Files:**
- Create: `src-tauri/src/bootstrap.rs`
- Create: `src-tauri/src/settings_store.rs`
- Modify: `src-tauri/src/lib.rs`
- Keep unchanged until Task 5: `src-tauri/src/device_settings.rs`
- Test: `src-tauri/tests/settings_storage.rs`

**Interfaces:**
- Produces: `DataRootRefV1 { path, root_id }`、`RetainedDataRootV1 { root, layout, legacy_entries }` 和 `BootstrapV1 { schema_version, current_root, active_account_id, retained_data_roots }`，其中路径以绝对 UTF-8 Windows 路径序列化。
- Produces: `DataRootMarkerV1 { schema_version, application_id, root_id, created_at }`；标准数据根加载必须同时验证 marker 与 bootstrap `rootId`。
- Produces: `PortableSettingsV1 { schema_version, revision, legacy_frontend_import, global, account_defaults, accounts }`。
- Produces: `GlobalSettingsV1 { like_count_format, detail_fields }`。
- Produces: `AccountSettingsV1 { sync_mode, sync_days, hide_browser, scroll_speed }`，只包含当前已生效账号设置。
- Produces: portable account key `sec:<sec_uid>`，无 secUid 时使用 `uid:<uid>`；禁止用设备本地 `profile_key` 作为 `settings.json` 账号键。
- Produces: `LegacyImportStateV1 { completed, fingerprint }`；同一规范 payload 重试必须幂等，冲突 payload 在 completed 后不得覆盖。
- Produces: `read_bootstrap`, `write_bootstrap_atomic`, `update_active_account`, `load_portable_settings`, `update_global_settings`, `update_account_settings`, `import_legacy_frontend_settings`。
- Preserves until Task 5: 现有 `device_settings.rs` 和生产调用路径；本任务只增加新模块与测试，避免中间提交切换数据根或活动账号格式。

- [ ] **Step 1: 写入失败的 bootstrap 与 settings 严格反序列化测试**

  测试默认值、未知字段、错误 schema、相对路径、非绝对 data root、非法 rootId、非法 portable 账号键、同步天数边界、点赞格式和详情字段。关键断言：

  ```rust
  assert_eq!(BootstrapV1::new(root.clone(), root_id.clone()).current_root.path, root);
  assert!(parse_bootstrap(r#"{"schemaVersion":1,"currentRoot":{"path":"relative","rootId":"abc"},"activeAccountId":null,"retainedDataRoots":[]}"#).is_err());
  assert!(serde_json::from_str::<PortableSettingsV1>(
      r#"{"schemaVersion":1,"revision":0,"legacyFrontendImport":{"completed":false,"fingerprint":null},"global":{},"accountDefaults":{},"accounts":{},"extra":true}"#,
  ).is_err());
  assert_eq!(PortableSettingsV1::default().global.like_count_format, LikeCountFormat::Wan);
  assert_eq!(PortableSettingsV1::default().account_defaults.sync_days, 7);
  ```

- [ ] **Step 2: 运行测试并确认 RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test settings_storage contracts -- --nocapture`

  Expected: FAIL，因为两个新模块和契约尚不存在。

- [ ] **Step 3: 实现严格契约和同目录原子文件写入**

  所有 JSON 类型使用 `#[serde(rename_all = "camelCase", deny_unknown_fields)]`。原子写流程固定为 `create_new` 临时文件、`write_all`、`sync_all`、`MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`；失败时只删除本次临时文件。

  ```rust
  pub fn write_bootstrap_atomic(dir: &Path, value: &BootstrapV1) -> Result<(), SettingsError>;
  pub fn load_portable_settings(root: &Path) -> Result<PortableSettingsV1, SettingsError>;
  pub fn update_account_settings(
      root: &Path,
      portable_account_key: &str,
      value: AccountSettingsV1,
  ) -> Result<PortableSettingsV1, SettingsError>;
  ```

- [ ] **Step 4: 增加失败的 read-modify-write 与旧活动账号回归测试**

  验证修改活动账号保留 `currentRoot`、`rootId` 与 retained roots；portable account key 在两台数据库使用同一 uid/secUid 时一致；导入 fingerprint 重试幂等且无活动账号时只更新 account defaults。账号生产调用仍留到 Task 5 一次迁移。

- [ ] **Step 5: 实现 portable key、幂等导入和 marker 验证并运行 GREEN**

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test settings_storage
  ```

  Expected: PASS。

- [ ] **Step 6: 提交**

  ```powershell
  git add src-tauri/src/bootstrap.rs src-tauri/src/settings_store.rs src-tauri/src/lib.rs src-tauri/tests/settings_storage.rs
  git commit -m "feat: add unified settings storage"
  ```

### Task 2: 动态 AppPaths 与独占维护租约

**Files:**
- Create: `src-tauri/src/runtime_paths.rs`
- Create: `src-tauri/src/process_lock.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/tests/runtime_paths.rs`
- Test: `src-tauri/tests/runtime.rs`

**Interfaces:**
- Changes: `AppPaths` 明确区分 `bootstrap_dir`、`data_root` 和 `resource_dir`，并增加 `covers`、`backups`、`downloads`。
- Preserves: `AppPaths::new(data_root, resource_dir)` 供 collector 和既有测试使用，令 `bootstrap_dir == data_root`。
- Produces: `AppPaths::from_roots(bootstrap_dir, data_root, resource_dir)` 供 GUI 运行时使用。
- Produces: `RuntimePaths::new`, `acquire`, `begin_maintenance`，以及 RAII `PathLease`、`MaintenanceLease`。
- Invariant: `begin_maintenance` 设置维护标记后等待既有路径租约归零；维护期间新的 `acquire` 返回 `data_root_migration_in_progress`。
- Produces: `GuiInstanceGuard` 与 `BootstrapProcessGuard`，使用 current-user、应用 ID 和规范 bootstrap 路径派生的稳定 Windows named mutex。
- Invariant: `--collector` 和后续卸载辅助模式在 mutex 之前分流；只有 GUI 获取单实例 mutex。锁顺序固定为 GUI instance -> runtime maintenance -> bootstrap process mutex -> bootstrap RMW mutex -> settings mutex/SQLite，禁止反向获取。
- Compatibility: 在 Task 4 的 legacy adoption 与 bootstrap 初始化同时落地前，bootstrap 缺失时继续使用现有 `app_data_dir` 作为 legacy runtime root；本任务不得切到空的 `Data`。

- [ ] **Step 1: 写入失败的路径构造与并发租约测试**

  使用线程屏障验证维护租约等待现有读租约、阻止新读租约、abort 恢复旧路径、commit 后只返回新路径。

  ```rust
  let runtime = RuntimePaths::new(old_paths.clone());
  let lease = runtime.acquire().unwrap();
  let maintenance = begin_in_thread(runtime.clone());
  assert_eq!(runtime.acquire().unwrap_err(), "data_root_migration_in_progress");
  drop(lease);
  maintenance.join().unwrap().commit(new_paths.clone()).unwrap();
  assert_eq!(runtime.acquire().unwrap().data_root, new_paths.data_root);
  ```

- [ ] **Step 2: 运行并确认 RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test runtime_paths -- --nocapture`

  Expected: FAIL，因为运行时路径模块尚不存在。

- [ ] **Step 3: 实现租约计数、条件变量和路径快照**

  `PathLease` 持有克隆后的 `AppPaths` 和 `Arc<RuntimePaths>`；Drop 时递减 active count 并通知条件变量。`BackgroundPathLease` 由同步 driver、登录 watchdog 和收尾 monitor 持有到最终数据库写入及任务文件清理结束，不能以 `running == None` 代替。`MaintenanceLease` 不持有 mutex guard，允许 async 命令等待后台租约；Drop 未 commit 时自动解除维护状态并保留旧路径。

- [ ] **Step 4: 增加 Windows 单实例和 bootstrap mutex 失败测试**

  在 collector 参数分流测试中验证两个 GUI guard 只有一个成功，collector 模式不获取 GUI mutex；不同 bootstrap 目录使用不同 migration mutex，同一路径并发只能一个持有。进程异常退出后 mutex 由内核释放。

- [ ] **Step 5: 将所有 Tauri 数据命令改为从 AppState 取得路径租约**

  `AppState` 增加 `OnceLock<Arc<RuntimePaths>>`、bootstrap RMW 锁和 settings 写锁。删除 GUI 命令中的 `resolve_paths(&app)`；collector CLI 继续直接构造 `AppPaths::new`。同步和登录启动时把 lease 转交后台任务，所有清理完成后才释放。

- [ ] **Step 6: 运行 main/runtime 回归并提交**

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime_paths
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime
  cargo test --manifest-path src-tauri/Cargo.toml --bin douyin-history-archive
  ```

  Expected: PASS。

  Commit: `refactor: own data paths in app state`

### Task 3: Windows 目录规范化与真实权限探针

**Files:**
- Create: `src-tauri/src/data_root.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: `src-tauri/tests/data_root.rs`

**Interfaces:**
- Produces: `DataRootContext { app_data_dir, install_dir }`。
- Produces: `normalize_requested_root(context, requested) -> Result<NormalizedDataRoot, DataRootError>`，结果包含规范路径、父目录身份、目标原始存在状态和迁移 ownership token。
- Produces: `probe_data_root_permissions(path) -> Result<(), DataRootError>`。
- Produces error codes: `data_root_not_absolute`、`data_root_not_unicode`、`data_root_is_drive_root`、`data_root_is_remote`、`data_root_overlaps_current`、`data_root_not_empty`、`data_root_reparse_point`、`data_root_permission_denied`、`data_root_wal_probe_failed`。
- Rule: 规范化后的请求等于安装目录时，返回安装目录下 `Data`；比较使用规范绝对路径和 Windows 大小写不敏感语义。
- Rule: 使用 `GetDriveTypeW` 拒绝 UNC、mapped remote 和其他 remote volume；根目录及任意遍历到的子项都拒绝 symlink/junction/reparse point。

- [ ] **Step 1: 写入失败的路径规则测试**

  覆盖普通目录、安装目录根自动附加 `Data`、已经选择安装目录 `Data`、`C:\`、UNC share、映射网络盘、相对路径、当前根、当前根子目录、当前根父目录、根/内部 reparse point 及非空未知目标。

  ```rust
  assert_eq!(normalize_requested_root(&context, &install_dir)?.path, install_dir.join("Data"));
  assert_eq!(normalize_requested_root(&context, Path::new(r"C:\")).unwrap_err().code(), "data_root_is_drive_root");
  assert_eq!(normalize_requested_root(&context, &current.join("child")).unwrap_err().code(), "data_root_overlaps_current");
  ```

- [ ] **Step 2: 写入失败的文件与 SQLite WAL/SHM 探针测试**

  测试探针成功后不残留文件；只读目录返回稳定错误；SQLite 连接仍打开时必须观察到 `probe.sqlite-wal` 和 `probe.sqlite-shm`，完成后删除数据库及 sidecar。

- [ ] **Step 3: 运行并确认 RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test data_root probe -- --nocapture`

  Expected: FAIL，因为规范化和权限探针尚不存在。

- [ ] **Step 4: 实现规范化和探针**

  使用 `std::path`、Windows volume API 和 no-follow 元数据检查；目录选择 UI 使用 `rfd = "0.15"`，但所有规则和探针只在 Rust 命令中执行。对已存在空目标写入唯一 ownership token；探针顺序固定：创建专属目录 -> 文件 create_new/write/sync/read -> rename -> 双 SQLite 连接 WAL/锁语义与 sidecar 观察 -> 关闭连接 -> 删除所有探针文件和目录。提交前必须重新验证父目录、token、空目录和 reparse 状态。

- [ ] **Step 5: 运行测试并提交**

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test data_root
  cargo test --manifest-path src-tauri/Cargo.toml
  ```

  Expected: PASS。

  Commit: `feat: validate custom data roots`

### Task 4: 可恢复复制、校验和原子指针切换

**Files:**
- Modify: `src-tauri/src/data_root.rs`
- Modify: `src-tauri/src/bootstrap.rs`
- Modify: `src-tauri/src/collector/browser_state.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/tests/data_root.rs`
- Test: `src-tauri/tests/runtime_paths.rs`
- Test: `src-tauri/tests/browser_native.rs`

**Interfaces:**
- Produces: no-follow `DataInventory { file_count, total_bytes, critical_hashes, semantic_browser_hashes }`；关键文件为 `history.sqlite`、`settings.json` 和转换后的浏览器选择/登录标记语义值。
- Produces: `MigrationJournalV1 { schema_version, migration_id, source, target, staging, target_token, phase, source_inventory }`，只保存在 bootstrap 目录并受 bootstrap process mutex 保护。
- Produces: `migrate_data_root(source, target, bootstrap_dir, hooks) -> MigrationResult`。
- Produces: `recover_interrupted_data_root_migration(bootstrap_dir, bootstrap)`；只恢复/清理由精确 migration ID、target token 和 `rootId` 证明拥有的 staging/空壳，绝不根据模糊名称删除目录。
- Produces: `delete_retained_data_root(bootstrap_dir, expected_root)`；标准布局校验 marker/rootId 后删除，legacy-flat 只删除记录的顶层 allowlist 条目，永远不递归删除 bootstrap 根。
- Changes browser state: 新写 `BrowserSelection` schema v2 与 `LoginValidation` schema v3 不包含绝对 `profilePath`；旧 schema 仍可在 source root 读取，并在 staging 中重写为路径无关 schema。

- [ ] **Step 1: 写入失败的 inventory 与复制校验测试**

  构造数据库、settings、profiles、旧绝对路径浏览器状态和普通缓存文件，验证相对文件计数、总大小、关键哈希和浏览器状态语义哈希。复制后篡改一个关键文件必须返回 `data_root_verification_failed`；只改变未列为关键文件的字节数也必须由总大小比较发现；内部 junction/symlink 必须在复制前失败。

- [ ] **Step 2: 写入失败的迁移故障矩阵测试**

  通过 test hooks 在以下点注入失败：写 journal 前、复制中、浏览器状态重写中、checkpoint busy、校验前、删除目标空壳后、staging rename 后、bootstrap 写入前、bootstrap 写入后。断言 bootstrap 写入前始终指向 source；bootstrap 写入后 runtime 才可 commit；任一时刻最多一个路径被 `RuntimePaths::acquire` 返回。删除空壳后崩溃时，journal recovery 必须用 verified staging 完成 rename 或保留 source，不能创建空数据库。

  ```rust
  assert_eq!(load_bootstrap(&bootstrap_dir)?.current_root.path, source);
  assert!(!database_was_opened_at(&target));
  assert_eq!(runtime.acquire()?.data_root, source);
  ```

- [ ] **Step 3: 运行并确认 RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test data_root migration -- --nocapture`

  Expected: FAIL，因为迁移状态机尚不存在。

- [ ] **Step 4: 实现 checkpoint、复制、校验与原子切换**

  在无前台/后台租约后，对 source 数据库查询 `PRAGMA wal_checkpoint(TRUNCATE)` 三元组并要求 `busy == 0`。关闭最后连接后，`-shm` 必须不存在，`-wal` 只能不存在或零字节，复制清单明确排除 SHM 与零字节 WAL。复制到 source 树外、target 同卷 sibling staging，写入新的 rootId marker，把浏览器绝对 profile 状态重写为 schema v2/v3，再比较转换后的语义 inventory。提交前重新验证 target token、父目录身份和 no-reparse；移除受控空壳后原子 rename staging，随后原子写 bootstrap 并将 source identity 加入 retained list。只有 bootstrap 成功后调用 `MaintenanceLease::commit(new_paths)`。

- [ ] **Step 5: 实现旧版 app_data 根一次性收纳**

  bootstrap 缺失时，先以 legacy root 启动兼容 resolver；本步骤与默认切换到 `<app_data_dir>\Data` 在同一提交落地。旧布局只读取精确 allowlist：`history.sqlite`、`history.sqlite-wal`、`history.sqlite-shm`、`profiles`、`tmp`、`diagnostics`、`logs` 和根级 `browser-selection.json`，禁止通配符。copy staging 和 legacy-retained 目录都固定放在 `app_data_dir` 的 sibling，inventory 只遍历 allowlist，绝不递归扫描整个 source。bootstrap 切换后把旧平铺条目逐项 no-follow rename 到带 rootId marker 的 sibling retained 目录；若中断，startup journal 继续搬移，应用只打开新 Data。`device-settings.json` 的活动账号读入 bootstrap 后也移入 retained 目录。最终 `app_data_dir` 只保留 bootstrap/journal 临时文件，旧副本仍以 `RetainedLayout::LegacyFlat { root, entries, inventory }` 保留到用户确认删除。

- [ ] **Step 6: 实现 retained root 明确删除与安全回归**

  删除命令只能匹配 bootstrap 中完整 `DataRootRef`。遍历全程 no-follow 并拒绝任意内部 reparse point；标准根先删除普通子项、最后删除 marker 和根目录，legacy-flat 只删除 inventory 中未改变的 allowlist 条目。任何部分失败保留 marker（若尚未删除）和 retained 记录，便于安全重试。覆盖安装目录根、磁盘根、当前根、marker/rootId 不符、根或内部路径被替换为 junction 的拒绝测试。

- [ ] **Step 7: 验证迁移后浏览器登录态无需重建**

  使用旧 schema 选择/登录标记和受控 profile 完成迁移，随后从新 account root 调用 `read_browser_selection`、`read_login_validation` 和隐藏同步预检。断言浏览器 ID、可执行路径、validation ID 与历史 URL 保持一致，文件中不再包含旧/new data root 绝对路径。

- [ ] **Step 8: 运行测试并提交**

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test data_root
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime_paths
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime
  cargo test --manifest-path src-tauri/Cargo.toml --test browser_native
  ```

  Expected: PASS。

  Commit: `feat: migrate data roots atomically`

### Task 5: Tauri 设置与数据目录命令

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/settings_store.rs`
- Modify: `src-tauri/src/bootstrap.rs`
- Modify: `src-tauri/src/accounts.rs`
- Remove: `src-tauri/src/device_settings.rs`
- Test: `src-tauri/tests/runtime.rs`
- Test: `src-tauri/tests/settings_storage.rs`

**Interfaces:**
- Produces commands: `get_app_settings(account_id: Option<i64>)`、`update_global_settings(settings)`、`update_account_settings(account_id, settings)`、`import_legacy_settings(account_id, legacy)`。
- Produces commands: `choose_data_root()`、`validate_data_root(path)`、`set_data_root(path)`、`delete_retained_data_root(path)`。
- Produces: `AppSettingsView { revision, data_root, install_dir, retained_data_roots, legacy_import_required, global, account }`。
- Produces: `DataRootValidationView { requested_path, normalized_path, adjusted_to_install_data }` 与 `DataRootMigrationView { data_root, retained_source }`。
- Invariant: `set_data_root` 先取得 maintenance lease，lease 本身等待全部前台与后台路径租约归零；等待期间取消/退出只 abort lease，不复制、不改 bootstrap。`running`/`running_login` 仅用于 UI 状态，不作为迁移安全判据。

- [ ] **Step 1: 写入失败的 IPC 序列化和 scope 测试**

  验证全局更新不改账号设置、账号更新不改全局或其他账号、未知字段被拒绝、account ID 必须解析到当前数据库中的 uid/secUid portable key。legacy import 覆盖：相同 fingerprint 重试、Rust 已持久化但 IPC 响应丢失、无活动账号、导入期间切换账号及冲突 payload 拒绝。

- [ ] **Step 2: 写入失败的维护等待与竞态测试**

  用 barrier 模拟同步/登录在迁移请求前后启动，并让 monitor 在 `running` 清空后继续持有后台 lease。迁移标记一旦设置，新同步、登录、历史查询和设置写入均返回 `data_root_migration_in_progress`；只有最终数据库写入与文件清理释放后台 lease 后才调用复制函数。

- [ ] **Step 3: 运行并确认 RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test runtime data_root_commands -- --nocapture`

  Expected: FAIL，因为命令和维护闸门尚未注册。

- [ ] **Step 4: 实现命令和稳定中文错误映射**

  `choose_data_root` 在 blocking thread 调用 rfd。`validate_data_root` 只规范化并做权限探针，不复制。`set_data_root` 再次完整验证以防 TOCTOU，并在任务完成后返回实际生效路径。错误对前端保留稳定代码，UI 负责中文文案。

- [ ] **Step 5: 清理删除账号后的 settings map**

  本任务一次性把 `accounts.rs` 的活动账号读写从 `device_settings.rs` 迁移到 bootstrap read-modify-write，并在所有调用者编译通过后删除旧模块。删除账号前读取 portable account key，数据库删除成功后从 `settings.json.accounts` 删除对应项；失败只留下无害孤儿设置，下一次 settings load 根据数据库 uid/secUid key 集合清理并原子写回。

- [ ] **Step 6: 运行 Rust 全套并提交**

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml --test settings_storage
  cargo test --manifest-path src-tauri/Cargo.toml --test runtime
  cargo test --manifest-path src-tauri/Cargo.toml
  ```

  Expected: PASS。

  Commit: `feat: expose settings and data root commands`

### Task 6: 前端统一设置、一次性导入和数据位置 UI

**Files:**
- Modify: `app/src/types.ts`
- Modify: `app/src/app-api.ts`
- Modify: `app/src/settings.ts`
- Modify: `app/src/App.svelte`
- Modify: `app/src/styles.css`
- Modify: `app/src/App.test.ts`
- Create: `tests/app/settings.test.ts`
- Regenerate: `dist/index.html`
- Regenerate: `dist/assets/**`
- Regenerate: `dist/build-manifest.json`

**Interfaces:**
- Adds frontend mirrors for `AppSettingsView`、`GlobalSettings`、`AccountSettings`、`DataRootValidationView` 和 `DataRootMigrationView`。
- Replaces: 生产设置读取/写入不再直接使用 `localStorage`；`localStorage` 只在 `legacyImportRequired` 时读取一次。
- Produces: 串行 `settingsSaveQueue`，避免快速切换造成旧响应覆盖新值。
- Produces: 单调 `settingsRequestGeneration`；账号切换后的迟到 load/import/save 响应不得覆盖当前账号 UI。
- Produces: 设置浮窗中的“本地数据位置”行、只读当前路径、目录选择按钮、验证/迁移状态和 retained source 删除确认。

- [ ] **Step 1: 写入失败的 settings helper 与组件测试**

  覆盖 backend 设置初始化、账号切换读取不同账号设置、保存全局/账号 scope、legacy localStorage 成功后删除、Rust 已提交但 IPC 响应丢失后的下次启动清理、重复导入、无活动账号导入 defaults、导入期间账号切换、迟到响应丢弃、导入失败时保留、目录根错误中文映射、安装目录自动转 Data 提示、迁移期间禁用同步与设置控件。

  ```ts
  expect(api.importLegacySettings).toHaveBeenCalledWith(11, expect.objectContaining({ syncDays: 7 }));
  expect(localStorage.getItem("douyin-history-settings")).toBeNull();
  expect(api.updateGlobalSettings).toHaveBeenCalledWith(expect.objectContaining({ likeCountFormat: "k" }));
  expect(api.updateAccountSettings).toHaveBeenCalledWith(11, expect.objectContaining({ hideBrowser: true }));
  ```

- [ ] **Step 2: 运行并确认 RED**

  ```powershell
  npx vitest run tests/app/settings.test.ts app/src/App.test.ts
  npm run typecheck
  ```

  Expected: FAIL，因为新 API 和异步设置初始化尚不存在。

- [ ] **Step 3: 实现 API、类型和 legacy 导入 helper**

  Web 预览返回内存设置，Tauri 环境调用新增命令。legacy payload 先通过现有 normalize 函数收窄并计算稳定 fingerprint；若首次 `get_app_settings` 已返回 `legacyImportRequired == false`，直接删除遗留 key。调用 import 时，只有 Rust 返回完成状态或下一次读取确认完成后才删除 localStorage；generation 不匹配的响应只用于完成持久化确认，不更新当前账号 UI。

- [ ] **Step 4: 将 App 初始化改为先加载账号，再加载 Rust 设置**

  移除模块顶层 `loadSettings()`。`initializeAccounts()` 确认 active account 后调用 `refreshAppSettings(activeAccount?.id)`；切换账号成功后重新载入 account scope。设置加载失败显示错误，但不阻止账号列表和历史页面恢复。

- [ ] **Step 5: 实现数据位置设置 UI**

  使用 `Database`、`FolderOpen`、`Trash2` 图标。选择目录后先显示规范化路径，再由用户点击明确的“迁移数据”命令；迁移成功后刷新 settings 并显示旧目录保留状态。删除旧根必须使用独立确认浮层，禁止通过 modal backdrop 误触确认。

- [ ] **Step 6: 运行前端门禁并重建 committed dist**

  ```powershell
  npm test
  npm run typecheck
  npm run app:build
  ```

  Expected: PASS；`dist/build-manifest.json` 与全部输入/输出哈希一致。

- [ ] **Step 7: 提交**

  ```powershell
  git add app/src/types.ts app/src/app-api.ts app/src/settings.ts app/src/App.svelte app/src/styles.css app/src/App.test.ts tests/app/settings.test.ts dist/index.html dist/assets dist/build-manifest.json
  git commit -m "feat: manage settings and data location"
  ```

### Task 7: NSIS 升级保留与显式卸载数据选择

**Files:**
- Create: `src-tauri/nsis/installer-hooks.nsh`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/data_root.rs`
- Test: `src-tauri/tests/runtime.rs`
- Test: `src-tauri/tests/data_root.rs`

**Interfaces:**
- Adds process mode: `--uninstall-managed-data --bootstrap-dir <absolute path>`，在 Tauri 初始化前分流，不需要 Node，不获取 GUI mutex。
- Uses existing Tauri NSIS “删除应用数据” checkbox，默认未选中即保留；`NSIS_HOOK_PREUNINSTALL` 只在非 `/UPDATE` 且 checkbox 选中时调用卸载辅助模式。
- Invariant: 升级 `/UPDATE` 永不调用数据删除；安装目录内 `Data` 不是 bundle 文件，普通 uninstall 的非递归 `RMDir "$INSTDIR"` 不得删除它。
- Invariant: 卸载辅助模式使用与应用内 retained 删除相同的 rootId/no-follow 安全函数；bootstrap 无效时失败并保留数据，不猜测或扫描其他目录。

- [ ] **Step 1: 写入失败的卸载 CLI 和 NSIS hook 静态测试**

  验证 collector 分流优先级不变；卸载模式严格要求 absolute bootstrap dir；hook 包含 `$UpdateMode <> 1` 和 `$DeleteAppDataCheckboxState = 1` 两个条件，并将 `$APPDATA\${BUNDLEID}` 作为 bootstrap dir 传入当前主程序。

- [ ] **Step 2: 运行并确认 RED**

  Run: `cargo test --manifest-path src-tauri/Cargo.toml --test runtime uninstall_managed_data -- --nocapture`

  Expected: FAIL，因为卸载辅助模式和 hook 尚不存在。

- [ ] **Step 3: 实现卸载辅助模式与 hook**

  helper 读取严格 bootstrap，验证 current root marker/rootId 后删除当前标准数据根；legacy-flat retained 只按 allowlist 删除。删除成功后移除 bootstrap；任一步失败返回非零。hook 使用 `nsExec::ExecToStack` 检查退出码，失败时显示稳定中文错误并 `Abort`，确保 Tauri 后续 `RmDir /r "$APPDATA\${BUNDLEID}"` 不会误删 bootstrap。`tauri.conf.json` 只增加 `installerHooks`，不调整图标、图片、文案布局或其他视觉配置。

- [ ] **Step 4: 构建并检查生成的 installer.nsi**

  Run: `cargo tauri build`

  Expected: generated `installer.nsi` 在文件删除前展开 hook；`/UPDATE` 路径跳过 helper；未选中数据 checkbox 时不调用 helper；安装目录 `Data` 未出现在 bundle Delete 清单。

- [ ] **Step 5: 提交**

  ```powershell
  git add src-tauri/nsis/installer-hooks.nsh src-tauri/tauri.conf.json src-tauri/src/main.rs src-tauri/src/data_root.rs src-tauri/tests/runtime.rs src-tauri/tests/data_root.rs
  git commit -m "feat: preserve data during installer lifecycle"
  ```

### Task 8: 重启、安装包与迁移门禁

**Files:**
- Modify: `docs/stage-three-usable-version-roadmap.md`
- Create: `docs/testing/unified-settings-data-root-verification.md`
- Modify if needed: `src-tauri/tests/runtime.rs`
- Modify if needed: `src-tauri/tests/data_root.rs`

**Interfaces:**
- Provides: 可复现的升级、重启、迁移故障和 NSIS 验收记录。
- Changes roadmap: 只在自动门禁与安装包验证通过后标记 3.2 完成；3.3 仍保持未开始。

- [ ] **Step 1: 运行完整自动门禁**

  ```powershell
  cargo test --manifest-path src-tauri/Cargo.toml
  npm test
  npm run typecheck
  npm run app:build
  cargo tauri build
  ```

  Expected: 全部 PASS；正式安装包不包含 Node、NCC、CRI、Playwright、Chromium 或 Headless Shell。

- [ ] **Step 2: 执行隔离 APPDATA/LOCALAPPDATA 的旧版布局升级测试**

  通过测试专用 bootstrap override，或同时重定向 `APPDATA` 与 `LOCALAPPDATA`，准备旧布局数据库、profile marker、日志和 `device-settings.json`。启动新程序一次并退出，断言 bootstrap 指向带正确 rootId 的 `Data`，数据库 row count/`user_version` 不变，旧平铺受管条目仍作为 retained legacy-flat 存在，第二次启动仍打开同一路径且隐藏同步登录标记有效。

- [ ] **Step 3: 执行四类真实目录验收**

  验证普通可写目录成功；选择安装目录根时实际落在 `Data`；磁盘根、UNC、mapped remote 被拒绝；当前用户不可写目录被拒绝且没有 UAC。迁移后完成一次隐藏同步、重启、再次查询，确认数据库、profile 和登录状态均来自新根。

- [ ] **Step 4: 执行中断与旧根保留验收**

  在复制中和 bootstrap 切换前分别终止进程。重启后必须只打开旧数据库且无 target 写入；正常迁移后旧目录仍存在，拒绝删除当前根，用户确认删除 retained root 后仅删除精确旧根。

- [ ] **Step 5: 记录脱敏证据并更新路线图**

  报告只记录临时目录类别、文件计数、总字节数、哈希是否相等、错误码、数据库 schema/row count、安装包大小与 SHA-256；不得记录真实账号 ID、Cookie、profile 内容或用户绝对自定义路径。

- [ ] **Step 6: 执行无 Node 正式打包门禁**

  使用已提交 `dist/`，在子进程 PATH 中移除 Node 安装目录，先断言 `Get-Command node`、`npm`、`npx` 均失败，再单独执行 `cargo tauri build`。Expected: PASS，且安装包扫描无 Node/NCC/Playwright/CRI/browser binary。

- [ ] **Step 7: 提交验证文档**

  ```powershell
  git add docs/stage-three-usable-version-roadmap.md docs/testing/unified-settings-data-root-verification.md src-tauri/tests/runtime.rs src-tauri/tests/data_root.rs
  git commit -m "test: verify unified settings data roots"
  ```

## Completion Checklist

- [ ] `git status --short` 无未提交改动。
- [ ] bootstrap 是系统 `app_data_dir` 中唯一长期业务配置，`currentRoot.rootId` 与数据根 marker 完全一致，数据库和全部受管目录位于该根。
- [ ] 前端生产代码不再使用 `localStorage` 保存设置；旧 key 只在一次性导入路径读取。
- [ ] 普通、安装目录根自动转 Data、磁盘根、UNC/remote、无权限、重叠目录、reparse point 和非空目录测试均通过。
- [ ] GUI 单实例、bootstrap mutex 和前台/后台路径租约门禁通过；任一迁移故障点均保持旧 bootstrap 和旧 runtime path，不存在两个由应用同时写入的数据库。
- [ ] 浏览器状态文件不含数据根绝对 profile 路径；标准迁移和 legacy adoption 后隐藏同步均无需重新登录。
- [ ] WAL checkpoint busy、残留 WAL/SHM、目标空壳删除后崩溃及 bootstrap/marker identity mismatch 均失败关闭且不创建空数据库。
- [ ] 全 Rust、前端测试、typecheck、dist manifest、NSIS 构建和安装扫描通过。
- [ ] NSIS `/UPDATE` 保留安装目录 `Data`；显式卸载默认保留，只有用户选中删除应用数据时才调用 rootId 校验后的 helper。
- [ ] 安装包不包含 Node、NCC、CRI、Playwright、Chromium 或 Headless Shell。
- [ ] 未暂存账号标识、Cookie、数据库转储、真实 profile 或绝对用户数据路径。
