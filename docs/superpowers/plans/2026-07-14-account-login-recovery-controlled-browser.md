# Account Login Recovery and Controlled Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Project `AGENTS.md` prohibits implementation subagents; do not use `subagent-driven-development`. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让新账号仅在抖音身份校验成功后可见，登录浏览器关闭后两秒内恢复重试，并对应用专属 Edge/Chrome profile 统一禁用用户扩展、浏览器账号登录、同步和后台驻留。

**Architecture:** 保留内部 `pending_login` 行作为稳定任务身份，但从所有公开账号列表隐藏。collector 通过任务级 `LoginProgressV1` 将关闭原因和截止时间传给 Tauri，Tauri 自己维护两秒看门狗；慢速 profile 删除转为持久清理标记。浏览器约束由统一启动参数和每次启动前对 `Default/Preferences`、`Local State` 的幂等原子合并共同实现。

**Tech Stack:** Rust 2021、Tauri 2、Tokio、tokio-tungstenite、rusqlite 0.32、serde/serde_json、Windows Job Object、Svelte 5、TypeScript、Vitest、原生 CDP。

## Global Constraints

- 需求基线唯一来源：`docs/superpowers/specs/2026-07-14-account-login-recovery-controlled-browser-design.md`。
- 仅支持 Windows x64、系统 Edge/Chrome 和应用专属 `profiles/<profile_key>/<browser>/`。
- 禁止修改系统普通浏览器 profile，以及 `HKCU/HKLM\Software\Policies\Microsoft\Edge`、`HKCU/HKLM\Software\Policies\Google\Chrome`。
- 生产运行和 `cargo tauri build` 不得引入 Node、Playwright、NCC、CRI 或随包浏览器；Node 只允许开发前端与生成已提交 `dist/`。
- `pending_login` 永不通过公开账号列表返回；新账号失败必须删除 pending 行并从全新 profile 重试。
- 数据库提交为 `active` 是新登录和重新登录的不可逆成功点；提交后的设备设置或普通文件清理失败不得改写为登录失败。
- 浏览器关闭后两秒只约束 pending 可见性、运行锁和 UI 按钮恢复；被锁 profile 使用持久标记后台清理。
- 用户扩展、默认应用及后台组件扩展必须禁用；PDF 查看器、错误页和设置页等内建组件允许保留。
- 每项实现严格执行失败测试、确认预期失败、最小实现、定向回归、完整回归和单一提交。
- 除设计文档审查和计划审查外严禁调用子代理；本计划实施全程内联执行。

---

### Task 1: 隐藏内部 Pending 账号

**Files:**
- Modify: `src-tauri/src/accounts.rs`
- Modify: `src-tauri/tests/account_login.rs`

**Interfaces:**
- Consumes: `list_accounts(connection)` 的完整内部账号集合。
- Produces: `list_command_accounts(connection, include_archived)` 只返回 `active | login_expired`，以及可选 `archived`；永不返回 `pending_login`。

- [ ] **Step 1: 写公开列表隐藏 pending 的失败测试**

将 `command_account_views_and_active_state_follow_task_four_status_rules` 的管理列表断言改为：

```rust
let managed = list_command_accounts(&connection, true).unwrap();
assert_eq!(managed.len(), 3);
assert!(!managed.iter().any(|account| account.id == management_pending.id));
assert!(list_accounts(&connection)
    .unwrap()
    .iter()
    .any(|account| account.id == management_pending.id));
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test account_login command_account_views_and_active_state_follow_task_four_status_rules
```

Expected: Rust 测试仍返回 pending。

- [ ] **Step 3: 实现公开过滤并删除 pending 管理分支**

`list_command_accounts` 固定过滤：

```rust
.filter(|account| {
    account.status == "active"
        || account.status == "login_expired"
        || (include_archived && account.status == "archived")
})
```

内部 `list_accounts`、`create_pending_account` 与兼容命令暂时保留；前端 pending 分支在 Task 6 通过真实渲染测试删除。

