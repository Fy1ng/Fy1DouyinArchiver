# Task 2 Report

## Status

完成 accounts repository、设备当前账号状态、账号身份决策矩阵、账号归档/恢复/删除，以及本地收藏夹 disposition 与 aggregate provenance 规则。

## RED

- 命令：`cargo test --manifest-path src-tauri/Cargo.toml --test accounts`
- 结果：失败，`douyin_history_archive::accounts` 与 `douyin_history_archive::device_settings` 不存在。
- 失败原因符合预期：测试先定义 Task 2 的公开接口和行为，生产模块尚未实现。

## GREEN

- 专项：`cargo test --manifest-path src-tauri/Cargo.toml --test accounts`
- 结果：19 passed，0 failed。
- 全量：`cargo test --manifest-path src-tauri/Cargo.toml`
- 结果：全部通过；1 个需要本机 Edge/Chrome 的既有系统测试 ignored。
- 格式：`rustfmt --edition 2021 --check src-tauri/src/accounts.rs src-tauri/src/device_settings.rs src-tauri/tests/accounts.rs` 通过。

## Commit

- `216588e feat: add account repository and active account state`
- 提交仅包含：`src-tauri/src/accounts.rs`、`src-tauri/src/device_settings.rs`、`src-tauri/src/lib.rs`、`src-tauri/tests/accounts.rs`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`。

## Self-review

- 身份矩阵覆盖 pending、active、login_expired、archived、单字段命中、双字段分裂命中、已有账号命中和 profile_key 稳定性。
- 所有账号仓库数据库写操作使用 immediate transaction；身份输入统一 trim，并校验 ID、昵称和 HTTP(S) 头像 URL 上限。
- 设备设置使用同目录临时文件、刷盘和原子替换；无效 active id 会保留文件并自修复为空。
- 删除流程先提交数据库，再写设备设置，最后清理受控 profiles 根的直接子目录；设置或目录失败不会伪装成数据库回滚。
- account scope provenance 写入在仓库层被拒绝；转换、转移、删除和孤立视频清理均有测试。转换测试使用临时审计触发器证明原 owner provenance 在账号删除前确实写入。
- SQLite 错误仅在约束冲突时映射为 `account_identity_conflict`，其他存储错误保留原始错误信息。

## Concerns

- 无未解决的 Task 2 correctness concern。
- 全量测试中 `cdp_system_browser` 的 1 个测试因需要本机已安装浏览器而按既有标记 ignored；不属于本任务回归失败。

---

## Review Findings Fix (2026-07-12)

### Scope

- Profile 删除不再只依赖 `profile_key` 的词法直接子目录判断。存在目标会先读取 `symlink_metadata`，拒绝 symlink；Windows 额外通过 `MetadataExt::file_attributes` 拒绝 `FILE_ATTRIBUTE_REPARSE_POINT`，覆盖 junction。
- 存在目标会 canonicalize profiles root 和 target，并要求 target 的 canonical parent 与 canonical root 完全相等。不存在目标保持安全 no-op；实际删除前再次执行同一验证。
- `AccountFolderDisposition::TransferToAccount.target_account_id` 显式反序列化为 `targetAccountId`。
- 身份矩阵增加 pending 账号命中自身 uid+sec_uid，以及 uid+sec_uid 同时命中同一 active/login_expired 账号的精确数据用例。
- 删除账号的生产入口继续使用真实原子 device settings writer；测试通过注入 writer closure 确定性制造提交后写失败，不再依赖目录/Windows 文件系统错误。

### TDD Evidence

- Profile junction RED：旧实现返回 `Ok(())`，未拒绝 junction；修复后返回 `profile_path_not_controlled`，外部 sentinel 保留且账号记录未删除。
- Serde RED：`targetAccountId` 报 `unknown field`，旧实现期待 `target_account_id`；显式 rename 后通过。
- Writer 注入 RED：测试先因 `delete_account_data_with_settings_writer` 接口不存在而编译失败；增加生产 wrapper 和注入入口后通过。
- 两个身份命中测试首次运行即通过，确认生产决策矩阵已正确，finding 属于原测试数据未真正覆盖目标分支。

### Verification

- `cargo test --test accounts`：23 passed，0 failed。
- `cargo test`：全部通过；既有 `cdp_system_browser` 系统浏览器测试 1 ignored。
- `git diff --check`：提交前执行并通过。

### Concerns

- 无未解决的 Task 2 correctness concern。
- junction 回归测试在 Windows 使用系统内置 `mklink /J` 创建临时 junction；测试目标完全位于测试临时目录内。
