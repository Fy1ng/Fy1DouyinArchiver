# Task 3 Report

## Status

完成多账号浏览器 profile、selection、登录验证、同步任务文件和登录 attempt 文件隔离；collector CLI/runner 已携带账号上下文，采集结果升级为 `CollectorSuccessV3`，Rust 导入前校验结果账号与 `sync_jobs.account_id` 一致。

未实现 Task 4 身份页面读取、身份结果解析或 Tauri 账号命令。

## RED

- 路径与 browser state：`cargo test --manifest-path src-tauri/Cargo.toml --test runtime --test browser_native` 首次因缺少账号路径接口、任务路径仍为全局布局、login validation 不接收账号根而编译失败。
- collector 契约：`cargo test --manifest-path src-tauri/Cargo.toml --test native_collector --test storage_v2` 首次因缺少 `CollectorSuccessV3`、CLI 账号字段、进度/诊断 `accountId` 和结果账号校验而编译失败。
- 登录诊断：单测首次因 `LoginDiagnostic` 不存在而编译失败。
- 取消隔离：单测首次因账号级 PID 终止 helper 不存在而编译失败。
- 跨账号 login validation：最后新增测试首次得到 `invalid_login_validation`，预期为 `browser_profile_scope_mismatch`；调整检查顺序和错误传播后通过。

## GREEN

- 专项：`cargo test --manifest-path src-tauri/Cargo.toml --test runtime --test browser_native --test native_collector`
- 结果：23 passed，0 failed。
- 全量：`cargo test --manifest-path src-tauri/Cargo.toml`
- 结果：86 passed，0 failed，1 ignored；ignored 项为既有的本机 Edge/Chrome 系统浏览器集成测试。
- 差异检查：`git diff --check` 通过。

## Implementation

- `AppPaths` 校验 32 位小写无连字符 UUID `profile_key`，并生成账号级 profile、同步与登录 attempt 路径。
- browser selection 位于 `profiles/<profile_key>/browser-selection.json`，login validation 位于账号浏览器 profile 内；跨账号 selection 或 validation 路径返回 `browser_profile_scope_mismatch`。
- collect CLI 必须包含 `--account-id` 与 `--account-profile-root`；login 额外包含 attempt、identity result、diagnostic、cancel 和 browser PID 文件参数。
- `CollectorSuccessV3`、采集进度和同步诊断包含 `accountId`；登录失败诊断包含 `accountId` 与 `loginAttemptId`。
- Tauri 从数据库读取 `profile_key` 后构造 collector invocation，不接收前端路径；同步完成、取消和启动恢复均使用 `account_id + job_id` 派生文件。
- 全局 stale temp 文件枚举已移除，避免仅凭文件名跨账号清理。

## Directly Related Files Beyond Brief

- `src-tauri/src/collector/browser_runtime.rs`：适配账号 profile root，并传播跨账号状态错误。
- `src-tauri/src/collector/contracts.rs`、`src-tauri/src/collector/sync_engine.rs`：定义 V3 结果、进度和诊断账号契约。
- `src-tauri/src/main.rs`：从数据库读取 profile key，接线账号级 invocation、完成、取消、恢复与日志路径。
- `src-tauri/tests/native_collector.rs`、`src-tauri/tests/storage_v2.rs`：覆盖 CLI、V3 序列化和结果账号二次校验。

## Commit

- `feat: isolate browser profiles by account`

## Concerns

- 无未解决的 Task 3 correctness concern。
- Task 4 仍需实现身份页面读取及 identity result 生成/消费，本任务仅保留账号级文件和 CLI 契约。
- 全仓 `cargo fmt --check` 会报告仓库既有未格式化文件；未执行全仓格式化，避免引入与 Task 3 无关的改动。