- [ ] **Step 4: 运行定向和完整测试**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test account_login
```

Expected: 全部通过。

- [ ] **Step 5: 提交**

```powershell
git add src-tauri/src/accounts.rs src-tauri/tests/account_login.rs
git commit -m "fix: hide incomplete account logins"
```

### Task 2: 统一浏览器约束与 Profile 偏好

**Files:**
- Create: `src-tauri/src/collector/profile_constraints.rs`
- Modify: `src-tauri/src/collector/mod.rs`
- Modify: `src-tauri/src/collector/session.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/tests/browser_native.rs`

**Interfaces:**
- Produces: `apply_profile_constraints(account_profile_root: &Path, browser_id: BrowserId) -> Result<PathBuf, String>`，返回经过校验的直接子 profile 路径。
- Produces: `browser_launch_arguments(profile: &Path, headless: bool) -> Vec<String>` 始终包含统一约束参数。
- Consumes: `file_store::write_json_atomically`、`browser_state` 的链接/路径防护模式。

- [ ] **Step 1: 写启动参数失败测试**

```rust
#[test]
fn every_browser_session_disables_extensions_signin_sync_and_background_mode() {
    for headless in [false, true] {
        let args = browser_launch_arguments(Path::new(r"C:\App\profile"), headless);
        for required in [
            "--disable-extensions",
            "--disable-component-extensions-with-background-pages",
            "--disable-default-apps",
            "--disable-sync",
            "--disable-background-mode",
            "--no-first-run",
            "--no-default-browser-check",
        ] {
            assert!(args.iter().any(|arg| arg == required), "{required}");
        }
    }
}
```

- [ ] **Step 2: 写 profile 合并失败测试**

建立临时 `Default/Preferences` 和 `Local State`，保留哨兵字段后调用尚不存在的函数：

```rust
let profile = apply_profile_constraints(&account_root, BrowserId::Edge).unwrap();
let preferences: Value = read_json(&profile.join("Default/Preferences")).unwrap();
assert_eq!(preferences.pointer("/signin/allowed_on_next_startup"), Some(&json!(false)));
assert_eq!(preferences.pointer("/sync/requested"), Some(&json!(false)));
assert_eq!(preferences.pointer("/sentinel"), Some(&json!("keep")));
let local_state: Value = read_json(&profile.join("Local State")).unwrap();
assert_eq!(local_state.pointer("/background_mode/enabled"), Some(&json!(false)));
```

再覆盖缺失文件、重复调用、损坏 JSON、Preferences 文件 symlink/junction 和普通文件父目录，断言拒绝且不触碰外部目标。

- [ ] **Step 3: 运行测试确认失败**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_native profile_constraints
cargo test --manifest-path src-tauri/Cargo.toml every_browser_session_disables
```

Expected: 缺少模块/函数或缺少参数。

- [ ] **Step 4: 实现最小 profile 合并器**

`profile_constraints.rs` 只能从可信账号根与浏览器 ID 推导目标：

```rust
pub fn apply_profile_constraints(
    account_profile_root: &Path,
    browser_id: BrowserId,
) -> Result<PathBuf, String> {
    validate_account_root_filesystem_scope(account_profile_root)?;
    let profile = browser_profile_path(account_profile_root, browser_id);
    validate_or_create_direct_profile(account_profile_root, &profile)?;
    merge_json(profile.join("Default/Preferences"), &[
        (&["signin", "allowed_on_next_startup"], json!(false)),
        (&["signin", "allowed"], json!(false)),
        (&["sync", "requested"], json!(false)),
        (&["sync", "has_setup_completed"], json!(false)),
    ])?;
    merge_json(profile.join("Local State"), &[
        (&["background_mode", "enabled"], json!(false)),
    ])?;
    Ok(profile)
}
```

目标必须是账号根的规范化直接子目录，browser ID 只能为 edge/chrome；账号根、profile、Default、Preferences 和 Local State 每一级都拒绝 symlink/reparse。`merge_json` 对缺失文件使用空对象，只更新指定对象路径，同目录原子替换；存在文件必须是非链接普通文件，根 JSON 必须是对象。

- [ ] **Step 5: 在每次浏览器启动前应用约束**

将 `launch_browser_session` 改为接收账号根与 BrowserId，并在启动前执行：

```rust
let profile = apply_profile_constraints(account_profile_root, browser_id)?;
let args = browser_launch_arguments(profile, headless);
```

runner 从 `BrowserChoice.browser_id` 和任务级账号根调用该接口，不接受调用者直接传入任意 profile。扩展参数列表，保持隐藏模式额外参数不变。

- [ ] **Step 6: 运行回归并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test browser_native
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/collector/profile_constraints.rs src-tauri/src/collector/mod.rs src-tauri/src/collector/session.rs src-tauri/src/collector/runner.rs src-tauri/tests/browser_native.rs
git commit -m "feat: constrain managed browser profiles"
```

### Task 3: 登录进度契约与 CDP 关闭信号

**Files:**
- Modify: `src-tauri/src/collector/contracts.rs`
- Modify: `src-tauri/src/collector/cli.rs`
- Modify: `src-tauri/src/collector/cdp.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/native_collector.rs`
- Modify: `src-tauri/tests/runtime.rs`
- Modify: `src-tauri/tests/cdp_transport.rs`

**Interfaces:**
- Produces: `LoginProgressV1`、`LoginProgressReason`、`LoginOptions.progress_path`。
- Produces: `AppPaths::login_progress_path(account_id, attempt_id)`。
- Produces: `CdpCloseReason { PageSocket, BrowserSocket, TargetDestroyed }` 与 `CdpPage::subscribe_closed() -> watch::Receiver<Option<CdpCloseReason>>`；三者在登录边界统一映射为 `BrowserClosed`。
- Consumes: Task 2 的统一浏览器 session。

- [ ] **Step 1: 写 JSON 与 CLI 契约失败测试**

固定类型：

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginProgressV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub login_attempt_id: String,
    pub state: String,
    pub reason: LoginProgressReason,
    pub observed_at_ms: i64,
    pub deadline_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginProgressReason { Cancelled, BrowserClosed, Timeout, Failed }
```

