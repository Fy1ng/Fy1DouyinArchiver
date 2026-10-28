# Schema V3 Multi-Account Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将当前单账号 schema v2 应用迁移为单 SQLite、多抖音账号、账号独立浏览器 profile 和账号范围查询/同步的 schema v3 基础。

**Architecture:** `videos` 继续作为跨账号共享的内容元数据表，观看、喜欢、收藏、远端收藏夹和同步任务通过 `account_id` 隔离。账号远端身份同时保存可选 `uid` 与 `sec_uid`，至少一个存在并分别建立唯一索引；本地 `profile_key` 使用 UUID，与远端身份和昵称解耦。当前账号属于设备状态，保存在 `app_data_dir/device-settings.json`，不进入可跨设备覆盖的数据库设置。

**Tech Stack:** Rust 2021、Tauri 2、rusqlite 0.32、serde/serde_json、uuid 1、Svelte 5、TypeScript、Vitest、SQLite schema v3、Rust 原生 CDP。

## Global Constraints

- 当前处于开发阶段，v3 migration 直接清空现有 videos、watch_records 和 sync_jobs，不迁移旧观看数据或旧全局浏览器 profile。
- 一个数据库保存全部账号；所有账号事件和关系必须显式包含 `account_id`。
- `videos` 仅保存公共内容元数据，不因账号或来源重复创建。
- 每个账号使用 `profiles/<profile_key>/<edge|chrome>/`，不得共享 Cookie、selection 或登录标记；普通续登复用原 profile，只有显式“使用新 profile 重新认证”流程才允许轮换。
- 昵称和头像不是身份依据；`uid` 或 `sec_uid` 至少一个存在，任一匹配即视为同一远端账号。
- 喜欢、收藏和远端收藏夹本计划只建立 schema 与仓库契约，不实现页面采集。
- 本地收藏夹支持账号专属与多账号聚合两种 scope；聚合收藏夹按视频去重并保留来源账号关系。
- 生产运行与打包不得重新引入 Node、Playwright、NCC 或随包浏览器。
- 每个数据库写操作使用事务；每个公开反序列化结构启用 `deny_unknown_fields`。
- 每个任务按失败测试、最小实现、完整回归、独立提交执行。

---

### Task 1: Schema V3 重建迁移与未来来源表

**Files:**
- Create: `src-tauri/migrations/003_multi_account.sql`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/storage_v3.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: 现有 `initialize_schema_v2(&mut Connection)` 和 `002_react_history.sql`。
- Produces: `initialize_schema_v3(&mut Connection) -> Result<(), String>`，以及 v3 表、索引和外键约束。

- [ ] **Step 1: 写迁移失败测试**

在 `storage_v3.rs` 创建 v2 内存数据库，插入旧视频、观看记录和同步任务，调用尚不存在的 `initialize_schema_v3`，断言：

```rust
assert_eq!(user_version(&connection), 3);
assert_eq!(count(&connection, "accounts"), 0);
assert_eq!(count(&connection, "videos"), 0);
assert_eq!(count(&connection, "watch_records"), 0);
assert_eq!(count(&connection, "sync_jobs"), 0);
assert!(foreign_keys_enabled(&connection));
```

