# Schema v3 Task 1 Report

## 状态

- 完成。
- 实现提交：`5159d1c63d8c86fca78b68eb490640f66a1998ec` (`feat: add schema v3 multi-account storage`)

## 变更文件

- `src-tauri/migrations/003_multi_account.sql`
- `src-tauri/src/lib.rs`
- `src-tauri/src/main.rs`
- `src-tauri/tests/storage_v3.rs`
- `src-tauri/tests/storage_v2.rs`
- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock`
- `.superpowers/sdd/task-1-report.md`

## 失败测试证据

1. `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`
   - RED：编译失败，缺少 `migrations/003_multi_account.sql` 和 `initialize_schema_v3`。
2. `cargo test --manifest-path src-tauri/Cargo.toml --bin douyin-history-archive sync_requires_an_active_account`
   - RED：编译失败，缺少 `require_active_account_id` 和 v3 初始化接口。
3. `cargo test --manifest-path src-tauri/Cargo.toml --bin douyin-history-archive sync_status_is_idle_without_an_active_account`
   - RED：编译失败，缺少 `load_latest_sync_status`。
4. `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3 public_sync_options_reject_unknown_fields`
   - RED：断言失败，`SyncOptions` 当时仍接受未知字段。
5. 首轮完整 `cargo test --manifest-path src-tauri/Cargo.toml`
   - 回归证据：`runtime::invocation_uses_current_executable_and_task_files` 因旧前端字段 `blockMedia`/`blockImages` 被拒绝而失败；将两者显式建模为兼容字段后恢复通过，同时继续拒绝真正未知字段。

## 通过测试

- `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`
  - PASS：8 passed，0 failed。
- `cargo test --manifest-path src-tauri/Cargo.toml`
  - PASS：44 passed，0 failed，1 ignored（需要已安装系统 Edge/Chrome 的环境测试）。
- `git diff --check`
  - PASS：无空白错误。

## 自审

- v3 SQL 是生产与测试的唯一迁移来源，不包含 `BEGIN`、`COMMIT`、`PRAGMA foreign_keys` 或 `CREATE TABLE IF NOT EXISTS`。
- v0、v1、v2 使用 `Immediate` 事务重建；v3 验证关键表和外键；未来版本返回 `unsupported_schema_version`。
- 覆盖迁移清空、重复初始化、约束、账号隔离、外键检查和中途失败完整回滚。
- 所有 Tauri 生产数据库入口已切换到 `initialize_schema_v3`；`initialize_schema_v2` 仅用于迁移测试构造输入。
- 无 active account 时，历史列表为空、删除计数为零、同步状态为 idle、启动同步返回 `no_active_account`，且不会创建虚假账号。
- 旧历史导入、查询和删除按 active account 范围执行；共享视频只在所有来源关系均无引用时清理。
- 未实现账号仓库、设备当前账号状态或账号 UI，保持 Task 1 范围。
- 公开反序列化结构使用 `deny_unknown_fields`，同时显式保留旧前端兼容字段。

## Concerns

- 原简报文件清单漏列 `src-tauri/src/main.rs`；用户随后明确要求将其纳入本任务所有权和提交。
- `cargo fmt --check` 会报告仓库中大量既有、非本任务文件的格式差异；为避免无关改写，未运行全仓自动格式化。本任务新增测试文件已单独使用 `rustfmt`。
- 系统浏览器 CDP 集成测试因本机环境要求保持 ignored；其余 Rust 测试全部通过。
- 基于 `device-settings.json` 的当前账号选择和 `profiles/<profile_key>/<browser>/` 生产接线按计划留给后续账号/认证任务；本任务只提供 schema 与无账号保护，不提前实现账号仓库。

## Schema v3 Task 1 审查修复

### 修复说明

- `assert_schema_v3` 现在使用 `PRAGMA table_info` 验证各 v3 表的关键列，缺失时返回稳定的 `schema_v3_missing_column:<table>:<column>`。
- 使用 `PRAGMA index_list` 与 `PRAGMA index_info` 验证账号、视频、观看记录、点赞、收藏和文件夹关系的必要唯一约束。
- 使用 `PRAGMA index_list`、`PRAGMA index_info` 和 `sqlite_master` 验证账号标识与本地文件夹命名索引的唯一、partial、列顺序及谓词语义；损坏时返回稳定的 `schema_v3_invalid_index:<index>`。
- 为 `RawHistoryItem` 和 `RawReactSnapshot` 添加 `#[serde(deny_unknown_fields)]`，公开采集页面契约不再静默接受未知字段。
- 从 `Cargo.toml` 及根 crate 的 `Cargo.lock` 依赖列表移除 Task 1 未使用的直接 `uuid` 依赖；锁文件中的 `uuid` 包仍由既有传递依赖引用。