测试未知字段、错误 schema/account/attempt、非 `closing`、`deadline < observed` 和超过 2000ms 的 collector deadline。`build_login_invocation` 必须包含 `--login-progress-file`，CLI 必须拒绝不等于 `AppPaths::login_progress_path` 的路径。

- [ ] **Step 2: 运行契约测试确认失败**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test runtime login
cargo test --manifest-path src-tauri/Cargo.toml --test native_collector login_progress
```

Expected: 缺少字段、路径和类型。

- [ ] **Step 3: 实现 AppPaths、CLI 与调用参数**

任务文件名固定为：

```text
tmp/account-<account_id>-login-<attempt_id>.progress.json
```

`LoginOptions` 新增 `progress_path: PathBuf`，解析器和调用构造器均绑定该路径。`cleanup_login_attempt_files` 也删除 progress 文件。

- [ ] **Step 4: 写真实 WebSocket 关闭失败测试**

在 Tokio `TcpListener` 上接受 WebSocket，收到一个 CDP 请求后直接关闭连接：

```rust
let connection = CdpConnection::connect(&url).await.unwrap();
let started = Instant::now();
assert_eq!(
    connection.call::<Value>("Runtime.evaluate", json!({})).await,
    Err("cdp_target_closed".into())
);
assert!(started.elapsed() < Duration::from_secs(1));
```

另一个夹具发送 `Target.targetDestroyed`，断言只有匹配 `target_id` 才更新 `CdpPage` 的专用 close receiver；同时向现有普通事件队列发送大量无关事件，证明 close 信号不会被单消费者事件队列抢走。

- [ ] **Step 5: 实现无损关闭广播**

`CdpConnection` 增加连接级 `watch::Sender<Option<CdpCloseReason>>`。reader 结束时先广播，再 drain 全部 pending waiter。`CdpPage` 另持有当前 target 专用的 close watch channel；Browser reader 识别匹配的 `Target.targetDestroyed` 后直接写该通道，不经过普通事件队列。Browser 连接启用：

```rust
browser.call::<Value>("Target.setDiscoverTargets", json!({"discover": true})).await?;
```

`CdpPage::subscribe_closed` 将 page/browser 连接关闭与匹配 Target 销毁汇聚到专用 receiver；其他 Target 事件继续保留给调用方。关闭广播后所有 pending request 立即返回 `cdp_target_closed`。

- [ ] **Step 6: 在身份等待中保留原始终止原因**

将单次身份探测也纳入同一个 `tokio::select!`，不得先 sleep 再串行等待最长 60 秒的 evaluate：

```rust
loop {
    let evaluate = page.evaluate(ACCOUNT_IDENTITY_SCRIPT, &json!({}));
    tokio::pin!(evaluate);
    tokio::select! {
        biased;
        _ = close_rx.changed() => return Err(read_close_reason(&close_rx)?),
        _ = cancel_rx.changed() => return Err("account_login_cancelled".into()),
        _ = tokio::time::sleep_until(hard_deadline) => return Err("account_identity_unavailable".into()),
        identity = &mut evaluate => {
            let identity = identity.map_err(map_evaluate_login_error)?;
            if let Some(identity) = validate_identity(identity)? { return Ok(identity); }
        }
    }
    tokio::select! {
        biased;
        _ = close_rx.changed() => return Err(read_close_reason(&close_rx)?),
        _ = cancel_rx.changed() => return Err("account_login_cancelled".into()),
        _ = tokio::time::sleep_until(hard_deadline) => return Err("account_identity_unavailable".into()),
        _ = tokio::time::sleep(Duration::from_millis(200)) => {}
    }
}
```

`map_evaluate_login_error` 必须把 `cdp_target_closed`、page/browser socket closed 映射为 `account_login_browser_closed`，而不是普通 `failed`。增加 close channel 与 evaluate 同时 ready 的暂停时间测试，连续运行 100 次均必须得到 BrowserClosed；cancel 和硬超时同理优先于普通 evaluate 失败。

`run_login` 在观察到取消、浏览器关闭、超时或失败后，先原子写 `LoginProgressV1`，`deadline_at_ms = observed_at_ms + 2000`，再清理 session。不得把 `cdp_target_closed` 折叠为普通取消。

- [ ] **Step 7: 运行回归并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test runtime --test native_collector --test cdp_transport
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/collector/contracts.rs src-tauri/src/collector/cli.rs src-tauri/src/collector/cdp.rs src-tauri/src/collector/runner.rs src-tauri/src/lib.rs src-tauri/tests/native_collector.rs src-tauri/tests/runtime.rs src-tauri/tests/cdp_transport.rs
git commit -m "feat: report bounded login termination"
```

