# Task 4 审查修复报告

## 状态

Task 4 审查 findings 已修复并完成验证。

## 修复内容

- 登录取消和应用退出共用取消编排：写入 cancel 后等待 2 秒；collector 未退出时先结束 collector，再终止账号、attempt、profile 绑定的 recorded browser；所有路径最终清理 attempt、identity、cancel 和 PID 文件。
- 登录浏览器终止与启动恢复从数据库账号的 `profile_key` 派生账号 profile，再按 browser 类型得到 expected profile。PID record 的 `profilePath`、完整 `--user-data-dir`、PID、创建时间、浏览器可执行文件和真实命令行参数必须全部匹配。
- 损坏 PID、跨账号/profile PID 或不完整 user-data-dir 参数不会触发终止回调，恢复仍清理对应登录尝试文件。
- `AccountIdentityV1` 严格拒绝 uid、secUid、nickname、avatarUrl 的首尾空白；`complete_login_attempt` 只使用 `validated_for` 返回的已验证身份值完成账号去重与激活。
- `account-identity.js` 删除任意对象和 Fiber 广度扫描，仅读取 `identitySourcePaths` 集中列出的状态路径。fixture 可覆盖来源，仅保留一个显式 React props 路径。
- 所有允许来源的候选都会被收集和规范化。UID 或 secUid 冲突返回 `account_identity_ambiguous`；无可靠候选或字段结构未知返回 `account_identity_unavailable`。页面必须绑定 `www.douyin.com/user/self?showTab=record`。

## TDD 记录

- RED：空白身份字段被接受，空白 UID 在 complete 阶段进入重复账号匹配。
- RED：多个候选身份返回 unavailable，脚本仍包含 Fiber/Object.values/queue 广度扫描。
- RED：浏览器身份校验不接收 expected profile，record/profile mismatch 无法表达。
- RED：损坏或跨 profile PID 在恢复路径可能进入终止回调。
- RED：退出强制取消缺少 collector -> browser -> cleanup 顺序保证；poisoned child lock 会跳过文件清理。
- GREEN：新增回归测试覆盖上述路径并通过。

## 验证

- `cargo test --test account_login`：12 passed。
- `cargo test --test runtime`：7 passed。
- `cargo test --test browser_native`：9 passed。
- `cargo test --test native_collector`：11 passed。
- `cargo test --bin douyin-history-archive`：15 passed。
- 完整 `cargo test`：全部已运行测试通过；1 个需要已安装系统浏览器的在线集成测试按原配置 ignored。
- `git diff --check`：通过，仅有仓库现有 Windows LF/CRLF 转换提示。

## 审查结论

现有差异仅覆盖 Task 4 登录身份、浏览器进程绑定、取消/退出清理、恢复与对应测试和报告，没有包含无关功能改动。

## 最后一项补充修复

- 应用退出登录 attempt 时仍先写入 cancel 并等待 2 秒。collector 仅在等待后仍未退出时执行 kill；无论 collector 是否已协作退出，都会尝试终止 account、attempt、profile 严格绑定的 recorded browser，最后清理 attempt 文件。
- recorded browser PID 文件不存在时直接安全返回，不调用终止回调；PID 已不存在时视为 browser 已退出并安全 no-op。进程仍存在时继续执行创建时间、可执行文件、profile 和完整 `--user-data-dir` 参数校验，身份不匹配仍拒绝终止。
- RED：协作退出测试实际事件仅为 `cleanup`，期望 `browser -> cleanup`；已退出 PID 返回“浏览器进程已退出或无法识别”错误。
- GREEN：`cargo test --manifest-path src-tauri/Cargo.toml --bin douyin-history-archive` 为 17 passed；`cargo test --manifest-path src-tauri/Cargo.toml --test account_login` 为 12 passed。
- 完整 `cargo test --manifest-path src-tauri/Cargo.toml` 全部通过；1 个需要已安装系统 Edge 或 Chrome 的在线集成测试按原配置 ignored。
- `git diff --check` 通过，仅有仓库现有 Windows LF/CRLF 转换提示。