### 失败测试证据

1. `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`
   - RED：`8 passed，3 failed`。
   - 缺失 `accounts.created_at`、被替换为普通索引的 `idx_accounts_uid`、缺失观看记录复合唯一约束均被旧校验错误接受。
2. `cargo test --manifest-path src-tauri/Cargo.toml --test native_collector`
   - RED：`5 passed，2 failed`。
   - `RawHistoryItem` 与 `RawReactSnapshot` 均错误接受 `unexpected` 字段。

### 通过测试

- `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`
  - PASS：`11 passed，0 failed`。
- `cargo test --manifest-path src-tauri/Cargo.toml --test native_collector`
  - PASS：`7 passed，0 failed`。
- `cargo test --manifest-path src-tauri/Cargo.toml`
  - PASS：`49 passed，0 failed，1 ignored`；ignored 项需要已安装的系统 Edge 或 Chrome。

### 审查修复 Concerns

- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 仍因大量既有、非本任务 Rust 文件的格式差异失败；未执行全仓格式化，避免提交无关改动。
- Schema 校验刻意验证关键结构不变量，而不是复制完整 migration SQL；表级 CHECK 约束和全部非唯一性能索引仍由迁移测试及行为测试覆盖。

## Schema v3 Task 1 二次复审修复

### 修复说明

- `assert_schema_v3` 现在逐表验证全部 v3 列名以及 `PRAGMA table_info` 返回的 `notnull`、`pk` 签名；不校验默认值。
- 缺失列继续返回 `schema_v3_missing_column:<table>:<column>`；`NOT NULL` 或主键序号损坏分别返回稳定的 `schema_v3_invalid_column:<table>:<column>:notnull|pk`。
- 意外新增列返回稳定的 `schema_v3_unexpected_column:<table>:<column>`，避免只验证预期列子集。
- partial unique index 只提取并规范化 `WHERE` 谓词后做精确比较，不再使用整段 SQL `contains`。
- 所有必要唯一索引改用 `PRAGMA index_xinfo` 验证键列顺序与 collation；账号 `uid`/`sec_uid` 要求 `BINARY`，本地收藏夹 `name` 要求 `NOCASE`。

### TDD 失败测试证据

- `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`
  - RED：`11 passed，6 failed`。
  - 旧实现错误接受：未抽查的 `accounts.nickname` 缺失、`videos.canonical_url` 丢失 `NOT NULL`、本地收藏夹来源复合主键退化为普通唯一约束、账号 partial predicate 被扩宽、账号 uid 改为 `NOCASE`、本地收藏夹 name 索引改为 `BINARY`。

### 通过测试

- `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`
  - PASS：`17 passed，0 failed`。
- `cargo test --manifest-path src-tauri/Cargo.toml`
  - PASS：`55 passed，0 failed，1 ignored`；ignored 项需要已安装的系统 Edge 或 Chrome。
- `git diff --check`
  - PASS：无空白错误；Git 仅提示工作区 LF 将按本机配置转换为 CRLF。

### 二次复审 Concerns

- 列签名按列名匹配并验证完整列集合、`notnull` 与 `pk`，刻意不把列声明顺序或默认值纳入 v3 兼容契约。
- partial predicate 规范化忽略谓词外部的 ASCII 空白与 SQL 关键字/标识符大小写，但保留字符串字面量内容，因此扩宽或改变字面量的谓词会被拒绝。
- `rustfmt --check` 会从 `lib.rs` 递归检查既有 collector 子模块，并报告大量非本任务格式差异；未执行全仓格式化，避免引入无关改动。