### Task 4: Pending 失败清理与启动恢复

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/accounts.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/tests/account_login.rs`
- Modify: `src-tauri/tests/accounts.rs`

**Interfaces:**
- Produces: `PendingProfileCleanupV1`、`cleanup_failed_login`、`retry_pending_profile_cleanups`。
- Produces: `PendingLoginCleanupGuard`，pending 创建后立即持有，只有数据库提交 `active` 后才 `disarm()`。
- Produces: `LoginCleanupReport { primary_error: Option<String>, deferred_profile: bool }`。
- Produces: `AppPaths::pending_profile_cleanup_path(profile_key) -> Result<PathBuf, String>`。
- Consumes: Task 3 的 progress/attempt 文件路径。

- [ ] **Step 1: 写失败矩阵测试**

分别建立 pending、active、login_expired、archived 和缺失账号的 attempt/PID/profile：

```rust
recover_login_attempts_with(&paths, &connection, terminate).unwrap();
assert!(get_account(&connection, pending_id).unwrap().is_none());
assert!(!pending_profile.exists());
assert!(active_profile.exists());
assert!(expired_profile.exists());
assert!(archived_profile.exists());
```

损坏/跨 profile/PID 复用记录不得调用 terminate。文件删除器和 profile 删除器使用注入闭包制造失败，断言 pending 行仍被删除、后续步骤仍执行并保留首个错误。

- [ ] **Step 2: 写延迟 profile 清理测试**

注入 `remove_profile` 返回共享冲突，断言生成：

```json
{"schemaVersion":1,"profileKey":"<32 hex>","createdAt":"<RFC3339>"}
```

标记路径固定为 `tmp/pending-profile-cleanup-<profile_key>.json`。重试成功后 profile 与标记同时删除；外部路径、链接和无效 profileKey 必须拒绝。

再使用可注入时钟和删除器验证后台重试策略：命令返回后立即尝试一次，随后按 `250ms, 500ms, 1s, 2s, 4s` 最多五次；成功即删除标记，仍失败则保留给启动恢复。一个损坏标记只被隔离/删除并记录诊断，不得阻止其他标记或应用启动。

- [ ] **Step 3: 运行测试确认失败**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test account_login startup_recovery
cargo test --manifest-path src-tauri/Cargo.toml --test accounts cleanup
```

Expected: 当前恢复只删 attempt 文件，不删 pending 行/profile，也没有清理标记。

- [ ] **Step 4: 实现不可跳过的清理报告**

固定结果：

```rust
pub struct LoginCleanupReport {
    pub primary_error: Option<String>,
    pub deferred_profile: bool,
}
```

`cleanup_failed_login` 依次尝试终止、删除 pending 行、删除所有任务文件、删除 profile；每一步使用 `record_first_error`，不得提前返回。只有初始账号状态是 `pending_login` 才删除行/profile；其他状态只清任务文件。

实现 `PendingLoginCleanupGuard`，在 pending 行创建成功的下一行立即构造。guard 保存 account/profile/attempt 上下文和幂等清理句柄；所有提前返回与 panic unwind 都触发 Task 4 清理。guard 执行破坏性清理前必须重新查询数据库，只有账号仍为 `pending_login` 且 attempt/profile_key 与 guard 完全匹配时才删除行/profile。数据库事务提交 `active` 后的下一条语句立即 `disarm()`，再进行设备设置与普通文件清理。profile 慢删除时 guard 只登记标记，不阻塞 UI 截止时间。

- [ ] **Step 5: 重写启动恢复决策表**

先用内部 `list_accounts` 枚举全部 pending，再独立扫描 attempt 文件。pending 即使没有 attempt 文件也必须清理。active/login_expired/archived 只终止匹配进程和删任务文件。最后逐个容错调用 `retry_pending_profile_cleanups`，并启动有界后台重试任务；单个损坏标记不得使恢复函数整体失败。

- [ ] **Step 6: 为 collector 启动前每个失败点写故障注入测试**

将登录编排依赖抽成 `LoginLaunchHooks`（attempt 文件写入、invocation 构造、日志创建、collector spawn），逐项注入失败：

```rust
for stage in [
    LaunchFailure::CreateAttempt,
    LaunchFailure::BuildInvocation,
    LaunchFailure::CreateLog,
    LaunchFailure::SpawnCollector,
] {
    let result = begin_account_login_with_hooks(&state, hooks.failing_at(stage)).await;
    assert!(result.is_err());
    assert!(list_command_accounts(&connection, true).unwrap().is_empty());
    assert!(list_accounts(&connection).unwrap().iter().all(|a| a.status != "pending_login"));
    assert!(!account_profile_root.exists());
    assert!(state.running_login.lock().unwrap().is_none());
}
```