再增加 schema 约束测试：重复非空 `uid`、重复非空 `sec_uid`、重复 `profile_key`、跨账号重复观看身份、无效 folder scope 和缺少账号外键均失败。迁移矩阵覆盖真实 v0、v1、v2、已是 v3 和大于 v3；另外覆盖重复初始化、`PRAGMA foreign_key_check` 为空，以及故意注入 SQL 中途失败后原 schema 与 `user_version` 完整保留。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3`

Expected: FAIL，缺少 `initialize_schema_v3` 或 migration 文件。

- [ ] **Step 3: 编写唯一 migration SQL**

`initialize_schema_v3` 使用 `TransactionBehavior::Immediate` 包裹 SQL；SQL 文件自身不得包含 `BEGIN/COMMIT`。`003_multi_account.sql` 首先按子表到父表顺序删除所有可能来自 v1/v2/v3 的表，禁止使用 `CREATE TABLE IF NOT EXISTS` 掩盖半迁移状态：

```sql
DROP TABLE IF EXISTS local_folder_item_accounts;
DROP TABLE IF EXISTS local_folder_items;
DROP TABLE IF EXISTS local_folders;
DROP TABLE IF EXISTS remote_folder_items;
DROP TABLE IF EXISTS remote_folders;
DROP TABLE IF EXISTS favorite_items;
DROP TABLE IF EXISTS user_likes;
DROP TABLE IF EXISTS watch_records;
DROP TABLE IF EXISTS sync_jobs;
DROP TABLE IF EXISTS videos;
DROP TABLE IF EXISTS accounts;
```

随后重建以下结构：

```sql
CREATE TABLE accounts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  uid TEXT,
  sec_uid TEXT,
  profile_key TEXT NOT NULL UNIQUE,
  nickname TEXT,
  avatar_url TEXT,
  status TEXT NOT NULL CHECK(status IN ('pending_login', 'active', 'login_expired', 'archived')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_login_validated_at TEXT,
  CHECK(uid IS NULL OR length(trim(uid)) > 0),
  CHECK(sec_uid IS NULL OR length(trim(sec_uid)) > 0),
  CHECK(uid IS NOT NULL OR sec_uid IS NOT NULL OR status = 'pending_login')
);
CREATE UNIQUE INDEX idx_accounts_uid ON accounts(uid) WHERE uid IS NOT NULL;
CREATE UNIQUE INDEX idx_accounts_sec_uid ON accounts(sec_uid) WHERE sec_uid IS NOT NULL;

CREATE TABLE videos (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  aweme_id TEXT NOT NULL UNIQUE,
  canonical_url TEXT NOT NULL,
  content_type TEXT NOT NULL CHECK(content_type IN ('video', 'note', 'unknown')),
  title TEXT,
  author_name TEXT,
  cover_url TEXT,
  like_count INTEGER,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);

CREATE TABLE sync_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  started_at TEXT NOT NULL,
  finished_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('running', 'success', 'failed', 'cancelled')),
  inserted_videos INTEGER NOT NULL DEFAULT 0,
  inserted_records INTEGER NOT NULL DEFAULT 0,
  error_code TEXT,
  error_message TEXT
);

CREATE TABLE watch_records (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  history_time_ms INTEGER NOT NULL,
  watched_at TEXT NOT NULL,
  watched_date TEXT NOT NULL,
  source_position INTEGER NOT NULL,
  watch_progress_rate REAL CHECK(watch_progress_rate BETWEEN 0.0 AND 1.0),
  sync_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(account_id, video_id, history_time_ms)
);

CREATE TABLE user_likes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_created_at_ms INTEGER,
  remote_removed_at TEXT,
  source_position INTEGER,
  UNIQUE(account_id, video_id)
);

CREATE TABLE favorite_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_created_at_ms INTEGER,
  remote_removed_at TEXT,
  source_position INTEGER,
  UNIQUE(account_id, video_id)
);

CREATE TABLE remote_folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  remote_folder_id TEXT NOT NULL,
  name TEXT NOT NULL,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_removed_at TEXT,
  UNIQUE(account_id, remote_folder_id)
);

CREATE TABLE remote_folder_items (
  folder_id INTEGER NOT NULL REFERENCES remote_folders(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  remote_removed_at TEXT,
  PRIMARY KEY(folder_id, video_id)
);

CREATE TABLE local_folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  scope TEXT NOT NULL CHECK(scope IN ('account', 'aggregate')),
  owner_account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
  name TEXT NOT NULL COLLATE NOCASE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK((scope = 'account' AND owner_account_id IS NOT NULL) OR
        (scope = 'aggregate' AND owner_account_id IS NULL))
);
CREATE UNIQUE INDEX idx_local_folders_account_name
  ON local_folders(owner_account_id, name)
  WHERE scope = 'account';
CREATE UNIQUE INDEX idx_local_folders_aggregate_name
  ON local_folders(name)
  WHERE scope = 'aggregate';