`CreateAttempt` 指 pending 已创建、任务文件首次写入失败；四个阶段都必须证明 guard 生效、运行状态释放。另在 active 提交和 `disarm()` 之间注入 panic/错误，证明 guard 重新读取状态后不会删除 active 账号/profile；设备设置失败同样不得触发清理。

- [ ] **Step 7: 运行完整 Rust 回归并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test accounts --test account_login
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/lib.rs src-tauri/src/accounts.rs src-tauri/src/main.rs src-tauri/tests/account_login.rs src-tauri/tests/accounts.rs
git commit -m "fix: clean failed account logins atomically"
```

### Task 5: Tauri 两秒看门狗与登录成功边界

**Files:**
- Create: `src-tauri/src/login_watchdog.rs`
- Create: `src-tauri/src/login_orchestrator.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/tests/account_login.rs`
- Modify: `src-tauri/tests/runtime.rs`

**Interfaces:**
- Produces: `LoginWatchdog`、`LoginWatchdogEvent`、`LoginTermination`、可注入的 `LoginProcess`、`LoginClock`、`BrowserProcessQuery`、`LoginCleanupHooks`。
- Produces: `CompletedLogin { account: AccountView, device_settings_warning: Option<String> }`。
- Produces Tauri command: `cancel_account_login() -> Result<(), String>`。
- Consumes: Tasks 3–4 的 progress、V2 PID 与清理函数。

固定终止枚举：

```rust
pub enum LoginTermination { Cancelled, BrowserClosed, Timeout, Failed }
```

- [ ] **Step 1: 写纯状态机失败测试**

固定状态机行为：

```rust
let mut watchdog = LoginWatchdog::new(started_at, hard_deadline);
assert_eq!(watchdog.observe_progress(valid_browser_closed, now), LoginWatchdogEvent::BeginShutdown);
assert_eq!(watchdog.deadline(), now + Duration::from_secs(2));
assert_eq!(watchdog.tick(now + Duration::from_millis(500)), LoginWatchdogEvent::ForceBrowser);
assert_eq!(watchdog.tick(now + Duration::from_secs(2)), LoginWatchdogEvent::ForceCollectorAndRelease);
```

覆盖过去 deadline 立即强制、未来超过两秒 deadline 被钳制、错误 account/attempt 被忽略、浏览器 V2 进程先退出时合成 `BrowserClosed`。

RED command:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test runtime login_watchdog_state_machine
```

Expected: `login_watchdog` 模块/类型尚不存在。

- [ ] **Step 2: 写完成边界失败测试**

抽出可注入设置写入器：

```rust
let completed = complete_login_attempt_with_settings_writer(
    &paths, pending.id, ATTEMPT_ID, |_path, _settings| Err("disk full".into())
).unwrap();
assert_eq!(completed.account.status, "active");
assert_eq!(completed.device_settings_warning.as_deref(), Some("device_settings_update_failed"));
assert!(profile.exists());
```

对 `login_expired` 重复同一测试。身份冲突、数据库提交失败仍返回 Err，并保持原状态。

RED command:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test account_login login_completion_boundary
```

Expected: 可注入 settings writer 与 `CompletedLogin` 尚不存在。

- [ ] **Step 3: 实现完成结果**

`complete_login_attempt_with_settings_writer` 先 finalize 数据库。提交成功后设备设置写入最多尝试两次；失败返回 `Ok(CompletedLogin { warning })`。提交后的 attempt/progress/identity 清理也是非致命，不得删除 profile。

- [ ] **Step 4: 替换阻塞等待为父进程轮询**

将父轮询从 `main.rs` 抽到 `login_orchestrator.rs`，通过 trait/闭包注入 child、时钟、进程查询和清理动作，使测试无需真实等待。`run_account_login` 每 50ms：

1. `child.try_wait()`；
2. 读取并验证 progress；
3. 读取 V2 PID 并检查浏览器创建时间/进程是否仍存活；
4. 驱动 `LoginWatchdog`。

开始关闭后写 cancel；500ms 仍未退出则终止匹配浏览器；两秒截止仍未退出则 kill collector，再终止匹配浏览器。无论普通清理结果如何，都清除 `running_login` 并调用 Task 4 清理。新 pending 返回稳定错误；active/login_expired 失败保留账号/profile。

命令返回后，Task 4 的 profile marker 后台重试继续运行，但不持有浏览器操作锁。`CompletedLogin.device_settings_warning` 只写日志/诊断；Tauri 命令固定返回 `completed.account`，绝不把 warning 转换为 Err。

- [ ] **Step 5: 增加取消命令与应用退出复用**

```rust
#[tauri::command]
async fn cancel_account_login(state: State<'_, AppState>) -> Result<(), String> {
    cancel_current_login_with_watchdog(state.inner(), LoginTermination::Cancelled).await
}
```

加入 Tauri invoke handler。应用退出调用同一函数的同步封装，不再维护第二套取消顺序。

- [ ] **Step 6: 故障注入验证两秒边界**

使用不会退出的子进程和模拟 browser PID：progress 写入 BrowserClosed 后断言 collector 被 kill、pending 行删除、`running_login` 为空、总耗时小于 2200ms。另测 progress 永远缺失但浏览器进程先退出的路径。

- [ ] **Step 7: 运行回归并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test account_login --test runtime
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/login_watchdog.rs src-tauri/src/login_orchestrator.rs src-tauri/src/lib.rs src-tauri/src/main.rs src-tauri/tests/account_login.rs src-tauri/tests/runtime.rs
git commit -m "fix: bound account login shutdown"
```

### Task 6: 前端设置与登录失败恢复

**Files:**
- Create: `app/src/account-login-ui.ts`
- Create: `app/src/App.test.ts`
- Create: `vitest.config.ts`
- Modify: `app/src/app-api.ts`
- Modify: `app/src/App.svelte`
- Modify: `tests/app/accounts.test.ts`
- Modify: `app/src/types.ts`
- Modify: `package.json`
- Modify: `package-lock.json`

**Interfaces:**
- Produces: `accountLoginButtonLabel(inProgress)`、`accountLoginErrorMessage(error)`。
- Produces API wrapper: `cancelAccountLogin()`。
- Consumes: Task 5 的稳定错误码和成功 AccountView。

- [ ] **Step 1: 写纯 UI 状态失败测试**

```ts
expect(accountLoginButtonLabel(false)).toBe("添加账号");
expect(accountLoginButtonLabel(true)).toBe("等待登录…");
expect(accountLoginErrorMessage("account_login_browser_closed"))
  .toBe("登录浏览器已关闭，未添加账号");
expect(accountLoginErrorMessage("account_login_cancelled"))
  .toBe("登录已取消，未添加账号");
```

IPC 测试精确断言 `cancelAccountLogin()` 调用 `invoke("cancel_account_login")`。

- [ ] **Step 2: 写真实 Svelte 渲染失败测试**

配置 Vitest `environment: "jsdom"` 并加入 `@testing-library/svelte` 与 `@testing-library/jest-dom`。mock `app-api` 后渲染 `App`，验证无账号时设置可操作、登录失败错误可见、pending 不出现、失败后按钮恢复：

```ts
listAccountsMock.mockResolvedValue([
  { id: 99, nickname: "PENDING-SHOULD-NOT-RENDER", status: "pending_login" },
]);
render(App);
await waitFor(() => expect(listAccountsMock).toHaveBeenCalled());
await fireEvent.click(screen.getByRole("button", { name: "设置" }));
expect(screen.getByRole("dialog", { name: "设置" })).toBeVisible();

await fireEvent.click(screen.getByRole("button", { name: "账号管理" }));
expect(screen.queryByText("PENDING-SHOULD-NOT-RENDER")).not.toBeInTheDocument();
beginAccountLoginMock.mockRejectedValueOnce("account_login_browser_closed");
await fireEvent.click(screen.getByRole("button", { name: "添加账号" }));
expect(await screen.findByText("登录浏览器已关闭，未添加账号")).toBeVisible();
expect(screen.getByRole("button", { name: "添加账号" })).toBeEnabled();
expect(screen.queryByText("PENDING-SHOULD-NOT-RENDER")).not.toBeInTheDocument();
```

- [ ] **Step 3: 运行测试确认失败**

```powershell
npm test -- tests/app/accounts.test.ts
npm test -- app/src/App.test.ts
npm run typecheck
```

Expected: helper/wrapper 缺失，真实渲染测试中设置不可用或 pending 昵称进入 DOM。

- [ ] **Step 4: 实现独立登录忙碌状态**

新增 `let accountLoginBusy = false`。`addAccount` 只控制该状态并在 `finally` 中恢复：

```ts
accountLoginBusy = true;
try {
  activeAccount = await beginAccountLogin();
  await refreshAccounts(accountManagerOpen);
  await refreshHistory(1);
} catch (error) {
  errorMessage = accountLoginErrorMessage(error);
} finally {
  accountLoginBusy = false;
}
```

所有添加入口显示 `accountLoginButtonLabel(accountLoginBusy)` 并只在登录中禁用。账号切换、归档、删除和重新登录控件统一使用 `accountBusy || accountLoginBusy`，避免登录期间改变账号集合；设置按钮永不因这两个状态禁用。

- [ ] **Step 5: 移动全局 UI 块**

将 `settingsOpen` modal、`errorMessage` 和 `noticeMessage` 块移动到账号空状态分支之前；历史专属同步错误仍留在 active 分支。移除账号管理中的 pending 文案与按钮。

- [ ] **Step 6: 运行前端回归并提交**