CREATE TABLE local_folder_items (
  folder_id INTEGER NOT NULL REFERENCES local_folders(id) ON DELETE CASCADE,
  video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  added_at TEXT NOT NULL,
  PRIMARY KEY(folder_id, video_id)
);

CREATE TABLE local_folder_item_accounts (
  folder_id INTEGER NOT NULL,
  video_id INTEGER NOT NULL,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  added_at TEXT NOT NULL,
  PRIMARY KEY(folder_id, video_id, account_id),
  FOREIGN KEY(folder_id, video_id) REFERENCES local_folder_items(folder_id, video_id) ON DELETE CASCADE
);
```

添加 canonical URL 普通索引、history 时间排序、各来源 account/status 和远端移除索引，最后执行 `PRAGMA user_version = 3`。SQL 文件不得自行执行 `PRAGMA foreign_keys`，该设置由连接初始化在事务外完成；SQL 文件是生产代码与测试的唯一 migration 来源。

- [ ] **Step 4: 实现版本分派**

`initialize_schema_v3` 先在事务外执行 `PRAGMA foreign_keys = ON` 并断言已生效，再读取 `user_version`：版本 3 验证全部关键表并执行 `foreign_key_check`；0、1、2 使用立即事务执行 `003_multi_account.sql` 并在提交前断言版本与外键检查；大于 3 返回 `unsupported_schema_version`。

本 Task 立即将所有生产数据库入口切换为 v3。为了让旧前端在 Task 6 之前仍可启动，增加临时兼容适配：无 active account 时历史列表返回空、同步返回 `no_active_account`、删除返回零计数、状态返回 idle；兼容层不得创建虚假账号或旧数据。`initialize_schema_v2` 只保留给迁移测试构造 v2 输入。

- [ ] **Step 5: 运行迁移测试和完整 Rust 测试**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: v0/v1/v2/v3/future version、失败回滚和外键测试全部 PASS；旧 storage_v2 测试按新预期更新后全部 PASS；当前前端可启动并显示无账号空状态兼容结果。

- [ ] **Step 6: 提交**

```powershell
git add src-tauri/migrations/003_multi_account.sql src-tauri/src/lib.rs src-tauri/tests/storage_v3.rs src-tauri/tests/storage_v2.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add schema v3 multi-account storage"
```

### Task 2: 账号仓库与设备当前账号状态

**Files:**
- Create: `src-tauri/src/accounts.rs`
- Create: `src-tauri/src/device_settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/accounts.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: Task 1 的 `accounts` 表。
- Produces: `AccountView`、`AccountIdentity`、`create_pending_account`、`finalize_account_identity`、`list_accounts`、`archive_account`、`delete_account_data`、`AccountFolderDisposition`、`DeviceSettingsV1`。

- [ ] **Step 1: 写账号身份合并失败测试**

覆盖以下身份决策矩阵：

- 0 个命中：pending 行补全身份并变为 active。
- uid/sec_uid 同时命中同一 active 或 login_expired 账号：允许更新昵称、头像和登录时间；只允许用新值补全已有空 ID，不覆盖已有非空且不同的 ID。
- 仅一个字段命中：另一字段为空时补全；双方非空且不同则返回 `account_identity_conflict`。
- 两字段分别命中不同账号：返回 `account_identity_conflict`。
- 命中 archived 账号：返回 `account_archived_requires_restore`，不得自动恢复。
- 新账号流程命中已有账号：返回 `account_already_exists`，保留 pending 行与 profile，等待用户取消；不得自动替换已有账号 profile。
- pending 自身命中：正常完成。

昵称变化只更新展示字段，profile_key 永不随昵称变化。增加 `cancel_pending_account` 和 `restore_account` 测试；取消 pending 只允许删除受控 profiles 根内目录。

- [ ] **Step 2: 写设备设置原子文件测试**

固定 JSON 契约：

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceSettingsV1 {
    pub schema_version: i64,
    pub active_account_id: Option<i64>,
}
```

测试缺失文件返回默认值、未知字段拒绝、原子替换、active account 不存在时自动清空但不删除文件。

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test accounts`