```powershell
npm test
npm run typecheck
npm run app:build
git add app/src/account-login-ui.ts app/src/App.test.ts app/src/app-api.ts app/src/App.svelte app/src/types.ts tests/app/accounts.test.ts vitest.config.ts package.json package-lock.json dist
git commit -m "fix: recover account login controls"
```

### Task 7: Rust-only 集成、安装包与验收报告

**Files:**
- Modify: `src-tauri/tests/cdp_system_browser.rs`
- Modify: `src-tauri/tests/frontend_manifest.rs`
- Create: `scripts/verify-browser-boundaries.ps1`
- Create: `scripts/verify-upgrade-baseline.ps1`
- Create: `scripts/audit-installed-package.ps1`
- Create: `scripts/run-browser-constraint-matrix.ps1`
- Create: `docs/verification/2026-07-14-account-login-recovery.md`
- Modify: `docs/stage-three-usable-version-roadmap.md`

**Interfaces:**
- Consumes: Tasks 1–6 的完整登录流程。
- Produces: 自动门禁、Edge/Chrome 安装包验收记录和最新 NSIS 安装包。

- [ ] **Step 1: 扩展 Rust-only CDP 夹具矩阵**

嵌入测试页分别模拟：成功身份、Page Target 关闭、Browser WebSocket 断开和不退出 collector。断言 progress 原因、父看门狗动作、pending 清理和 profile 保留决策。增加四个明确测试名：`edge_visible_constraints`、`edge_hidden_constraints`、`chrome_visible_constraints`、`chrome_hidden_constraints`；每个 test 内部显式设置 browser/headless，并用 RAII 环境守卫恢复原值。`scripts/run-browser-constraint-matrix.ps1` 先以 `cargo test -- --list` 确认四个测试都存在，再逐个运行并要求每项恰好 `1 passed; 0 failed; 0 ignored`；零测试、浏览器缺失、ignored/skipped 均返回非零。

- [ ] **Step 2: 验证扩展与 profile 约束**

测试 profile 预置一个带 background/content script 哨兵的本地扩展。先用取消约束参数的正向对照启动，证明扩展确实能生成哨兵；再使用生产参数启动，断言 background/content 两类哨兵均未生成。保留一个无关 Preferences 键和抖音 Cookie 数据库哈希，约束合并前后必须一致。

- [ ] **Step 3: 运行全部自动门禁**

```powershell
npm test
npm run typecheck
npm run app:build
cargo test --manifest-path src-tauri/Cargo.toml
./scripts/run-browser-constraint-matrix.ps1
$repoRoot = (Resolve-Path -LiteralPath '.').Path
$bundleRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'src-tauri/target/release/bundle'))
$nsisRoot = [IO.Path]::GetFullPath((Join-Path $bundleRoot 'nsis'))
if (-not $nsisRoot.StartsWith($bundleRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'unsafe nsis cleanup path' }
if (Test-Path -LiteralPath $nsisRoot) { Remove-Item -LiteralPath $nsisRoot -Recurse -Force }
cargo tauri build
```

Expected: 全部退出码 0；系统浏览器集成测试在对应浏览器已安装时通过。

- [ ] **Step 4: 实际安装后扫描发布物**

将以下逻辑实现为 `scripts/audit-installed-package.ps1 -Installer <exact-path> -ExpectedSha256 <sha>`。安装包路径必须来自本次 `cargo tauri build` 前清空的 `bundle/nsis` 目录，构建后要求恰好一个新 EXE，禁止 `Select-Object -First 1` 选取旧文件。先校验 SHA-256，再静默安装到任务级临时目录，递归检查实际安装内容；测试结束无论成功失败都运行卸载器并删除临时目录：

```powershell
$installers = @(Get-ChildItem -LiteralPath 'src-tauri/target/release/bundle/nsis' -Filter '*.exe')
if ($installers.Count -ne 1) { throw "expected exactly one installer, got $($installers.Count)" }
$installer = $installers[0].FullName
$installerSha = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
$installRoot = Join-Path $env:TEMP ("emily-package-audit-" + [guid]::NewGuid().ToString('N'))
try {
  $process = Start-Process -FilePath $installer -ArgumentList '/S', "/D=$installRoot" -Wait -PassThru -WindowStyle Hidden
  if ($process.ExitCode -ne 0) { throw "silent install failed: $($process.ExitCode)" }
  $forbidden = Get-ChildItem -LiteralPath $installRoot -Recurse -Force | Where-Object {
    $_.Name -match '(?i)(^node(w)?\.exe$|^index\.cjs$|\.map$|chromium|chrome-headless|headless-shell|msedge\.exe|chrome\.exe)' -or
    $_.FullName -match '(?i)(playwright|chrome-remote-interface|@vercel[\\/]ncc|[\\/](runtime|collector|node_modules|browser|browsers)[\\/])'
  }
  if ($forbidden) { throw "forbidden installed files: $($forbidden.Name -join ', ')" }
} finally {
  $uninstaller = Join-Path $installRoot 'uninstall.exe'
  if (Test-Path -LiteralPath $uninstaller) {
    Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -WindowStyle Hidden
  }
  $resolvedTemp = [IO.Path]::GetFullPath($env:TEMP).TrimEnd([IO.Path]::DirectorySeparatorChar)
  $resolvedInstall = [IO.Path]::GetFullPath($installRoot)
  if (-not $resolvedInstall.StartsWith($resolvedTemp + [IO.Path]::DirectorySeparatorChar + 'emily-package-audit-', [StringComparison]::OrdinalIgnoreCase)) { throw 'unsafe audit cleanup path' }
  if (Test-Path -LiteralPath $resolvedInstall) { Remove-Item -LiteralPath $resolvedInstall -Recurse -Force }
}
```

Expected: 命令退出码 0 且不打印匹配。Cargo manifest 额外文件、runtime/collector 目录、source map 和浏览器二进制拒绝测试全部通过。

- [ ] **Step 5: 执行安装包人工验收**

在 Windows 已登录 Microsoft 账号的环境分别验证 Edge/Chrome：无账号设置、关闭窗口两秒恢复、失败账号不可见、成功后出现、同步提示不显示、用户扩展不运行、浏览器账号登录/同步不可用、抖音 Cookie 持久、可见/隐藏同步正常。再从上一安装包升级验证既有 profile。

`scripts/verify-browser-boundaries.ps1` 开始前要求所有普通 Edge/Chrome 进程已关闭，否则返回非零；随后按规范化路径和键名排序，自动导出应用外 profile 目录摘要和四个 Edge/Chrome policy 注册表路径的键名/类型/值哈希。只比较确定性哈希与结构，不在报告中记录账号、Cookie 或注册表原值；任一差异返回非零退出码。

`scripts/verify-upgrade-baseline.ps1` 必须接收并记录：基线 Git commit、基线安装包绝对路径与 SHA-256、隔离的基线 app-data 模板目录、升级安装包绝对路径与 SHA-256。每轮开始先确认 Edge/Chrome 已关闭、卸载旧应用、清除本轮安装目录，并从只读模板复制出全新的基线 app-data；摘要采用规范化路径和字段排序。步骤固定为校验两包哈希、安装基线、暂停等待人工完成一次登录并更新模板、保存 profile/数据库摘要、覆盖升级、暂停等待执行登录与同步、比较数据/profile/策略摘要；无法建立干净状态、缺少参数或摘要变化时返回非零退出码。

发布命令固定执行：

```powershell
./scripts/audit-installed-package.ps1 -Installer $installer -ExpectedSha256 $installerSha
./scripts/verify-browser-boundaries.ps1 -Mode Before -SnapshotPath $boundarySnapshot
# 完成人工 Edge/Chrome 验收
./scripts/verify-browser-boundaries.ps1 -Mode After -SnapshotPath $boundarySnapshot
./scripts/verify-upgrade-baseline.ps1 -BaselineCommit $baselineCommit -BaselineInstaller $baselineInstaller -BaselineSha256 $baselineSha -AppDataDir $baselineAppData -UpgradeInstaller $installer -UpgradeSha256 $installerSha
```

- [ ] **Step 6: 写验收报告并更新路线图**

报告记录 Windows、Edge、Chrome、安装包版本，R1–R7 的自动测试和人工结果、安装包路径/SHA-256，以及未通过项。只有 Edge 和 Chrome 均通过时才将该阶段标为完成。

- [ ] **Step 7: 提交**

```powershell
git add src-tauri/tests/cdp_system_browser.rs src-tauri/tests/frontend_manifest.rs scripts/verify-browser-boundaries.ps1 scripts/verify-upgrade-baseline.ps1 scripts/audit-installed-package.ps1 scripts/run-browser-constraint-matrix.ps1 docs/verification/2026-07-14-account-login-recovery.md docs/stage-three-usable-version-roadmap.md
git commit -m "test: verify account login recovery"
git status --short --branch
```

## Final Verification

- [ ] `pending_login` 永不出现在普通或管理列表。
- [ ] 浏览器关闭后两秒内命令返回、锁释放、按钮恢复，慢 profile 清理已登记。
- [ ] 新账号失败删除 pending 行和临时 profile；已有账号重登失败保留账号/profile。
- [ ] 新登录与重登在数据库提交 active 后均返回成功，设备设置失败只产生非致命诊断。
- [ ] Edge/Chrome 所有调用路径统一禁用用户扩展、同步、后台驻留，并应用 profile 偏好。
- [ ] 无账号状态设置和错误提示可用。
- [ ] 普通系统浏览器 profile 与全局策略未变化。
- [ ] 前端、Rust、CDP 实页、manifest、NSIS 和无 Node 扫描门禁全部通过。