Expected: FAIL，缺少 accounts/device_settings 模块。

- [ ] **Step 4: 实现账号仓库**

增加 `uuid = { version = "1", features = ["v4"] }`。`create_pending_account` 生成小写无连字符 UUID 作为 `profile_key`；`finalize_account_identity` 在立即事务中按上述矩阵处理 uid/sec_uid。普通 `resume` 必须复用已有 profile，不创建或替换 profile。所有身份字符串先 trim，uid/sec_uid 设置明确的 256 字节上限，昵称 200 字符上限，头像 URL 2048 字节上限且只接受 HTTP(S)，空昵称归一为 `None`。

- [ ] **Step 5: 实现设备状态与账号删除语义**

设备状态写入 `app_data_dir/device-settings.json`，使用现有同目录临时文件原子替换。`archive_account` 将状态改为 archived 并保留归档。`delete_account_data` 必须接收：

```rust
pub enum AccountFolderDisposition {
    Delete,
    ConvertToAggregate,
    TransferToAccount { target_account_id: i64 },
}
```

`local_folder_item_accounts` 只允许 aggregate scope 使用；account scope 的来源由 `owner_account_id` 推导。转换为 aggregate 时为全部现有条目写入原 owner provenance；转移账号时只更新 owner，不写 provenance；删除账号时删除该账号的 aggregate provenance，但只要 `local_folder_items` 仍存在就保留条目。仓库层必须拒绝向 account scope 写 provenance。

账号删除采用可恢复顺序：先计算 fallback active account；在同一事务中处理账号专属本地收藏夹、删除账号关系，并清理不再被 watch、likes、favorites、remote folders 或 local folders 引用的孤立 videos；提交数据库后再原子写设备设置。设置写失败返回 `device_settings_update_failed`，但不回滚已提交数据库；下次读取必须校验 active id 并自修复。转换或转移若与目标 scope 下已有同名收藏夹冲突则返回 `local_folder_name_conflict`。目标账号必须 active 且不能是被删除账号。数据库提交后才删除对应 profile 目录；删除前必须确认目录是受控 profiles 根的直接子目录。测试数据库已提交但设置写失败、当前账号归档、provenance 转换/转移/删除和目录清理失败。

- [ ] **Step 6: 运行测试并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test accounts
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/accounts.rs src-tauri/src/device_settings.rs src-tauri/src/lib.rs src-tauri/tests/accounts.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add account repository and active account state"
```

### Task 3: 账号隔离的浏览器状态与 Profile 路径

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/collector/browser_state.rs`
- Modify: `src-tauri/src/collector/cli.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/tests/browser_native.rs`
- Modify: `src-tauri/tests/runtime.rs`

**Interfaces:**
- Consumes: `AccountView.profile_key`。
- Produces: `AppPaths::account_root(profile_key)`、`account_browser_root(profile_key)`、collector CLI `--account-id` 与 `--account-profile-root`、`CollectorSuccessV3`。

- [ ] **Step 1: 写路径隔离失败测试**

断言账号 A/B 的 selection、login validation、profile、PID 和任务文件路径不相同；拒绝包含分隔符、`.`、`..` 或非小写 UUID 格式的 profile_key。

- [ ] **Step 2: 运行测试确认仍使用全局路径**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test runtime --test browser_native`

Expected: FAIL，现有路径仍为 `profiles/<browser>` 和全局 `browser-selection.json`。

- [ ] **Step 3: 修改 AppPaths 与 browser_state**

固定所有同步与登录布局：

```text
profiles/<profile_key>/browser-selection.json
profiles/<profile_key>/<browser>/.login-validated.json
profiles/<profile_key>/<browser>/User Data...
tmp/account-<account_id>-sync-<job_id>.result.json
tmp/account-<account_id>-sync-<job_id>.progress.json
tmp/account-<account_id>-sync-<job_id>.cancel
tmp/account-<account_id>-sync-<job_id>.browser.pid
diagnostics/account-<account_id>/sync-<job_id>.json
logs/account-<account_id>/sync-<job_id>.log
tmp/account-<account_id>-login-<attempt_id>.identity.json
tmp/account-<account_id>-login-<attempt_id>.cancel
tmp/account-<account_id>-login-<attempt_id>.browser.pid
diagnostics/account-<account_id>/login-<attempt_id>.json
logs/account-<account_id>/login-<attempt_id>.log
```

`browser_profile_path` 接收 account profile root；selection 和 validation 只能引用该 root 下的规范路径，跨账号路径返回 `browser_profile_scope_mismatch`。所有清理和恢复函数必须同时接收 accountId 与 jobId/attemptId，禁止仅凭数字文件名跨账号枚举。

- [ ] **Step 4: 修改 collector CLI 和调用构造**

collect 必须携带 `--account-id <i64>`、`--account-profile-root <path>` 和上述账号级任务文件。login 额外携带 `--login-attempt-id <uuid>`、`--identity-result <path>`、`--diagnostic <path>`、`--cancel-file <path>`、`--browser-pid-file <path>`。Tauri 只从数据库读取 profile_key 后构造路径，不接受前端直接提供路径。将同步结果升级为 `CollectorSuccessV3`：保留现有记录、范围和哈希字段，固定 `schemaVersion: 3` 并新增 `accountId`；记录身份哈希算法保持不变。进度和诊断同样包含 accountId，Rust 导入前再次与 sync_job.account_id 校验。

- [ ] **Step 5: 运行测试并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test runtime --test browser_native
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/lib.rs src-tauri/src/collector/browser_state.rs src-tauri/src/collector/cli.rs src-tauri/src/collector/runner.rs src-tauri/tests/browser_native.rs src-tauri/tests/runtime.rs
git commit -m "feat: isolate browser profiles by account"
```

### Task 4: 登录身份握手与账号 Tauri API

**Files:**
- Modify: `src-tauri/src/collector/contracts.rs`
- Modify: `src-tauri/src/collector/session.rs`
- Modify: `src-tauri/src/collector/runner.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/account_login.rs`

**Interfaces:**
- Consumes: Tasks 2-3 的 pending account 和 account profile root。
- Produces: `AccountIdentityV1`、`begin_account_login`、`resume_account_login`、`list_accounts`、`get_active_account`、`set_active_account`、`cancel_pending_account`、`restore_account`、`archive_account`、`delete_account` Tauri commands。

- [ ] **Step 1: 写登录身份契约失败测试**

固定 collector 输出：

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountIdentityV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub login_attempt_id: String,
    pub profile_key: String,
    pub uid: Option<String>,
    pub sec_uid: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub validated_at: String,
    pub history_url: String,
}
```

测试未知字段、空双 ID、错误 account_id/attempt_id/profile_key、非 Douyin self/history URL、非 HTTP(S) avatar 和过长身份字段均拒绝。

- [ ] **Step 2: 实现页面身份读取**

登录 collector 在用户进入已登录 self 页面后，优先读取页面已有账号 API/React 状态中的 uid、sec_uid、昵称和头像；不得从昵称推断身份。至少获得 uid/sec_uid 之一才原子写入任务级 identity result。

每次登录使用 UUID `login_attempt_id`。`begin/resume_account_login` 创建 attempt 文件后启动 collector，并校验 identity 中的 accountId、attemptId、profileKey 与任务参数完全一致。用户关闭窗口或 10 分钟超时：关闭 Job、删除不完整 identity、保留 pending/login_expired 状态并写 cancelled/timeout 诊断；应用退出：协作取消 2 秒后强制结束，启动恢复按 accountId+attemptId 清理；原子写 identity 失败：状态不变并返回 `identity_result_write_failed`。

- [ ] **Step 3: 实现 Tauri 命令**

- `begin_account_login()` 创建 pending account 与 attempt，启动对应 profile 的可见浏览器，等待 collector 完成，读取并校验 identity result，调用账号仓库完成身份，设置 active account，最终返回 `AccountView`。
- `resume_account_login(accountId)` 只接受 pending_login/login_expired，复用原 profile 并执行同一身份完成流程。
- `list_accounts()` 默认返回 active/login_expired，归档账号通过 includeArchived 参数读取。
- `get_active_account()` 读取并自修复 device settings，返回 `Option<AccountView>`。
- `set_active_account(accountId)` 只允许 active/login_expired 账号。
- `cancel_pending_account(accountId)` 只允许 pending_login，取消 attempt 后删除受控 profile。
- `restore_account(accountId)` 只允许 archived，恢复为 login_expired 并要求续登。
- `archive_account(accountId)` 保留归档；`delete_account(accountId, folderDisposition)` 删除数据并按 Task 2 的 disposition 处理账号专属本地收藏夹。

- [ ] **Step 4: 覆盖重复登录与冲突测试**

新账号流程发现已有远端账号时返回 `account_already_exists`，不替换已有 profile，也不删除 pending profile；用户可取消 pending 后使用已有账号的 `resume_account_login`。uid/sec_uid 分裂冲突不得删除任一已有或 pending profile。

- [ ] **Step 5: 运行测试并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test account_login
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/collector/contracts.rs src-tauri/src/collector/session.rs src-tauri/src/collector/runner.rs src-tauri/src/main.rs src-tauri/src/lib.rs src-tauri/tests/account_login.rs
git commit -m "feat: add multi-account login handshake"
```

### Task 5: 账号范围的观看历史同步、查询与删除

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/tests/storage_v3.rs`
- Modify: `src-tauri/tests/native_collector.rs`

**Interfaces:**
- Consumes: active account、account-specific collector invocation 和 v3 watch_records。
- Produces: 所有历史 API 的显式 `account_id` 范围，以及 account-aware sync jobs。

- [ ] **Step 1: 写跨账号隔离失败测试**

两个账号导入同一 aweme_id 和同一 history_time_ms：videos 总数为 1，watch_records 为 2；账号 A 查询/搜索/删除不得返回或修改 B；同账号重复事件仍 upsert。sync_job 的 account_id 与 collector result.accountId 不一致必须在事务前返回 `sync_account_mismatch`。

- [ ] **Step 2: 修改同步任务与导入事务**

本阶段明确只允许全局一个运行任务。`start_sync_job` 接收 account_id/source；当前阶段 source 固定 history。`start_sync(accountId, options)` 拒绝 pending/archived/login_expired，并返回包含 jobId/accountId 的结果。导入 SQL 的 watch identity 改为 `(account_id, video_id, history_time_ms)`，成功/失败更新条件包含 `id = ? AND account_id = ? AND status = 'running'`。

删除 Task 1 引入的旧前端临时兼容适配；生产入口已在 Task 1 切换为 v3。允许 `storage_v2` 继续直接调用 v2 初始化来构造迁移输入测试。

- [ ] **Step 3: 修改查询与删除 API**

```rust
pub fn list_history(
    connection: &Connection,
    account_id: i64,
    query: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<ListHistoryResult, String>;
```

delete by ids/query 同样必须携带 account_id，并在删除后仅清理不再被 watch、likes、favorites、remote folders 或 local folders 引用的 videos。前端不能通过记录 id 删除其他账号数据。测试分别建立每一种未来来源引用，确认删除观看记录不会误删仍被任一来源引用的视频；全部引用删除后才清理孤立视频。

- [ ] **Step 4: 修改恢复与取消**

启动恢复按 job.account_id 清理对应任务文件和账号浏览器进程。`get_sync_status(accountId, jobId)` 和 `cancel_sync(accountId, jobId)` 都要求精确目标；目标与全局运行任务不一致返回 `sync_job_mismatch`，不得取消其他账号任务。状态返回 accountId/jobId/source。增加切换账号后查询、取消错误 job、恢复其他账号 running job 和数据库条件更新测试。

- [ ] **Step 5: 运行测试并提交**

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test storage_v3 --test native_collector
cargo test --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/lib.rs src-tauri/src/main.rs src-tauri/tests/storage_v3.rs src-tauri/tests/native_collector.rs
git commit -m "feat: scope history sync and queries by account"
```

### Task 6: 最小账号切换 UI 与端到端门禁

**Files:**
- Modify: `app/src/types.ts`
- Modify: `app/src/app-api.ts`
- Modify: `app/src/App.svelte`
- Modify: `app/src/styles.css`
- Create: `tests/app/accounts.test.ts`
- Modify: `docs/stage-three-usable-version-roadmap.md`

**Interfaces:**
- Consumes: Task 4 的账号 commands 和 Task 5 的 account-aware history APIs。
- Produces: 最小账号选择器、添加账号入口、账号状态和当前账号范围的现有历史页面。

- [ ] **Step 1: 写前端类型与状态转换失败测试**

定义：

```ts
export interface AccountView {
  id: number;
  uid?: string;
  secUid?: string;
  profileKey: string;
  nickname?: string;
  avatarUrl?: string;
  status: "pending_login" | "active" | "login_expired" | "archived";
  lastLoginValidatedAt?: string;
  createdAt: string;
  updatedAt: string;
}
```

测试账号列表排序（active account first，其余按 createdAt）、无账号空状态、切换时清空选择并回到第一页、登录失效状态文案。

- [ ] **Step 2: 增加 app-api wrappers**

新增 `listAccounts`、`getActiveAccount`、`beginAccountLogin`、`resumeAccountLogin`、`cancelPendingAccount`、`restoreAccount`、`setActiveAccount`、`archiveAccount`、`deleteAccount`。删除旧 `openLoginPage` wrapper；登录统一由账号命令驱动。`listHistory/startSync/delete` 显式传 accountId，`getSyncStatus/cancelSync` 显式传 accountId+jobId。Web demo 使用两个固定假账号验证切换，不模拟登录成功。

为每个 wrapper 增加 IPC 序列化测试，精确断言 camelCase 前端字段映射为 Tauri command 参数，防止 accountId/jobId/folderDisposition 丢失。

- [ ] **Step 3: 实现最小账号 UI**

在现有 topbar 增加紧凑账号菜单：头像/昵称/状态、切换账号、添加账号、管理账号。无账号时主内容显示添加账号空状态；切换后刷新历史与同步状态。此任务不实施最终侧边导航或 UI 美化，只建立可用交互。

- [ ] **Step 4: 更新 dist 与 manifest**

运行 `npm run app:build`，提交 Git 跟踪的 `dist/index.html`、`dist/assets/**` 和 `dist/build-manifest.json`；Cargo manifest 校验必须通过。

- [ ] **Step 5: 运行自动门禁**

```powershell
npm test
npm run typecheck
npm run app:build
cargo test --manifest-path src-tauri/Cargo.toml
cargo tauri build
```

Expected: 全部退出码 0；正式安装包仍不包含 Node/Playwright/浏览器二进制。

- [ ] **Step 6: 真实账号验收**

使用两个测试账号分别登录并同步固定一天范围，验证：profile 独立、切换后列表正确、相同视频公共元数据复用、删除 A 不影响 B、重启后 active account 保留、隐藏同步不打开其他账号 profile。

- [ ] **Step 7: 更新路线图并提交**

在路线图记录 schema v3 与多账号基础完成状态及测试结果，不提前标记喜欢/收藏采集完成。

```powershell
git add app/src/types.ts app/src/app-api.ts app/src/App.svelte app/src/styles.css tests/app/accounts.test.ts dist docs/stage-three-usable-version-roadmap.md
git commit -m "feat: add multi-account switching foundation"
git status --short --branch
```

## Final Verification

- [ ] `PRAGMA user_version = 3`，旧开发数据和旧全局 profile 不迁移。
- [ ] 两个账号共享 videos，但所有事件、关系、任务、查询和删除均按 account_id 隔离。
- [ ] uid/sec_uid 身份冲突不静默合并，昵称变化不创建新账号。
- [ ] browser selection、login validation、PID、结果和诊断均位于账号范围。
- [ ] 喜欢、收藏、远端/本地收藏夹 schema 已存在，但未伪装为已实现采集。
- [ ] 当前账号为设备状态，不被未来跨设备备份设置覆盖。
- [ ] 全部 Rust、前端、manifest、Tauri bundle 和双账号实页验收通过。
