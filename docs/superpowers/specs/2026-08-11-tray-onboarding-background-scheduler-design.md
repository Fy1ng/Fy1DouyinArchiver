# 托盘、首次引导与后台调度设计

## 1. 目标

将当前以主窗口和手动同步为中心的桌面应用扩展为可长期低打扰常驻的 Windows 当前用户应用：

- 首次启动通过界面内引导完成账号、同步范围、自动同步、开机启动和关闭行为设置。
- 主窗口关闭时可按设置退出或隐藏到托盘，手动启动始终显示主窗口。
- 开机启动可选择显示主窗口或静默进入托盘。
- 所有手动和自动同步进入同一持久任务队列，手动任务优先且同一完整 task key 不重复排入等价任务。
- 自动同步按上一次任务终态时间加用户间隔计算；睡眠恢复后每个账号、来源最多补一个任务。
- 真正退出时停止调度、取消采集、清理浏览器和任务文件后再结束进程。

本阶段完成观看历史的完整调度链，并为观看历史、喜欢、收藏建立不同的范围与周期设置契约。喜欢和收藏页面采集、远端移除识别及收藏夹归档仍由 3.4 实现；对应执行器注册前，后端能力表返回 `available = false`，不得排入任务。3.3 的多来源最终验收与 3.4 执行器验收共同关闭，不能用仅有观看历史的结果宣称多来源调度完成。

## 2. 非目标

- 不实现喜欢、收藏和远端收藏夹采集。
- 不实现封面缓存、备份恢复、统计分析、视频下载或应用内更新。
- 不重新设计历史列表和整体视觉系统；新增界面沿用当前 Svelte 控件与样式，完整 UI 美化留在 3.6。
- 不修改 NSIS 品牌资源、安装流程视觉或安装包文案；安装包美化由用户下一轮确认。
- 不使用 Windows 任务计划程序、服务、管理员权限或机器级注册表。
- 不允许应用退出后由独立后台进程继续同步。

## 3. 已确认决策

### 3.1 调度时间语义

自动同步间隔以最近一次同一完整 task key 任务的 `finished_at` 为锚点。设置为 6 小时且任务在 10:20 结束时，下次到期时间为 16:20。

- 没有历史终态任务时，以本次应用就绪时间加“开机后首次同步延迟”计算。
- partial、失败和取消也是终态，下一次自动同步仍按配置间隔计算，不立即重试。
- 系统睡眠或应用线程长时间未调度时，只排入当前到期任务，不按错过的间隔次数补跑。
- 用户修改间隔后立即重新计算，但不会中断正在运行的任务。

### 3.2 桌面运行方式

- 正常双击、开始菜单或第二次手动启动始终显示并聚焦主窗口。
- 仅带内部参数 `--autostart` 的启动被视为开机启动。
- 开机启动且“静默启动”开启时隐藏主窗口，但首次引导未完成或托盘初始化失败时必须显示主窗口。
- 当前用户开机启动使用 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名固定为 `LifeArchiveProject.Emily`，值为带引号的当前 EXE 绝对路径和 `--autostart`。
- 不请求 UAC，不写入 HKLM，不创建计划任务。

### 3.3 单实例与退出

- collector 和卸载 helper 的进程模式继续在 GUI 单实例判断前分流。
- GUI 继续使用已有命名 mutex，并新增同作用域的自动重置激活事件。
- 第二个手动 GUI 进程发现 mutex 已占用时只触发激活事件并退出；第二个 `--autostart` 进程静默退出，不唤起窗口。
- 首进程监听激活事件，收到后恢复、显示并聚焦主窗口。
- “退出”是独立的受控关闭流程；普通窗口关闭事件不得直接绕过设置和清理。

## 4. 设置与文件边界

### 4.1 Portable settings schema v2

`<data_root>/settings.json` 升级到 schema version 2。它仍属于可备份、可跨设备合并的便携设置。

```rust
enum CloseBehaviorV1 {
    Tray,
    Exit,
}

struct GlobalSettingsV2 {
    like_count_format: LikeCountFormatV1,
    detail_fields: DetailFieldsV1,
    close_behavior: CloseBehaviorV1,
    startup_sync_delay_minutes: u32,
}

struct SourceScheduleV1 {
    enabled: bool,
    interval_minutes: u32,
}

#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum HistoryRangeV1 {
    Days { days: u32 },
    Bottom,
}

#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ArchiveListRangeV1 {
    All,
    RecentItems { count: u32 },
    RecordLimit { limit: u32 },
    RecentDays { days: u32 },
}

struct HistorySourceSettingsV1 {
    range: HistoryRangeV1,
    schedule: SourceScheduleV1,
}

struct ArchiveSourceSettingsV1 {
    range: ArchiveListRangeV1,
    schedule: SourceScheduleV1,
}

enum RemoteFolderScheduleModeV1 {
    Unified,
    PerFolder,
}

struct FavoriteSourceSettingsV1 {
    archive: ArchiveSourceSettingsV1,
    remote_folder_mode: RemoteFolderScheduleModeV1,
    remote_folder_defaults: ArchiveSourceSettingsV1,
    remote_folders: BTreeMap<String, ArchiveSourceSettingsV1>,
}

struct AccountSettingsV2 {
    hide_browser: bool,
    scroll_speed: u32,
    history: HistorySourceSettingsV1,
    likes: ArchiveSourceSettingsV1,
    favorites: FavoriteSourceSettingsV1,
}
```

约束：

- `startup_sync_delay_minutes` 范围为 0 到 120，默认 10。
- history、likes、favorites archive、remote folder defaults 和逐收藏夹的 `schedule.enabled` 均默认 `false`，自动同步必须由用户明确开启。
- 所有 `schedule.interval_minutes` 范围为 15 到 43,200，默认 360。
- `HistoryRangeV1::Days.days` 范围为 1 到 365。
- `ArchiveListRangeV1::RecentItems.count` 范围为 1 到 5,000；`RecordLimit.limit` 范围为 1 到 50,000。
- `ArchiveListRangeV1::RecentDays.days` 范围为 1 到 365，只有 capability 声明存在可靠远端时间字段时才可保存或入队；否则返回 `time_range_not_supported`。
- 旧 schema v1 原子迁移到 v2，原 `sync_mode/sync_days` 转换为 `history.range`，保留其他已有字段；旧用户的 `close_behavior` 默认 `Exit`，避免升级后突然常驻后台。
- `remote_folders` key 必须是 1 到 256 字节的规范远端收藏夹 ID，最多保存 1,000 项；`Unified` 使用 defaults，`PerFolder` 使用逐项设置并对新发现收藏夹回退 defaults。
- `account_defaults` 和每个账号都迁移全部来源设置；喜欢、收藏和远端收藏夹在执行器可用前即使文件中为 enabled，也由能力门禁强制不排队并在 UI 显示不可用状态。
- schema v2 继续拒绝未知字段、非法枚举、越界数字和不安全账号 key。

### 4.2 Device settings schema v1

`<system_app_data>/device-settings.json` 保存不可直接随数据库备份覆盖的本机行为：

```rust
struct DeviceSettingsV1 {
    schema_version: i64,
    revision: u64,
    onboarding_completed: bool,
    onboarding_step: u8,
    launch_at_startup: bool,
    silent_autostart: bool,
    scheduler_paused: bool,
    first_tray_hint_shown: bool,
    notified_failure_streaks: BTreeMap<String, i64>,
    accounts: BTreeMap<String, DeviceAccountSettingsV1>,
}

enum BrowserPreferenceV1 {
    Auto,
    Edge,
    Chrome,
}

struct DeviceAccountSettingsV1 {
    browser_preference: BrowserPreferenceV1,
}
```

- 文件与 bootstrap 位于同一系统应用数据目录，使用同目录临时文件和原子替换。
- `onboarding_step` 范围为 0 到 3；完成后固定为 3。
- `silent_autostart` 仅在 `launch_at_startup = true` 时生效。
- `notified_failure_streaks` 的 key 固定为 `<account_id>:<source>:<source_instance_key_sha256>`，value 是触发过本轮连续失败通知的最新 job ID；空 instance 也使用空字符串的 SHA-256，禁止拼接原远端收藏夹名称或 ID 到通知状态文件。
- `accounts` key 固定为数据库 `profile_key` 的小写 simple UUID，不使用可变昵称或远端账号 ID；浏览器偏好默认 Auto。
- 新账号登录请求可携带 `browserPreference`，collector 仍按 Edge→Chrome 发现并在成功后锁定实际浏览器。已有账号切换到不同浏览器必须先确认重新登录，使用该浏览器独立 profile，不能迁移 Cookie 或静默切换。
- 每个 `<profile_key>/<browser>` 目录持久保存独立 `profile_generation`；浏览器 selection/login marker 必须携带相同 generation。显式重置 profile 先生成新目录和 generation，再原子切换，不复用旧 queued payload。
- 更新开机启动时先操作并回读注册表，成功后才写 device settings；失败时文件保持原值。
- 应用启动时若开机启动已开启但注册表中的 EXE 路径过期，则按当前 EXE 修复；修复失败不阻止手动启动，只显示设置错误。

### 4.3 DeviceSettingsStore 与跨资源提交

`DeviceSettingsStore` 由 `AppState` 单例持有，内部使用一个 RMW mutex 和 revision CAS；所有引导进度、暂停状态、托盘提示、通知 streak 和开机启动字段更新都必须经过该 store。原子替换只负责文件完整性，mutex/CAS 负责防止并发更新丢字段。

- 普通 device settings 更新只持有 store 锁，不得同时持有队列、数据库、runtime path 或浏览器操作锁。portable source settings、autostart 和浏览器偏好/profile reset 等跨资源更新必须注册有界 `OwnedAdmissionOperation`，包含 operation ID、类型、completion handle 和 cancellation token；owner 在 admission 变为 pending 后仍可凭该 token 完成或补偿自身提交，但不能进入其他资源操作。
- 浏览器偏好/profile reset 先在 `OperationAdmission` 下设置 account browser update gate；该账号存在 queued/running job、登录或 browser lease 时返回 `account_browser_operation_in_progress`，不修改文件。无活动操作时注册 owned operation、释放 admission 后更新 device/profile state，成功或补偿完成后再清除 gate并唤醒 scheduler。
- maintenance 发现任何 active source/device/browser update gate 时立即返回 `settings_update_in_progress`，不得进入 pending 或等待 path lease。exit 可切换到 `ExitPending`，但必须在 marker 线性化点前锁外取消并有界等待全部 owned update operations 完成。
- 开机启动更新使用独立 `desktop_settings_tx` mutex。顺序固定为读取旧注册表与旧 device settings、写注册表、原子写 device settings；最后一步失败时必须把注册表恢复为原值。
- 注册表补偿失败返回 `autostart_state_uncertain`，启动时执行双向核对：device settings 为 enabled 时确保值等于当前 EXE 命令，为 disabled 时删除本应用拥有的固定值。
- 引导每步先提交 portable/account defaults，再写 device progress；任一步失败都不推进 `onboarding_step`。重复提交必须幂等。
- 新增 `update_account_defaults` 后端接口，跳过账号时不得伪造账号 ID 调用账号设置接口。

### 4.4 首次引导兼容

没有 device settings 时按以下顺序判定：

1. 数据库存在账号、观看记录或完成过的同步任务，视为升级用户，直接生成 `onboarding_completed = true`。
2. portable settings revision 大于 0，视为已使用用户，直接完成引导。
3. 其余情况视为全新安装，显示引导。

该判定只执行一次并持久化，升级不得重复展示引导。

## 5. 持久同步队列

### 5.1 schema v5

新增 `005_sync_queue.sql`，重建 `sync_jobs`，新增 batch/target/issue、queue state 和 remote-folder registry state，不清空账号、视频或历史记录。

```sql
CREATE TABLE sync_batches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  trigger TEXT NOT NULL CHECK(trigger = 'manual'),
  scope TEXT NOT NULL CHECK(scope IN ('currentAccountAvailableSources', 'allActiveAccounts')),
  created_at TEXT NOT NULL,
  finished_at TEXT,
  next_entry_seq INTEGER NOT NULL DEFAULT 1 CHECK(next_entry_seq BETWEEN 1 AND 9223372036854775807),
  status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'success', 'partial', 'failed', 'cancelled')),
  CHECK((status IN ('queued', 'running') AND finished_at IS NULL) OR
        (status IN ('success', 'partial', 'failed', 'cancelled') AND finished_at IS NOT NULL))
);

CREATE TABLE sync_batch_targets (
  batch_id INTEGER NOT NULL REFERENCES sync_batches(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  target_kind TEXT NOT NULL CHECK(target_kind IN ('task', 'aggregate')),
  resolution_kind TEXT NOT NULL CHECK(resolution_kind IN ('job', 'issue')),
  PRIMARY KEY(batch_id, account_id, source, source_instance_key),
  UNIQUE(batch_id, account_id, source, source_instance_key, resolution_kind),
  CHECK(target_kind = 'task' OR
        (target_kind = 'aggregate' AND resolution_kind = 'issue' AND
         source = 'remote_folder' AND source_instance_key = ''))
);

CREATE TABLE sync_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  trigger TEXT NOT NULL CHECK(trigger IN ('manual', 'automatic')),
  queued_at TEXT NOT NULL,
  queue_seq INTEGER,
  scheduled_for TEXT,
  settings_revision INTEGER,
  config_sha256 TEXT,
  payload_json TEXT,
  payload_sha256 TEXT,
  equivalence_sha256 TEXT,
  started_at TEXT,
  finished_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'success', 'partial', 'failed', 'cancelled')),
  inserted_videos INTEGER NOT NULL DEFAULT 0,
  inserted_records INTEGER NOT NULL DEFAULT 0,
  error_code TEXT,
  error_message TEXT,
  CHECK((status IN ('queued', 'running') AND finished_at IS NULL) OR
        (status IN ('success', 'partial', 'failed', 'cancelled') AND finished_at IS NOT NULL)),
  CHECK(status NOT IN ('queued', 'running') OR
        (queue_seq IS NOT NULL AND settings_revision IS NOT NULL AND
         payload_json IS NOT NULL AND payload_sha256 IS NOT NULL AND
         equivalence_sha256 IS NOT NULL)),
  CHECK(status NOT IN ('queued', 'running') OR
        (trigger = 'manual' AND config_sha256 IS NULL) OR
        (trigger = 'automatic' AND config_sha256 IS NOT NULL))
);

CREATE TABLE sync_queue_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  next_queue_seq INTEGER NOT NULL CHECK(next_queue_seq BETWEEN 1 AND 9223372036854775807)
);

CREATE TABLE remote_folder_registry_state (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9223372036854775807)
);

INSERT INTO remote_folder_registry_state(account_id, revision)
SELECT id, 1 FROM accounts;

CREATE INDEX idx_remote_folders_scheduler
  ON remote_folders(account_id, remote_removed_at, remote_folder_id);

CREATE TABLE sync_batch_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id INTEGER NOT NULL REFERENCES sync_batches(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  resolution_kind TEXT NOT NULL DEFAULT 'job' CHECK(resolution_kind = 'job'),
  job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  entry_seq INTEGER NOT NULL,
  job_identity_sha256 TEXT NOT NULL,
  job_view_snapshot_json TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('root', 'discoveredChild', 'attached')),
  UNIQUE(batch_id, account_id, source, source_instance_key),
  UNIQUE(batch_id, job_identity_sha256),
  UNIQUE(batch_id, entry_seq),
  FOREIGN KEY(batch_id, account_id, source, source_instance_key, resolution_kind)
    REFERENCES sync_batch_targets(batch_id, account_id, source, source_instance_key, resolution_kind)
    ON DELETE CASCADE
);

CREATE TABLE sync_job_issues (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id INTEGER NOT NULL REFERENCES sync_jobs(id) ON DELETE CASCADE,
  phase TEXT NOT NULL,
  code TEXT NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('warning', 'incomplete')),
  message TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(job_id, phase, code)
);

CREATE TABLE sync_batch_issues (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id INTEGER NOT NULL REFERENCES sync_batches(id) ON DELETE CASCADE,
  entry_seq INTEGER NOT NULL,
  account_id INTEGER NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('history', 'likes', 'favorites', 'remote_folder')),
  source_instance_key TEXT NOT NULL,
  resolution_kind TEXT NOT NULL DEFAULT 'issue' CHECK(resolution_kind = 'issue'),
  phase TEXT NOT NULL CHECK(phase IN ('initialExpansion', 'discoveryExpansion')),
  kind TEXT NOT NULL CHECK(kind IN ('skipped', 'conflict')),
  reason TEXT NOT NULL,
  requested_payload_sha256 TEXT,
  existing_payload_sha256 TEXT,
  existing_job_id INTEGER REFERENCES sync_jobs(id) ON DELETE SET NULL,
  conflicting_job_snapshot_json TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(batch_id, account_id, source, source_instance_key),
  UNIQUE(batch_id, entry_seq),
  CHECK((kind = 'skipped' AND existing_job_id IS NULL AND conflicting_job_snapshot_json IS NULL AND
         existing_payload_sha256 IS NULL) OR
        (kind = 'conflict' AND requested_payload_sha256 IS NOT NULL AND
         existing_payload_sha256 IS NOT NULL AND conflicting_job_snapshot_json IS NOT NULL)),
  FOREIGN KEY(batch_id, account_id, source, source_instance_key, resolution_kind)
    REFERENCES sync_batch_targets(batch_id, account_id, source, source_instance_key, resolution_kind)
    ON DELETE CASCADE
);

CREATE TRIGGER reject_terminal_batch_job
BEFORE INSERT ON sync_batch_jobs
WHEN (SELECT status FROM sync_batches WHERE id = NEW.batch_id)
     IN ('success', 'partial', 'failed', 'cancelled')
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

CREATE TRIGGER reject_terminal_batch_target
BEFORE INSERT ON sync_batch_targets
WHEN (SELECT status FROM sync_batches WHERE id = NEW.batch_id)
     IN ('success', 'partial', 'failed', 'cancelled')
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

CREATE TRIGGER reject_batch_target_update
BEFORE UPDATE ON sync_batch_targets
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_target_immutable');
END;

CREATE TRIGGER reject_direct_batch_target_delete
BEFORE DELETE ON sync_batch_targets
WHEN EXISTS (SELECT 1 FROM sync_batches WHERE id = OLD.batch_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_target_delete_forbidden');
END;

CREATE TRIGGER reject_sync_job_task_key_update
BEFORE UPDATE OF account_id, source, source_instance_key ON sync_jobs
WHEN NEW.account_id <> OLD.account_id
  OR NEW.source <> OLD.source
  OR NEW.source_instance_key <> OLD.source_instance_key
BEGIN
  SELECT RAISE(ABORT, 'sync_job_task_key_immutable');
END;

CREATE TRIGGER reject_initial_null_batch_job
BEFORE INSERT ON sync_batch_jobs
WHEN NEW.job_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_required');
END;

CREATE TRIGGER reject_batch_job_task_key_mismatch
BEFORE INSERT ON sync_batch_jobs
WHEN NEW.job_id IS NOT NULL AND NOT EXISTS (
  SELECT 1 FROM sync_jobs j
  WHERE j.id = NEW.job_id
    AND j.account_id = NEW.account_id
    AND j.source = NEW.source
    AND j.source_instance_key = NEW.source_instance_key
    AND j.status IN ('queued', 'running')
)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_task_key_mismatch');
END;

CREATE TRIGGER reject_batch_job_task_key_update
BEFORE UPDATE OF account_id, source, source_instance_key ON sync_batch_jobs
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_task_key_immutable');
END;

CREATE TRIGGER reject_batch_job_identity_update
BEFORE UPDATE OF batch_id, entry_seq, role, resolution_kind,
                 job_identity_sha256
ON sync_batch_jobs
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_identity_immutable');
END;

CREATE TRIGGER reject_batch_job_rebind
BEFORE UPDATE OF job_id ON sync_batch_jobs
WHEN NEW.job_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_rebind_forbidden');
END;

CREATE TRIGGER reject_batch_job_manual_unlink
BEFORE UPDATE OF job_id ON sync_batch_jobs
WHEN NEW.job_id IS NULL
  AND OLD.job_id IS NOT NULL
  AND EXISTS (SELECT 1 FROM sync_jobs WHERE id = OLD.job_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_manual_unlink_forbidden');
END;

CREATE TRIGGER reject_direct_batch_job_delete
BEFORE DELETE ON sync_batch_jobs
WHEN EXISTS (SELECT 1 FROM sync_batches WHERE id = OLD.batch_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_job_delete_forbidden');
END;

CREATE TRIGGER reject_terminal_batch_issue
BEFORE INSERT ON sync_batch_issues
WHEN (SELECT status FROM sync_batches WHERE id = NEW.batch_id)
     IN ('success', 'partial', 'failed', 'cancelled')
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;

CREATE TRIGGER reject_conflict_issue_job_identity
BEFORE INSERT ON sync_batch_issues
WHEN NEW.kind = 'conflict' AND (
  NEW.existing_job_id IS NULL OR NOT EXISTS (
    SELECT 1 FROM sync_jobs j
    WHERE j.id = NEW.existing_job_id
      AND j.account_id = NEW.account_id
      AND j.source = NEW.source
      AND j.source_instance_key = NEW.source_instance_key
      AND j.status IN ('queued', 'running')
  )
)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_conflict_job_identity_mismatch');
END;

CREATE TRIGGER reject_batch_issue_evidence_update
BEFORE UPDATE OF batch_id, account_id, source, source_instance_key, resolution_kind,
                 phase, kind, reason, requested_payload_sha256,
                 existing_payload_sha256, conflicting_job_snapshot_json,
                 entry_seq, created_at
ON sync_batch_issues
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_evidence_immutable');
END;

CREATE TRIGGER reject_batch_issue_job_rebind
BEFORE UPDATE OF existing_job_id ON sync_batch_issues
WHEN NEW.existing_job_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_job_rebind_forbidden');
END;

CREATE TRIGGER reject_batch_issue_manual_unlink
BEFORE UPDATE OF existing_job_id ON sync_batch_issues
WHEN NEW.existing_job_id IS NULL
  AND OLD.existing_job_id IS NOT NULL
  AND EXISTS (SELECT 1 FROM sync_jobs WHERE id = OLD.existing_job_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_manual_unlink_forbidden');
END;

CREATE TRIGGER reject_direct_batch_issue_delete
BEFORE DELETE ON sync_batch_issues
WHEN EXISTS (SELECT 1 FROM sync_batches WHERE id = OLD.batch_id)
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_issue_delete_forbidden');
END;

CREATE TRIGGER reject_terminal_batch_reopen
BEFORE UPDATE OF status ON sync_batches
WHEN OLD.status IN ('success', 'partial', 'failed', 'cancelled')
     AND NEW.status <> OLD.status
BEGIN
  SELECT RAISE(ABORT, 'sync_batch_terminal');
END;
```

- 旧 terminal 任务迁移时 `trigger = 'manual'`、`queued_at = started_at`、`queue_seq = NULL`、`scheduled_for = NULL`、`source_instance_key = ''`，五个 payload/config/equivalence 字段为 `NULL`；它们只用于历史审计和时间锚点，绝不重新执行，不能从当前设置伪造旧参数。
- 旧 `running` 任务在迁移事务中安全终态化为 failed，`finished_at` 使用迁移冻结时间，错误码为 `schema_upgrade_interrupted_running`，五个 payload/config/equivalence 字段为 `NULL`；迁移提交后按旧 orchestration/PID 身份执行 collector/browser 清理。
- history、likes、favorites 的 `source_instance_key` 固定为空字符串；remote-folder 固定为规范远端收藏夹 ID，必须与 payload 的 `remote_folder_id` 完全一致。
- 增加 `(account_id, source, source_instance_key)` 上针对 `queued/running` 的 partial unique index。
- 增加常量表达式上的 `WHERE status = 'running'` 唯一索引，数据库级保证全局最多一个 running job；应用事务仍执行同样检查并返回稳定错误。
- 新增按 `status, trigger, queue_seq, id` 的队列索引，以及 `(account_id, source, source_instance_key, finished_at DESC, id DESC)` 的完整 task-key 终态锚点索引。
- `user_version` 升级为 5；迁移失败完整回滚。
- 迁移必须在所有连接启动前执行：保存并关闭 `foreign_keys`，使用立即事务创建 `sync_jobs_v5_new`，按原 ID 复制并补齐旧 terminal 缺失的 `finished_at`，直接 drop 原 `sync_jobs`，再把新表 rename 为 `sync_jobs`；禁止先 rename 旧表，避免 SQLite 把 `watch_records` 等外键改指向临时表。随后创建 batch/target/issue、queue/registry state 表，恢复 `sqlite_sequence` 并重建索引/trigger。
- `sync_queue_state` 在迁移时初始化为 1；所有新 job 在立即事务中取出并递增 `next_queue_seq`，溢出时返回 `sync_queue_sequence_exhausted`。`remote_folder_registry_state` 为每个既有账号初始化 revision=1，新账号必须在创建账号的同一事务插入对应状态行；任一 registry 写事务先检查 revision 未达到 i64::MAX，再与 folder 变更一起递增，否则整个事务返回 `remote_folder_registry_revision_exhausted`。提交前必须检查 `sqlite_master` 和 `PRAGMA foreign_key_list`，断言所有原引用仍指向最终名称 `sync_jobs`，再执行 `PRAGMA foreign_key_check`；全部通过后才设置 `user_version = 5` 并提交。事务失败恢复原 schema 和原 `user_version`；事务结束后无论成功失败都恢复连接原 `foreign_keys` 设置并再次断言。
- `watch_records.sync_job_id` 等现有外键依赖原 ID，不允许通过重新编号后补写映射。
- 旧任务不创建 batch；新单任务可无 batch。manual `CurrentAccountAvailableSources` 和 `AllActiveAccounts` 必须创建 batch，所有初始、附着和后续发现任务通过 `sync_batch_jobs` 关联，同一 active job 可以被多个 batch 引用。
- `sync_batch_targets` 是 batch 内 key 的唯一事实源。每个初始目标、动态 discovery 目标或 source-level aggregate issue 都必须在产生 job association/issue 的同一事务先插入一行；`PRIMARY KEY(batch_id, account_id, source, source_instance_key)` 保证一个 batch 对同一 key 只决策一次，`target_kind` 区分可执行 task 与不计入 target 上限的 aggregate，`resolution_kind` 冻结该 key 最终写入 job 还是 issue。aggregate 只允许 RemoteFolder 空 instance + issue，不代表可执行 task key。终态 batch 禁止新增 target；target 写入后不可更新或直接删除，只能在父 batch 已删除时随 FK cascade 清理。
- `sync_jobs` 的 account/source/instance 从创建后永久不可变，manual promotion 只能修改 trigger、queue/payload/config 字段。`sync_batch_jobs` 对相同四元组再加 UNIQUE；插入时 trigger 要求非空 active job 且其真实 task key 完全匹配，batch ID、entry_seq、role、resolution kind、job identity hash、关联 key 和非空 job ID 此后不可重挂或重排，只有 snapshot 可随 job 状态更新，父 job 删除时 FK 才允许把 job_id 置 null并保留 snapshot。association 不能直接删除，只能随父 batch/target cascade 清理。
- conflict issue 插入时必须携带同一完整 task key 的 active `existing_job_id`；skip 固定无 existing job。issue 的 batch/key、kind、phase、reason、payload hashes、冲突 snapshot、entry_seq 和 created_at 一经写入不可修改，existing job 不能重绑，只有父 job 删除时 FK 可置 null。issue 不能直接删除，只能随父 batch/target cascade 清理；不可变 snapshot 始终保留冲突时身份与 payload 证据。
- `sync_batch_jobs.job_view_snapshot_json` 保存最新规范 `SyncJobViewV1`，关联时同时保存 target 的 account/source/instance；job 每次状态变化时在同一事务更新所有关联快照。`job_identity_sha256` 固定覆盖 job ID、task key、trigger 和 payload hash。查询时优先加载 live job，job 被账号级联删除后使用快照，禁止把历史 batch 变成空列表。
- batch 每次新增 job association 或 issue 时在同一事务取用并递增 `sync_batches.next_entry_seq`，把所得值写入对应表的 `entry_seq`；两张表共享该单调序列，entry 永不删除或重排。序号溢出返回 `sync_batch_entry_sequence_exhausted`，不得追加。
- `sync_batch_issues` 持久保存批量请求的 skip/conflict，不能只存在于首次 IPC 返回。conflict 同时保存请求和现有 payload hash，以及冲突发生当时的不可变 `conflicting_job_snapshot_json`；IPC 字段明确命名为 `existingJobAtConflict`，无论 live job 后续怎样变化都返回该快照，不混用“当前状态”语义。issue 按 account、source、instance、phase、id 稳定排序。
- `sync_job_issues` 保存执行完成时的非致命不完整状态；存在 `severity = incomplete` 时 job 终态必须为 `partial`，关联 batch 也按 partial 汇总。单任务、automatic 和批量任务都通过同一 IPC 暴露 issue，不能只在 batch 中记录。
- schema v5 只有两个批量 manual scope 会创建 batch；`scope = One` 始终返回 `batch_id = NULL`，automatic scheduler 也不创建 batch。`AllActiveAccounts` 没有 active 账号时直接返回 `no_active_accounts`，不创建无法归属 account 的空 batch。
- batch 事务每次新增关联 job、记录 blocking issue 或完成关联 job 后都重新计算持久状态：存在 running 为 `running`；无 running 但存在 queued 为 `queued`；全部关联 job success 且无 batch issue/`incomplete` job issue 为 `success`；全部关联 job cancelled 且无 batch issue/`incomplete` 为 `cancelled`；存在 success/partial 且同时存在 partial/failed/cancelled、batch skip/conflict 或 `incomplete` 为 `partial`；没有 success/partial 且存在 failed、batch issue 或 `incomplete` 为 `failed`。`warning` 只展示，不改变 job/batch 终态。终态时写 `finished_at`，数据库 trigger 和应用检查共同禁止再追加 child job/issue。
- 账号删除前必须先按现有规则取消 queued、拒绝 running，再更新所有 batch job 快照；删除账号/job 后在同一事务按保留快照重算受影响 batch，不能删除 batch 审计证据。

### 5.2 任务参数快照与 IPC

每个 queued job 必须保存可独立执行的不可变 payload，不能在真正启动时重新读取当前 UI 状态：

```rust
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteFolderExecutionOverrideV1 {
    range: ArchiveListRangeV1,
}

enum BrowserIdV1 {
    Edge,
    Chrome,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserExecutionIdentityV1 {
    preference: BrowserPreferenceV1,
    resolved_browser: BrowserIdV1,
    profile_key: String,
    profile_generation: String,
    executable_path_sha256: String,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RangeFreezeV1 {
    frozen_at_ms: i64,
    cutoff_ms: Option<i64>,
}

#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
enum SyncTaskPayloadV1 {
    History {
        range: HistoryRangeV1,
        range_freeze: RangeFreezeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Likes {
        range: ArchiveListRangeV1,
        range_freeze: RangeFreezeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Favorites {
        range: ArchiveListRangeV1,
        range_freeze: RangeFreezeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
        discover_remote_folders: bool,
        remote_folder_mode: RemoteFolderScheduleModeV1,
        remote_folder_default_range: ArchiveListRangeV1,
        remote_folder_overrides: BTreeMap<String, RemoteFolderExecutionOverrideV1>,
    },
    RemoteFolder {
        remote_folder_id: String,
        range: ArchiveListRangeV1,
        range_freeze: RangeFreezeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EffectiveRangeEquivalenceV1 {
    kind: SyncRangeKindV1,
    days: Option<u32>,
    count: Option<u32>,
    limit: Option<u32>,
    cutoff_ms: Option<i64>,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteFolderRangeEquivalenceV1 {
    remote_folder_id: String,
    range: EffectiveRangeEquivalenceV1,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutomaticScheduleConfigV1 {
    enabled: bool,
    interval_minutes: u32,
}

#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
enum AutomaticSyncConfigV1 {
    History {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: HistoryRangeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
    },
    Likes {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: ArchiveListRangeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
    },
    Favorites {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: ArchiveListRangeV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
        discover_remote_folders: bool,
    },
    RemoteFolder {
        schema_version: u32,
        source_instance_key: String,
        schedule: AutomaticScheduleConfigV1,
        range: ArchiveListRangeV1,
        run_mode: ArchiveRunModeV1,
        remote_folder_mode: RemoteFolderScheduleModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser_preference: BrowserPreferenceV1,
    },
}

#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
enum SyncExecutionEquivalenceV1 {
    History {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Likes {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
    Favorites {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
        discover_remote_folders: bool,
        remote_folder_mode: RemoteFolderScheduleModeV1,
        remote_folder_default_range: EffectiveRangeEquivalenceV1,
        remote_folder_overrides: Vec<RemoteFolderRangeEquivalenceV1>,
    },
    RemoteFolder {
        schema_version: u32,
        source_instance_key: String,
        range: EffectiveRangeEquivalenceV1,
        run_mode: ArchiveRunModeV1,
        scroll_speed: u32,
        hide_browser: bool,
        browser: BrowserExecutionIdentityV1,
    },
}
```

`ArchiveRunModeV1` 固定为 `Incremental | FullReconcile`。automatic job 永远使用 `Incremental`；只有用户明确发起的 manual 请求可使用 `FullReconcile`。完整复核仍必须由 3.4 执行器证明到达真实底端和稳定窗口后，才允许标记远端移除。

- `payload_json` 是固定字段顺序的规范 JSON，`payload_sha256` 为其 UTF-8 SHA-256 小写十六进制；Rust 启动前重新反序列化、拒绝未知字段并复核 hash、账号、source 和边界。
- `equivalence_sha256` 只能由 `SyncExecutionEquivalenceV1` 生成，不得从 payload JSON 删除字段或经通用 `Value/Map` 重排。`schemaVersion` 固定为 1，internally tagged `source` 固定先输出，随后严格按结构体声明顺序输出 camelCase 字段；所有 `Option` 必须显式编码为 JSON null，禁止 `skip_serializing_if`。字符串在进入投影前已按各字段契约完成验证，序列化时逐字节输出 Rust 字符串，不再执行 Unicode normalization、大小写转换或 trim；整数使用十进制 JSON number，布尔值使用 JSON boolean，结果不含空白并以 UTF-8 直接做 SHA-256。
- `EffectiveRangeEquivalenceV1` 的唯一合法组合为：Days 只填 days+cutoff；Bottom/All 四个可选字段全为 null；RecentItems 只填 count；RecordLimit 只填 limit；RecentDays 只填 days+cutoff。Favorites default/override 若为 RecentDays，必须用父 `frozen_at_ms` 各自派生 cutoff。`remote_folder_overrides` 不编码为 JSON object，而转换为按规范 `remoteFolderId` 字节序升序的数组；重复 ID、顺序不规范或任一非法 range 都拒绝计算 hash。
- 等价投影排除纯审计 `frozen_at_ms`、settings revision、queued time 和 schedule interval，但绝不排除 source、instance、run mode、派生 cutoff、浏览器/profile identity、滚动/隐藏参数，以及 Favorites 的 discovery、folder mode、default range 和全部 override range。无 cutoff 的 All/Bottom/RecentItems/RecordLimit 请求可跨创建时间等价；Days/RecentDays 只有所有有效 cutoff 完全相同才等价。
- job 创建、数据库插入、启动恢复/重建、附着或去重、automatic 提升/替换、discovery child 构造、queued→running 前和结果导入前，都必须从严格反序列化的 payload 重建规范 payload JSON 与等价投影，并同时复核 `payload_sha256`、`equivalence_sha256`。任何节点不一致都不得执行、附着或导入。
- `settings_revision` 记录创建任务时 portable settings revision，仅用于审计；写 SQLite 前必须小于等于 `i64::MAX`，revision 达到该值后拒绝继续递增。启动恢复只验证该值存在且范围合法，不与当前 portable revision 比较；manual 始终使用冻结 payload，automatic 是否仍有效只由当前 source-specific `config_sha256` 决定。
- `config_sha256` 只能由 `AutomaticSyncConfigV1` 的规范 JSON计算，schemaVersion 固定为 1，并完全复用等价投影的 tag-first、结构体字段顺序、无空白 UTF-8 和字符串原样编码规则。History/Likes/Favorites 的 instance 固定空字符串；RemoteFolder instance 固定规范 folder ID。automatic archive 的 runMode 必须显式为 Incremental。RemoteFolder 的 schedule/range 是按当前 mode 从该实例 override 或 defaults 解析后的有效值，并同时编码 `remoteFolderMode`；因此 defaults 变化只使实际继承它的实例 hash 改变，单个 override 变化只影响该实例。
- `config_sha256` 必须包含该 task key 当前的 enabled、interval、配置 range 原始天数/数量、账号 `scrollSpeed`、`hideBrowser`、device `browserPreference`，以及 Favorites 的派生 `discoverRemoteFolders` 或 RemoteFolder 的 mode/有效设置。Favorites defaults、overrides 和各 RemoteFolder schedule 不参与 Favorites 自身 config；它们分别只参与对应 RemoteFolder 实例的 config。明确排除 settings revision、`frozen_at_ms`、派生 cutoff、resolved browser、profile key/generation、executable path hash、login/selection marker、排队时间、通知状态和显示格式。
- automatic job 创建、设置协调、启动恢复、dequeue 和 promotion/replacement 前都从同一 settings/device/capability snapshot 重建 `AutomaticSyncConfigV1` 并复核 hash；任一输入字段变化都取消旧 queued automatic 并按新快照至多重建一个。manual job 的 `config_sha256` 固定为 NULL，动态 manual child 不从父 revision 反推旧 schedule。
- payload 仍冻结全部执行参数，包括 resolved browser/profile identity；真正运行时不得改用更新后的账号设置。`Auto` 只在入队时解析一次，payload 同时保存 preference、实际 Edge/Chrome、profile key、profile generation 和规范 executable path hash。config 排除这些 resolved identity 不代表忽略变化；worker 仍按独立 browser/profile identity 门禁复核，不一致返回 `browser_profile_changed`。
- `range_freeze.frozen_at_ms` 固定为创建该 payload 的 UTC epoch ms；History Days 和 archive RecentDays 必须同时保存已按 `Asia/Shanghai` 规则计算的 `cutoff_ms`，其他 range 的 cutoff 必须为 null。动态 child 只继承父 `frozen_at_ms`，再根据自己实际选中的 range 确定性生成新的 RangeFreeze：RecentDays(days) 调用同一 cutoff 函数，其他 range 置 null。DateBoundary 必须与 child cutoff 交叉验证。
- 每个账号/浏览器 profile 持久保存 simple UUID `profile_generation`；只有显式重置/重建 profile 才改变，普通登录恢复不改变。worker 必须复核 payload identity 与当前 selection、login marker、profile generation 和 executable path；不一致返回 `browser_profile_changed`，不得静默切换。
- Favorites payload 在入队时冻结 remote-folder mode、defaults 以及 portable settings 中全部已配置 folder ID 的执行 override，包括当前标记 removed 但仍保留独立设置的 ID；map 按规范 ID 排序且最多 1,000 项，与 settings schema 上限一致。`Unified` 要求 overrides 为空并统一使用 defaults；`PerFolder` 对 map 中 ID 使用冻结 override，未配置 override 的已知或新 ID 都回退 defaults。上述字段全部参与 payload hash。
- automatic job 是否失效只比较当前重算的 `config_sha256`，不受点赞量格式、其他账号或其他来源设置变化影响。所有 SHA-256 字段必须是 64 位小写十六进制。
- `scheduled_for` 只用于 automatic job，manual job 为 `NULL`。
- manual queued job 始终执行入队快照。设置变更不静默改写用户已提交的手动任务。
- automatic queued job 在对应 `config_sha256` 变化或 enabled 关闭后由 scheduler 事务取消为 `automatic_settings_changed`，再按新设置重新计算；已经 running 的任务不变。
- automatic queued job 被同 key manual 请求提升时，用本次手动请求 payload/revision 原子替换旧快照，并设置 `config_sha256 = NULL`、`scheduled_for = NULL`。
- automatic Favorites 和 manual bulk payload 在 capability `supports_remote_folders = true` 时固定 `discover_remote_folders = true`，否则固定为 false；manual `scope = One` 才允许显式选择。新收藏夹发现频率因此至少等于 Favorites 自动同步频率；若 Favorites schedule disabled，已知 RemoteFolder 可继续按自身 schedule 同步，但新收藏夹只会在手动 Favorites/bulk discovery 后出现，设置 UI 必须明确提示。

IPC 固定为：

```rust
enum SyncScopeV1 {
    One,
    CurrentAccountAvailableSources,
    AllActiveAccounts,
}

enum SyncTriggerV1 {
    Manual,
    Automatic,
}

enum SyncJobStatusV1 {
    Queued,
    Running,
    Success,
    Partial,
    Failed,
    Cancelled,
}

enum SyncRangeKindV1 {
    Days,
    Bottom,
    All,
    RecentItems,
    RecordLimit,
    RecentDays,
}

enum SyncBatchStatusV1 {
    Queued,
    Running,
    Success,
    Partial,
    Failed,
    Cancelled,
}

enum SyncBatchIssuePhaseV1 {
    InitialExpansion,
    DiscoveryExpansion,
}

enum SyncJobIssueSeverityV1 {
    Warning,
    Incomplete,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnqueueSyncRequestV1 {
    scope: SyncScopeV1,
    account_id: Option<i64>,
    source: Option<SyncSourceV1>,
    source_instance_key: Option<String>,
    payload: Option<SyncTaskPayloadV1>,
    full_reconcile_confirmation_sha256: Option<String>,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnqueueSyncResultV1 {
    batch_id: Option<i64>,
    counts: SyncBatchCountsV1,
    entries: Vec<SyncBatchEntryV1>,
    next_cursor: Option<String>,
}

#[serde(tag = "kind", content = "data", rename_all = "camelCase", deny_unknown_fields)]
enum SyncBatchEntryV1 {
    Job(SyncJobViewV1),
    Skipped(SyncSkipV1),
    Conflict(SyncConflictV1),
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncBatchCountsV1 {
    jobs: u32,
    skipped: u32,
    conflicts: u32,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncSkipV1 {
    account_id: i64,
    source: SyncSourceV1,
    source_instance_key: String,
    phase: SyncBatchIssuePhaseV1,
    reason: String,
    requested_payload_sha256: Option<String>,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncConflictV1 {
    account_id: i64,
    source: SyncSourceV1,
    source_instance_key: String,
    phase: SyncBatchIssuePhaseV1,
    reason: String,
    requested_payload_sha256: String,
    existing_payload_sha256: String,
    existing_job_at_conflict: SyncJobViewV1,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncJobIssueV1 {
    phase: String,
    code: String,
    severity: SyncJobIssueSeverityV1,
    message: Option<String>,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncJobViewV1 {
    id: i64,
    account_id: i64,
    source: SyncSourceV1,
    source_instance_key: String,
    trigger: SyncTriggerV1,
    status: SyncJobStatusV1,
    queued_at: String,
    scheduled_for: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    queue_position: Option<u32>,
    can_cancel: bool,
    issues: Vec<SyncJobIssueV1>,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncStatusV2 {
    account_id: i64,
    job_id: i64,
    source: SyncSourceV1,
    source_instance_key: String,
    trigger: SyncTriggerV1,
    status: SyncJobStatusV1,
    queue_position: Option<u32>,
    inserted_videos: i64,
    inserted_records: i64,
    inserted_watch_records: i64,
    record_count: i64,
    elapsed_ms: i64,
    retry_count: i64,
    phase: Option<String>,
    can_cancel: bool,
    started_at: Option<String>,
    finished_at: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    issues: Vec<SyncJobIssueV1>,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncBatchViewV1 {
    id: i64,
    trigger: SyncTriggerV1,
    scope: SyncScopeV1,
    status: SyncBatchStatusV1,
    created_at: String,
    finished_at: Option<String>,
    counts: SyncBatchCountsV1,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncBatchEntriesPageV1 {
    batch_id: i64,
    entries: Vec<SyncBatchEntryV1>,
    next_cursor: String,
    has_more: bool,
}
```

所有 IPC struct 和所有带字段的 tagged enum 都使用 `camelCase` 且 `deny_unknown_fields`；unit enum 只能反序列化下列固定字符串，禁止 alias。字符串映射固定为：

- `SyncSourceV1` JSON：`history | likes | favorites | remoteFolder`；SQLite：`history | likes | favorites | remote_folder`；Rust：`History | Likes | Favorites | RemoteFolder`。
- `SyncRangeKindV1`：`days | bottom | all | recentItems | recordLimit | recentDays`。range 对象统一使用 `kind` tag，例如 `{ "kind": "days", "days": 7 }`、`{ "kind": "recentItems", "count": 500 }`。
- `SyncScopeV1`：`one | currentAccountAvailableSources | allActiveAccounts`。
- `SyncTriggerV1`：`manual | automatic`；`SyncJobStatusV1` 和 `SyncBatchStatusV1`：`queued | running | success | partial | failed | cancelled`。
- `SyncBatchIssuePhaseV1`：`initialExpansion | discoveryExpansion`；`SyncJobIssueSeverityV1`：`warning | incomplete`。
- `ArchiveRunModeV1`：`incremental | fullReconcile`；archive completion status：`complete | incomplete`，complete stop：`bottom | requestedItemCount | recordLimit | dateBoundary | stableKnownBoundary`，incomplete reason：`safetyRecordCap | unconfirmedEnd`；`BrowserPreferenceV1`：`auto | edge | chrome`；`BrowserIdV1`：`edge | chrome`。

只允许通过显式函数执行 JSON/Rust/SQLite 映射，禁止在 SQL 或前端散落字符串替换。

`SyncResult` 升级为与 `SyncStatusV2` 相同的 identity/status/count/error/issues 子集，允许 `queued | partial`；history 同时填充兼容字段 `insertedWatchRecords = insertedRecords`，其他来源固定为 0。前端 `types.ts` 使用同一字符串联合类型，不再把 source 写死为 history。

- 保留 `start_sync(account_id, options)` 作为观看历史单任务兼容包装，返回使用完整 `SyncJobStatusV1`，通常初始为 queued/running，查询可得到任一终态。
- 新增 `enqueue_sync_tasks` 支持单来源、当前账号全部可用来源和全部 active 账号。
- `scope = One` 必须提供 account/source；history、likes、favorites 禁止非空 instance，remote-folder 必须提供规范非空 instance。payload 可显式提供，缺失时从该账号当前设置生成快照；显式 payload 的 source/folder ID 必须与请求字段交叉一致。
- `scope = CurrentAccountAvailableSources` 必须提供 account，禁止 source、instance 和 payload；后端逐可用来源及实例生成快照。
- `scope = AllActiveAccounts` 禁止 account、source、instance 和 payload；后端逐 active 账号、可用来源及实例生成快照。
- 所有隐式生成的 manual archive payload，包括两个 bulk scope 和 `scope = One` 缺失 payload，固定 `run_mode = Incremental`。Favorites payload 的 discovery child 永远生成 Incremental RemoteFolder；Favorites defaults/overrides 只保存 range，不允许嵌套 FullReconcile。
- 只有显式 `scope = One` 的 Likes、Favorites 顶层 archive 或单个 RemoteFolder payload 可请求 FullReconcile，且对应 range 必须为 All、`full_reconcile_confirmation_sha256` 必须等于完整规范 payload 的 `payload_sha256`；否则返回稳定错误。无 FullReconcile 时该字段必须为 null。远端收藏夹完整复核必须逐个以 source=RemoteFolder 发起，不通过 Favorites discovery 批量授权。payload 变化会使确认 hash 失效，bulk 永远不携带确认。
- bulk scope 展开后没有任何目标时返回 `no_available_sources`；`AllActiveAccounts` 在更早的账号层面为空时返回 `no_active_accounts`，两者都不创建空 batch。
- 两个批量 manual scope 都同步全部 `available = true` 的来源，不读取自动同步 enabled；enabled 只控制 scheduler。用户要同步单一来源时使用 `scope = One`。
- `get_sync_status` 必须返回 queued 状态和排队位置；`cancel_sync` 同时支持 queued 与 running。
- 批量入队在单一事务中按 `equivalence_sha256` 逐项去重并持久化每个 job/skip/conflict；执行不等价冲突不得使其他独立 task key 回滚，也不能部分成功后丢失结果。IPC 只返回稳定排序的前 200 个 `entries`、总 counts 和 opaque cursor，禁止一次序列化完整大 batch。
- 新增 `get_sync_batch(batch_id)` 返回 summary；`list_sync_batch_entries(batch_id, cursor, limit)` 按共享 `entry_seq ASC` 分页，limit 范围 1 到 200。cursor 绑定 batch ID 和最后已见 entry_seq，篡改或跨 batch 使用返回 `invalid_batch_cursor`。active batch 即使当前没有更多也返回可继续轮询的 cursor；后续动态 entry 序号只会更大，不会落入已翻过区间。
- 运行时上限固定为：全局 active batch 最多 16；一个 active job 最多关联 4 个未终态 batch；单个 batch 的 `target_kind='task'` 初始加动态可执行目标最多 5,000；单账号可调度的未移除 RemoteFolder 最多 2,000。`target_kind='aggregate'` 是持久不完整证据，不计入 5,000，但仍占一个 entry_seq。初始 preflight projected task 超过 5,000 时不创建 batch并返回稳定错误；discovery 对每个 batch 先以完整候选集合计算 projected task，<=5,000 才全部展开，>5,000 时不按遍历顺序接纳部分 child，只写一个不计数的 aggregate `sync_batch_target_limit_exceeded` skip 并关闭该 batch 的 RemoteFolder expansion，其他 batch 可继续扩展。已有恰好 5,000 个 task 时仍必须能写该 aggregate 作为第 5,001 个审计 entry。
- 未移除 RemoteFolder 超过 2,000 时 scheduler 不枚举实例，source summary 返回 `remote_folder_schedule_limit_exceeded`；manual bulk 仍可同步其他来源，并为 RemoteFolder 写一个 aggregate skip。`source_instance_key = ''` 仅在这种 source-level aggregate batch issue 中允许，不代表合法 RemoteFolder task key。
- 已持久化 `remote_folder_schedule_limit_exceeded` 或 `sync_batch_target_limit_exceeded` aggregate skip 的 batch 被标记为该次 RemoteFolder expansion closed，后续 Favorites discovery 不得再向它补 child。
- `CurrentAccountAvailableSources` 和 `AllActiveAccounts` 首次展开时，只有 `remote_removed_at IS NULL` 的已知 remote-folder 实例可直接创建或附着 child job；当 capability 支持 remote folders 时，Favorites 根任务必须把 `discover_remote_folders = true` 写入不可变 payload，以便在完整发现后向同一 batch 补齐首次未知或新出现的收藏夹。
- 初始展开和 discovery 扩展都必须先查询/保留 `sync_batch_targets`。已经由该 batch 关联过 queued、running 或 terminal job，或已经产生 skip/conflict issue 的完整 task key，一律从 discovery 候选中排除；不得因原 job 已终态、已删除或 payload 当前变为等价而再次关联或执行。只有该 batch 从未保留过的 folder task key 才能进入动态 child 决策。
- 一个 active job 可附着多个 batch。等价 active job 通过 `sync_batch_jobs.role = 'attached'` 关联；不等价 active job 写入该 batch 的 conflict，不修改原 job，也不阻止其他 task key 入队。

### 5.3 队列规则

`SyncQueue` 是 `AppState` 持有的内存索引和条件变量，SQLite 是可恢复事实源。

- 手动队列按持久 `queue_seq ASC` FIFO，始终先于自动队列。
- 自动队列按 `scheduled_for`、账号 ID、来源、instance、`queue_seq` 排序；正常运行和重启恢复必须使用同一个比较函数。
- 任务 key 固定为 `(account_id, source, source_instance_key)`；终态查询、配置 hash、通知 streak、去重、状态返回和下次到期计算都使用完整 key。
- 同一 key 已有 active job 且 `equivalence_sha256` 完全相同时，新的请求可附着；结果 envelope 仍必须匹配实际执行 job 的完整 `payload_sha256`，不能用新请求的审计 freeze 替换 running job。
- 已有 equivalent automatic queued 时收到 manual 请求，原 job 原子提升为 manual，并分配新的 `queue_seq` 移到手动队列尾部；只改内存顺序而不持久化视为错误。
- 已有 non-equivalent automatic queued 时收到 manual 请求，在同一事务取消旧 automatic job，再插入 manual job。
- 已有 running 或 manual queued 且 execution equivalence 不同时返回 `sync_task_payload_conflict` 和现有 job view，不静默丢弃完整复核或不同范围请求。
- 初始 bulk 展开、scheduler 入队和 Favorites discovery child 扩展必须调用同一个事务内 `decide_enqueue_for_task_key` 状态机，禁止为动态 child 复制一套简化去重规则。外部调用要求 admission Open；discovery 调用必须携带父 job 的 continuation token，只能生成该父 payload 推导出的 child。
- discovery 扩展先按 task key 聚合全部目标 batch，并排除 expansion-closed 或已存在 `sync_batch_targets` 行的 batch；随后基于事务开始时的 active job 只决策一次：等价 automatic queued 提升为 manual；不等价 automatic queued 取消并替换；等价 running/manual queued 附着；不等价 running/manual queued 为每个目标 batch 写 conflict；无 active job 至多创建一个 manual job。每个剩余 batch 的 target reservation 与对应 association/issue 必须在同一事务写入，唯一冲突表示该 key 已被处理，整个该 key 决策回滚并按数据库现状重新读取，禁止写第二条 entry。
- 对每个 task key，候选 batch 先按 ID 排序并排除 expansion-closed 项。若已有 job 的当前未终态 batch 关联数加全部新候选会超过 4，则对该 task key 采用 all-or-none：不附着任何新候选，并为每个候选写带 existing snapshot/hash 的 `sync_job_batch_attachment_limit_exceeded` conflict。若不存在 active job但候选本身超过 4，则不创建 job，并为每个候选写同 reason 的 skipped entry；禁止伪造 existing job，也禁止按遍历顺序只选择前几个。
- 本轮 discovery 创建、提升或替换得到的 job，对全部目标 batch 都标记 `discoveredChild`；只有事务开始前已经存在且保持不变的等价 running/manual job 标记 `attached`，结果不得受 batch 遍历顺序影响。
- 取消 queued job 直接更新为 cancelled 并从内存队列移除；取消 running job 复用现有协作取消和两秒强制终止。
- 应用异常退出后遗留 queued job 通过 payload/hash、settings revision 范围、账号、capability、browser/profile 和 automatic config 校验后恢复到内存队列；`settings_revision` 不与当前 portable revision 比较。无效 payload、已归档账号、不可用来源或过期 automatic 设置才标记 cancelled，并写入稳定 reason。
- 受控退出明确取消 queued job，因此用户主动退出后不会在下次启动意外执行旧任务。
- 启动遗留 running job 必须按 7.3 的统一恢复矩阵先导入合法完整结果，或按恢复 disposition 条件 CAS 为 failed/cancelled；恢复结束并断言数据库无 running 后才能重建内存队列和唤醒 worker，禁止留下没有 orchestration 的 running。
- `started_at IS NULL` 的 cancelled job 永远不作为后续自动调度锚点；只有同一完整 task key 真正进入 running 后产生的 success、partial、failed 或 cancelled 终态可作为锚点。

### 5.4 执行器边界

```rust
enum SyncSourceV1 {
    History,
    Likes,
    Favorites,
    RemoteFolder,
}

trait SyncExecutor {
    fn source(&self) -> SyncSourceV1;
    fn start(&self, context: QueuedSyncContext) -> Result<RunningSyncIdentity, String>;
}

trait SyncResultImporter {
    fn source(&self) -> SyncSourceV1;
    fn validate_and_import(
        &self,
        transaction: &rusqlite::Transaction,
        job: &SyncJobSnapshotV1,
        result: SourceExecutionResultV1,
    ) -> Result<ImportCountsV1, String>;
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncExecutionEnvelopeV1 {
    schema_version: i64,
    account_id: i64,
    job_id: i64,
    source: SyncSourceV1,
    source_instance_key: String,
    task_payload_sha256: String,
    result: SourceExecutionResultV1,
}

#[serde(
    tag = "source",
    content = "data",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum SourceExecutionResultV1 {
    History(CollectorSuccessV3),
    Likes(ArchiveCollectionSuccessV1),
    Favorites(FavoriteCollectionSuccessV1),
    RemoteFolder(RemoteFolderCollectionSuccessV1),
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveItemV1 {
    aweme_id: String,
    canonical_url: String,
    content_type: String,
    source_position: i64,
    remote_created_at_ms: Option<i64>,
    title: Option<String>,
    author_name: Option<String>,
    cover_url: Option<String>,
    like_count: Option<i64>,
}

#[serde(tag = "kind", content = "data", rename_all = "camelCase", deny_unknown_fields)]
enum ArchiveCompleteStopV1 {
    Bottom,
    RequestedItemCount { count: u32 },
    RecordLimit { limit: u32 },
    DateBoundary {
        cutoff_ms: i64,
        guard_aweme_id: String,
        guard_remote_created_at_ms: i64,
    },
    StableKnownBoundary { consecutive_known: u32 },
}

#[serde(tag = "kind", content = "data", rename_all = "camelCase", deny_unknown_fields)]
enum ArchiveIncompleteReasonV1 {
    SafetyRecordCap { cap: u32 },
    UnconfirmedEnd,
}

#[serde(tag = "status", content = "data", rename_all = "camelCase", deny_unknown_fields)]
enum ArchiveCompletionV1 {
    Complete { stop: ArchiveCompleteStopV1 },
    Incomplete { reason: ArchiveIncompleteReasonV1 },
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveCollectionSuccessV1 {
    records: Vec<ArchiveItemV1>,
    identity_hash: String,
    field_hash: String,
    completion: ArchiveCompletionV1,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteFolderDescriptorV1 {
    remote_folder_id: String,
    name: String,
    source_position: i64,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteFolderDiscoverySuccessV1 {
    folders: Vec<RemoteFolderDescriptorV1>,
    identity_hash: String,
    field_hash: String,
    complete_scan: bool,
    bottom_confirmed: bool,
    hit_record_limit: bool,
}

#[serde(
    tag = "status",
    content = "data",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum RemoteFolderDiscoveryResultV1 {
    NotRequested,
    Success(RemoteFolderDiscoverySuccessV1),
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FavoriteCollectionSuccessV1 {
    archive: ArchiveCollectionSuccessV1,
    remote_folder_discovery: RemoteFolderDiscoveryResultV1,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteFolderCollectionSuccessV1 {
    remote_folder_id: String,
    name: String,
    archive: ArchiveCollectionSuccessV1,
}
```

- 持久结果文件根节点只能是一个 `SyncExecutionEnvelopeV1`，`schemaVersion` 固定为 1；envelope、tagged enum、现有 `CollectorSuccessV3` 适配层以及所有嵌套 result/record struct 都必须使用 `camelCase` 和 `deny_unknown_fields`，禁止 serde alias、未标记 flatten 或宽松 `Value` 旁路。
- `SourceExecutionResultV1` 的 JSON tag 固定为 `history | likes | favorites | remoteFolder`。monitor 必须交叉验证 envelope source、result tag、数据库 job source、`accountId`、`jobId`、`sourceInstanceKey` 和 `taskPayloadSha256`；任一不一致按 `sync_result_identity_mismatch` 拒绝导入。
- `remoteFolderDiscovery` 字段必须存在：Favorites payload 中 `discover_remote_folders = true` 时固定为 `{ "status": "success", "data": ... }`，为 false 时固定为 `{ "status": "notRequested" }`。禁止通过缺字段或 `null` 表示未请求。RemoteFolder result 的 `remoteFolderId` 必须同时等于 envelope instance、job instance 和 payload folder ID。
- archive records 最多 50,000 条，必须按零基连续 `sourcePosition` 升序且 `awemeId` 唯一；`awemeId` 为 1 到 64 位十进制字符串，`contentType` 固定 `video | note`，规范 URL 必须是同源 `https://www.douyin.com/video/{id}` 或 `/note/{id}` 且路径 ID 一致。`remoteCreatedAtMs` 为 null 或 0 到 9,999,999,999,999，`likeCount` 为 null 或非负 i64；title 最多 2,048 字符、author 最多 256 字符、cover HTTPS URL 最多 4,096 字节。
- archive identity hash 输入为按 `sourcePosition ASC` 排列的 `[awemeId, sourcePosition]` 数组；field hash 输入固定为 `[awemeId, canonicalUrl, contentType, sourcePosition, remoteCreatedAtMs, title, authorName, coverUrl, likeCount]` 数组，缺失值必须编码为 JSON null。URL 先按统一 normalization 规范化再参与哈希，重复 ID、位置断裂、非法 null/字符串/范围均在导入前拒绝。
- `ArchiveCompletionV1::Complete` 必须严格匹配 payload，否则结果语义非法并 failed，来源数据零写入。规范 stop 选择优先级固定为：对 RecentItems/RecordLimit，记录数达到请求上限时必须使用对应 count/limit stop；只有记录数严格小于上限且确认真实底端时才能用 Bottom。FullReconcile 仅允许 range=All + Bottom。
- Incremental Complete 唯一矩阵为：All→Bottom(records <= 50,000) 或 StableKnownBoundary(records < 50,000)；RecentItems(count)→RequestedItemCount(records = count) 或 Bottom(records < count)；RecordLimit(limit)→RecordLimit(records = limit) 或 Bottom(records < limit)；RecentDays→DateBoundary 或 Bottom，且两者都要求所有 records 的可靠 `remoteCreatedAtMs` 非空并 `>= cutoffMs`。RecentDays 在 records=50,000 时仍可合法 Complete，但只能满足下一条规定的唯一优先级。
- DateBoundary 还必须复核 capability、`cutoffMs` 等于冻结 payload 值、guard ID 合法且不与 records 重复、`guardRemoteCreatedAtMs < cutoffMs`。StableKnownBoundary 仅允许 Incremental+All；importer 在写入前按 records 尾部顺序查询对应账号/source/instance 的既有关系，计算真实连续已归档数量，要求 result 的 `consecutiveKnown` 与该值完全相等且 >= 20。任一字段、count、range、run mode、日期或 DB 前态不匹配都是非法 result，不降级为 partial。
- `ArchiveCompletionV1::Incomplete` 是唯一允许“数据有效但未完成”的表达。SafetyRecordCap 允许 range=All 或 RecentDays，要求 cap 固定 50,000 且 records=50,000；RecentDays 还要求所有 records 时间非空并 >= cutoff。RecordLimit(50,000) 达限必须使用 Complete RecordLimit，不能伪装 safety cap。UnconfirmedEnd 的长度约束为：All < 50,000、RecentItems < count、RecordLimit < limit、RecentDays < 50,000 且所有已采 records 时间非空并 >= cutoff。合法 incomplete 分别写 `archive_safety_record_cap_reached` 或 `archive_unconfirmed_end`，导入本轮出现项、job partial、禁止远端移除。
- All 在 records=50,000 时，只有 collector 在触发 cap 前的同一最终稳定窗口已确认真实底端才能报告 Complete Bottom；否则必须报告 Incomplete SafetyRecordCap。该先后关系写入脱敏 diagnostic 并纳入 3.4 实页门禁。
- RecentDays 在第 50,000 条写入后必须停止继续扩充 records，并按固定优先级完成一次 guard 判定：若同一最终稳定窗口确认第 50,000 条恰为真实底端，报告 Complete Bottom；否则只允许观察第 50,001 个不写入 records 的 guard，若其可靠时间严格小于 cutoff，报告 Complete DateBoundary 并把该 guard 写入 stop；若 guard 仍在范围内、无法取得可靠时间，或 bottom/范围外 guard 均未确认，则报告 Incomplete SafetyRecordCap。不得因为先达到 cap 就遮蔽已经确认的 Bottom 或 DateBoundary，也不得把第 50,001 项写入结果数组。
- 只有 FullReconcile + Complete(Bottom) 可标记远端移除；其他合法 Complete 为 success 但不做批量移除。取消、脚本异常、导航/CDP 错误、硬超时、结构/语义非法结果均 failed/cancelled 且不导入，不通过 Incomplete 吞掉技术错误。
- 收藏夹发现最多 1,000 项，规范输入按零基连续 `sourcePosition ASC` 排列；identity hash 为 `[remoteFolderId, sourcePosition]` 数组，field hash 为 `[remoteFolderId, name, sourcePosition]` 数组。ID 必须唯一且 1 到 256 字节，name 为 1 到 256 字符。全部 SHA-256 必须是规范 JSON UTF-8 的 64 位小写十六进制。
- `RemoteFolderDiscoverySuccessV1` 的三个布尔值和长度只有以下三种合法组合：完整扫描为 `completeScan=true, bottomConfirmed=true, hitRecordLimit=false, folders.len() <= 1,000`；恰好达到发现上限为 `false, false, true, folders.len() = 1,000`；尚未确认结束为 `false, false, false, folders.len() < 1,000`。其他组合全部视为非法 result 并 failed、来源数据零写入。只有完整扫描组合允许创建动态 RemoteFolder child，且仍须满足 manual FullReconcile 等既有条件后才能标记远端收藏夹移除；后两种组合只能 upsert/恢复本轮已见 folder、导入合法 Favorites archive、写 incomplete issue 并把父 job 终态化为 partial。
- 本阶段注册 `HistorySyncExecutor`，内部复用当前 collector、恢复登录和导入流程。
- likes/favorites/remote-folder 未注册时不会被自动排队；外部手动请求返回 `sync_source_not_available`。
- 队列 monitor 只校验 envelope 的 job/task identity并将 tagged result 交给对应 importer；队列本身不读取观看时间、喜欢关系或收藏夹字段。
- importer 使用立即事务，第一步重新读取并确认 job 仍为 `running` 且 envelope identity/hash 一致；随后在同一事务完成来源记录、issue、远端移除、job counts/终态、全部 batch snapshot 和状态。最终更新必须是 `WHERE id = ? AND status = 'running'` 的条件 CAS；失败完整回滚来源数据，再用短事务仅在仍为 running 时标记 failed。
- cancel、强杀恢复和启动恢复也只能通过条件 CAS 抢占 running 终态。取消先提交则 importer 看到非 running 并不得写任何来源数据；importer 先取得 SQLite writer 并提交 success/partial 后，取消只能返回已终态。monitor 对竞争失败的结果文件只做脱敏诊断和清理。
- likes/favorites/RemoteFolder 内容关系只有 manual FullReconcile + `ArchiveCompletionV1::Complete(Bottom)` 时才能批量标记远端移除。
- Favorites importer 只有在 `remoteFolderDiscovery` 的 hash、唯一性和三个完整性字段全部通过，且父 payload 要求 discovery 时，才允许扩展与该 Favorites job 关联的全部未终态 batch。事务顺序固定为先 upsert/恢复发现的 folder registry，再按 task key 调用统一 enqueue 状态机扩展 child，最后导入 Favorites、终态化父 job 并重算所有 batch；中间不得出现父任务已终态但 child 尚未关联的可见状态。
- 动态 child 先按父 Favorites payload 中冻结的 mode/override/defaults 只选择 range，`run_mode` 无条件固定为 Incremental；再用父 `frozen_at_ms` 与 child range 重建自己的 RangeFreeze，并继承滚动速率、隐藏浏览器、browser identity 和 settings revision，不读取导入时的新设置。此前 removed 后重新出现且存在冻结 override 的 ID 使用该 override，其余已知或未知 ID 都回退 defaults。规范 child payload 单独计算完整/equivalence hash，trigger 为 manual 且 `config_sha256 = NULL`。创建/提升/替换/附着/conflict 和多 batch role完全遵循 5.3 的统一状态机。
- RemoteFolder capability 只取决于执行器是否注册，不取决于该账号是否已有发现结果；执行器可用但 registry 为空是合法的“零实例”状态。3.4 首次完整 discovery 因此可以在同一事务先 upsert folder，再创建 child，不形成可用性闭环。
- 远端 folder registry 每账号最多保留 50,000 个规范 ID，removed 项也计入；新增 ID 将超过上限时整个 discovery 事务以 `remote_folder_registry_limit_exceeded` 失败，不截断、不覆盖旧项。override payload 上限独立固定为 1,000，只冻结用户实际配置的 map，不要求枚举全部 registry。
- discovery 不完整时仍可导入已验证的 Favorites archive，并 upsert/恢复本轮确实出现的 folders，但不得为当前 batch 创建 RemoteFolder child，也不得标记未出现项被移除；父 job 写 `sync_job_issues(code = 'remote_folder_discovery_incomplete', severity = 'incomplete')` 并以 `partial` 终态完成，所有关联 batch 自动汇总为 partial。提交后仍唤醒 scheduler，使已知实例按正常 automatic 设置重新计算；单任务和 automatic 同样可见该 issue。
- 只有 manual `FullReconcile`、discovery 完整三条件成立且 hash 复核通过时，Favorites importer 才能把本地已知但本轮未出现的远端收藏夹标记为“已从远端移除”。incremental、automatic、记录上限命中或任何不完整扫描都只能 upsert 已发现项。
- scheduler 和 bulk 展开只使用 `remote_removed_at IS NULL` 的实例。完整复核标记 folder removed 时，在同一事务取消其 queued RemoteFolder job 为 `remote_folder_removed`、更新关联 snapshot/issue 并重算 batch；全局串行保证此时不存在另一个 running RemoteFolder。folder 后续重新出现时清除 removed 标记，再由 scheduler 重新计算。
- 3.4 实现三类 collector result 生成、来源字段深度验证和 importer；不改队列优先级、task identity、取消、恢复、monitor envelope 或托盘入口。
- `BrowserOperationGuard` 改为覆盖浏览器操作全生命周期的 lease，并存入 `RunningSync` 或 `RunningLogin`；不能像当前入口一样在子进程启动后立即释放。
- queued→running 成功时由 `OperationAdmission` 发放只绑定该 job/task identity 的 `OperationContinuationToken`，并由 `RunningSync` 与 monitor/importer 共同持有到终态事务提交或取消收尾完成。token 只授权已经获准任务执行 DB-only 的进度、结果导入、batch child 关联和终态更新；不能启动新浏览器、接受外部 enqueue、修改设置或开启无关任务。
- `MaintenancePending | ExitPending | Exiting` 不撤销已发放 token。maintenance 必须等待所有 continuation 终态后再取消 queued；退出先停止 worker并解决 running/monitor，再执行最终 queued 清扫。因此 importer 在 pending 状态下无需重新申请 admission，也不会在最终清扫之后留下 child。
- worker 必须先取得同一个全局浏览器 operation lease，再以数据库 CAS 把 queued 改为 running，之后才创建 orchestration 和 collector。登录占用 lease 时任务保持 queued；不得并发启动第二个浏览器。
- queued→running 的 CAS 前，worker 必须在同一 `OperationAdmission` 临界区按 trigger 分流复核。manual 只检查账号仍 active、source executor/capability 存在、payload/hash/instance 合法以及冻结 browser/profile 身份可运行，不比较当前 enabled、interval 或 `config_sha256`；automatic 还必须检查当前 enabled 和重算的 `config_sha256`，不匹配则取消/重建。
- 账号来源设置更新只通过 `source_settings_update_pending` 阻止对应 automatic key dequeue；portable settings 原子写入成功后协调 queued automatic 再解除 pending。manual queued/running 始终使用冻结 payload，不因普通设置变化被取消或改写。
- executor 启动任一步失败都在一个短事务中把 job 转 failed、释放 running 唯一索引，并唤醒下一任务；不得留下 DB running 但没有 orchestration 的状态。

## 6. 调度器

### 6.0 全局启动门禁

GUI 进程在 collector/login/uninstall helper 模式分流后，只允许通过一次性的 `recover_desktop_startup_once` 进入正常运行，顺序固定且不可交换：

1. 取得 GUI 单实例 mutex，只解析独立于可选 data root 的当前用户协调目录；此时不得加载 bootstrap、打开用户数据库或创建 scheduler。
2. 先严格读取 `autostart-cleanup-fault.json`、`uninstall-interrupted-fault.json` 和 `uninstall-session.json`。Preparing/Prepared 在产品/路径身份复核后完成幂等回滚；DestructiveCommitted 保持失败关闭并要求安装器继续卸载或修复；Completed/RepairCompleted 只清理终态残留。任一结构、身份或补偿错误都只启动最小恢复 UI，禁止解析 data root、打开 SQLite、启动托盘静默模式或自动调度。
3. 卸载状态不存在或已安全清理后，才读取 system app data 下的 device settings/bootstrap，解析并锁定当前 data root。
4. 打开数据库并完成 schema migration 后调用 `recover_sync_state_once`，按 startup fault、invalid shutdown marker、normal shutdown marker、普通崩溃的顺序处理 running/queued、结果导入和准确进程清理。
5. 断言数据库无 running、恢复内存队列并完成 device/onboarding 决策；随后创建托盘和主窗口状态，scheduler worker 最后启动。任何早期失败都不能留下后台 worker、浏览器 lease 或已经显示后再隐藏的窗口。

`recover_desktop_startup_once`、卸载前置恢复和 `recover_sync_state_once` 各自持久记录 phase，重复调用只返回既有结果；任一普通 IPC 重复执行目录准备都不得重跑这些恢复步骤。

### 6.1 空闲模型

调度器使用一个 Rust 线程、`Mutex + Condvar` 和最多 30 秒的等待周期：

- 设置变更、任务终态、账号变更、托盘命令和应用退出会主动唤醒。
- 空闲时不启动 Edge/Chrome，不轮询页面，不创建额外 WebView。
- 每次唤醒重新读取当前 data root、portable settings、device settings、账号状态和最新任务终态。
- 数据根进入 maintenance 时停止取新任务；迁移完成后由 runtime path 变更通知重新计算。
- `prepare_app_directories` 只负责幂等创建目录，不再执行任务恢复。`recover_sync_state_once` 只在 6.0 的卸载前置门禁、runtime path 初始化和 schema migration 成功后运行一次，严格按 shutdown/fault 判定、7.3 running 结果导入或终态化、queued disposition/校验、准确浏览器清理、无 running 断言和内存队列重建的顺序完成；scheduler 由外层启动流程最后唤醒。
- 任何登录、同步或设置命令重复调用目录准备时都不得取消或重排现有 queued job。

### 6.2 到期计算

调度器固定遍历后端 capability 表中的全部已知来源，再从该账号设置展开可调度实例：history、likes、favorites 各一个；remote-folder 在 Unified 和 PerFolder 两种模式都遍历 registry 中全部 `remote_removed_at IS NULL` 的 ID，再使用 `overrides[id].unwrap_or(remote_folder_defaults)` 取得该实例设置。bulk 使用同一个展开函数，只是忽略 schedule enabled。任何实现都不得把 PerFolder 误解为“只遍历 override map”。只有 `available = true` 且 automatic schedule enabled 的实例进入以下计算：

1. `scheduler_paused = true`、引导未完成或应用正在退出时不排队。
2. 有 queued/running job 时不重复排队。
3. 有最近且 `started_at IS NOT NULL` 的终态时，`next_due = finished_at + interval`；`cancelled_before_start`、启动清理和受控退出取消不参与计算。
4. 没有终态时，`next_due = app_ready_at + startup_sync_delay`。
5. `now >= next_due` 时只排入一个 automatic job。

手动任务不改变自动同步开关，但其终态会成为下一次自动同步的时间锚点，避免手动同步后立即重复自动同步。

capability 支持 remote folders 时，每次 automatic Favorites job 都执行收藏夹 discovery。Favorites schedule disabled 时不会自动发现新 folder；已知未移除 RemoteFolder 仍可按自己的 schedule 同步，设置页必须显示该限制，用户可启用 Favorites schedule 或手动执行 Favorites/bulk discovery。

### 6.3 暂停与恢复

“暂停自动同步”只影响 automatic job：

- 已经 running 的 automatic job 继续至终态，不强制取消；manual queued/running 始终照常执行。
- 已经 queued 的 automatic job 保留在 SQLite 和内存队列，但 worker 在 dequeue 前必须在 `OperationAdmission` 下检查 paused，暂停期间绝不启动浏览器。
- 暂停使用 `scheduler_control_tx` 串行化：先在 admission 中设置 `automatic_pause_pending` 阻止 dequeue，释放 admission 后原子写 device settings；写入成功再转为 paused，失败则清除 pending 并唤醒 worker。
- 恢复后先逐项复核 capability、账号状态、payload/config hash 和 enabled；无效项取消，有效项按原 `scheduled_for` 参与排序。暂停期间跨过多个周期仍只保留一个 task key，不补跑多次。
- 重启读取 durable `scheduler_paused` 后，在恢复 queued job 前设置 admission paused，避免启动窗口尚未完成时任务抢跑。

### 6.4 Maintenance 顺序

队列 dequeue/enqueue、`begin_browser_operation`、source settings 更新、maintenance 和退出共用一个 `OperationAdmission` mutex。模式固定为 `Open | MaintenancePending | MaintenanceRunning | ExitPending | Exiting`，并保存当前全生命周期 browser lease、source/account update gates、job continuation registry，以及统一 `owned_operation_registry`；maintenance、source/device/browser 跨资源更新都必须作为 owned operation 注册。任何登录或新任务启动都必须在该 mutex 下确认模式为 Open 且对应 gate 空闲；已发放 continuation 只按 5.4 的窄权限收尾。

数据根迁移和其他 maintenance 顺序固定为：

1. 在 `OperationAdmission` 锁内先检查 active source/device/browser update gates；存在时立即返回 `settings_update_in_progress`。随后检查当前 browser lease；若是 Login 或其他用户可无限保持的浏览器操作，立即返回 `browser_operation_in_progress`，不得修改队列或 gate。
2. 若无 browser lease或只有有硬超时的 RunningSync，在同一个 admission 临界区先创建 operation ID/handle placeholder 并登记到统一 owned registry，再把模式切换为 `MaintenancePending`，随后才能释放锁；从此不再有新操作进入。线程/future 启动失败也必须通过该 placeholder finalizer 移除 registry 并恢复状态。
3. 不持有 admission、队列、device store、bootstrap 或数据库锁，有界等待现有 RunningSync、sync monitor/importer 和全部 continuation token 终态。允许已获准 importer 在此期间提交 child；超时则只清除 admission gate 并唤醒 worker，原 queued job 保持不变。
4. RunningSync/monitor/continuation 全部清空后重新取得 admission 锁并验证 browser lease 为空；随后在一个短事务中取消此时存在的全部 queued job，包括 importer 新创建的 child。事务失败则清除 gate，数据库与内存队列保持原状。
5. 释放 admission 和队列锁后调用现有 `RuntimePaths::begin_maintenance`，等待残余 path lease 归零。
6. maintenance future/线程句柄由 `AppState` 保存并登记在统一 owned registry；迁移成功或失败后先释放 runtime maintenance lease，再在 admission 临界区移除自身。若模式仍为 `MaintenancePending | MaintenanceRunning`，恢复 Open；若为 `ExitPending` 且 `exit_abort_pending = true`、统一 registry 已空，则 abort finalizer 优先恢复 Open并清除退出状态；若为 ExitPending 且未 abort，或已 Exiting，则保持退出状态。任何路径都在锁外唤醒对应 worker/UI。

任何代码不得持有 admission/队列锁等待 path lease，也不得在已经进入 runtime maintenance 后等待一个尚未获得 path lease 的任务或登录执行。

### 6.5 状态查询

新增 `get_scheduler_state` 返回：

- 全局 paused、worker 状态和下次唤醒时间。
- 每个账号的登录状态。
- 全部已知来源的 capability，不得省略不可用来源。
- history、likes、favorites 的 enabled、interval、last status、last finished time、next due time、queued/running job ID，以及 RemoteFolder 的 aggregate count/error；不在全局状态中返回全部 folder 实例。

```rust
struct SourceCapabilityViewV1 {
    source: SyncSourceV1,
    available: bool,
    unavailable_reason: Option<String>,
    supported_ranges: Vec<SyncRangeKindV1>,
    supports_full_reconcile: bool,
    supports_remote_folders: bool,
}
```

`get_scheduler_state` 固定返回 history、likes、favorites、remote-folder 四项；本阶段后三项 `available = false`，并使用稳定原因 `collector_not_implemented`。3.4 中某来源的 executor/importer 完整注册后立即 `available = true`；账号是否完成过 folder discovery 只决定 RemoteFolder 实例列表是否为空，不参与 capability availability。`supports_remote_folders = true` 仅在 Favorites 与 RemoteFolder 两类执行器都注册时返回。

远端 folder registry 的事实源是 `remote_folder_registry_state(account_id, revision)`；每个账号恰有一行，主键同时提供 revision 查询索引。任一 discovery upsert 批次、removed 标记或恢复操作都在修改 `remote_folders` 的同一 SQLite 事务把 revision 递增一次，而不是按 folder 行数递增；事务无实际 registry 写入时不递增。新增 `list_scheduler_instances(account_id, source = remoteFolder, cursor, limit)`，cursor 同时绑定 account/source、registry revision、portable settings revision 和最后 folder ID，limit 为 1 到 200；分页使用 `idx_remote_folders_scheduler(account_id, remote_removed_at, remote_folder_id)`。

每页读取顺序固定为：先克隆指定 portable settings revision 的不可变 snapshot；开启 SQLite read transaction，在同一快照内校验 registry revision并查询 page；提交/结束 read transaction后再次读取 portable revision。前后 revision 不一致或 cursor 任一 revision 不匹配都返回 `scheduler_instances_changed`，前端从第一页重载。这样 JSON settings 与 SQLite 虽非同一事务，也不会返回混合版本页；响应可代表一个已完成的历史 revision，后续变化只使下一页失效。设置页只有展开远端收藏夹区域时才调用，scheduler 每次唤醒只查询到期候选和 aggregate，不做 50,000 项全表 materialize。

设置页和首次引导使用同一结构，不在前端自行推算时间。

## 7. 托盘和窗口生命周期

### 7.1 托盘菜单

菜单固定为：

1. 打开主界面
2. 立即同步当前账号
3. 同步全部账号
4. 暂停自动同步（可勾选）
5. 设置
6. 退出

- 左键单击托盘图标显示并聚焦主窗口，右键显示菜单。
- 没有 active 账号、maintenance 或应用退出中时，相关同步菜单项动态禁用；已有 running job 时仍允许排队或附着，保持队列规则一致。
- “立即同步当前账号”使用 `CurrentAccountAvailableSources`；“同步全部账号”使用 `AllActiveAccounts`，两者都忽略自动 enabled 并同步全部可用来源。
- “设置”先写入 `DesktopIntentState.open_settings`，显示主窗口并发送 `desktop:intent-changed` 提示事件。
- 托盘图标使用现有应用图标；视觉替换留到安装包美化阶段。
- `AppState` 保存 `tray_available`。托盘创建失败时固定显示主窗口、记录稳定错误并禁止静默启动；设置页禁用“关闭到托盘”，不得把窗口隐藏到不存在的托盘。

### 7.2 关闭行为

- `close_behavior = Tray` 且 `tray_available = true`：阻止 main window 关闭并隐藏；首次发生时发送一次“应用仍在后台运行”的系统通知，成功或失败后都记录 `first_tray_hint_shown`，避免循环提示。
- `close_behavior = Tray` 但托盘不可用：不隐藏窗口，显示 `tray_unavailable` 错误；用户可改为退出。若窗口关闭来自系统退出，则进入受控退出。
- `close_behavior = Exit`：进入受控退出流程。
- 引导未完成时关闭主窗口始终退出，不把未配置应用留在后台。
- 仅隐藏窗口不会取消正在运行的同步。
- `tauri.conf.json` 中 main window 固定 `visible: false`。setup 完成 startup recovery、device settings 读取和托盘创建后，才根据 manual/autostart、silent 和 onboarding 状态显式调用 `show`；禁止先显示再隐藏。

### 7.3 受控退出

`request_app_exit` 使用原子状态保证只执行一次：

1. 在 `OperationAdmission` 锁内切换到 `ExitPending`，立即阻止新队列、登录、设置更新和 maintenance；克隆统一 `owned_operation_registry` 中全部 maintenance/source/device/browser operation 的 cancellation token/handle 后释放锁。任何等待都不得发生在 admission 锁内。
2. 锁外触发 cancellation 并有界等待全部 owned operations。各 finalizer 在 `ExitPending` 且未设置 abort 时只完成/补偿自身事务并清除 gate，不恢复 Open；若 `exit_abort_pending = true`，最后一个 registry finalizer按第 3 步恢复。全部结束后才能读取稳定 bootstrap/root。
3. 任一 owned operation 在准备截止时间内未结束时，不得写 shutdown marker，也不得取消 queued/running。所有 marker 前失败统一调用 `abort_exit_before_marker(error)`：在同一个 admission 临界区设置 `exit_abort_pending`、重新检查 active owned-operation registry；若已空，立即把 `ExitPending` 恢复 Open、清除 abort 和“退出仅执行一次”状态并唤醒 scheduler/UI；若仍非空，最后一个 operation finalizer 在移除自身的同一临界区执行相同恢复。随后返回原稳定错误。这避免“最后一个 operation 恰在 abort 标记前完成”导致永久 ExitPending。
4. 稳定 bootstrap/root 读取、system app data 解析或原子写 `shutdown-intent.json` 任一步失败都走同一个 `abort_exit_before_marker`。marker 是受控退出线性化点；写成功前绝不取消 queued/running 或停止 worker。Windows 在此窗口强制结束仍按普通崩溃恢复。
5. marker 持久化成功后把 admission 切换为 `Exiting`，先停止 scheduler worker/dequeue，再对 running sync/login 发出协作取消；此时不清扫 queued，已获准 importer 仍可用 continuation 完成终态事务。
6. 等待最多 2 秒后终止准确匹配身份的 collector/browser 进程树；终态竞争遵循 5.4 的 CAS 规则，已成功导入的 job 不得被退出覆盖为 cancelled。
7. `AppState` 保存 scheduler worker、sync phase monitor、login watchdog 和 maintenance 的 owned join handles/cancellation tokens；退出等待 running/monitor/importer/continuation 全部完成或达到 10 秒总截止时间，防止 collector 已结束但 monitor 仍在导入 SQLite。
8. continuation 全部解决后执行最终 queued 清扫，将包括 importer 新建 child 在内的全部 queued 条件 CAS 为 cancelled，并同步更新 batch snapshots/status。随后确认数据库无 queued/running，清理任务文件并删除 shutdown intent。
9. 正常路径标记 shutdown complete 并调用 `app.exit(0)`。若 10 秒总截止时间先到，保留 marker，不标记 complete，完成准确进程树 best-effort 清理后直接终止当前进程；下次启动按受控退出中断恢复。无论哪条路径，进程退出后都不存在 detached monitor。

`RunEvent::ExitRequested` 在清理未完成时阻止首次退出；清理完成后的第二次退出不再阻止。Windows 会话强制结束若发生在 marker 写入前，下一次启动按普通崩溃恢复；发生在 marker 写入后，才按受控退出中断恢复。

marker 契约固定为：

```rust
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShutdownIntentV1 {
    schema_version: i64,
    application_id: String,
    root_id: String,
    pid: u32,
    process_created_at_100ns: u64,
    started_at: String,
    cancel_queued: bool,
}

#[serde(rename_all = "camelCase")]
enum StartupRecoveryFaultPhaseV1 {
    Detected,
    MarkerArchived,
    RecoveryApplied,
}

#[serde(rename_all = "camelCase")]
enum StartupRecoveryDispositionV1 {
    PreserveQueued,
    CancelQueued,
}

#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartupRecoveryFaultV1 {
    schema_version: i64,
    phase: StartupRecoveryFaultPhaseV1,
    reason: String,
    marker_snapshot: Option<ShutdownIntentV1>,
    marker_raw_sha256: String,
    disposition_applied: Option<StartupRecoveryDispositionV1>,
}
```

进入 `recover_sync_state_once` 时卸载 journal/fault 已由 6.0 处理完毕，且当前 data root 和 schema 已固定；该阶段内部必须先读取 shutdown intent：

- `application_id` 和当前 bootstrap `root_id` 必须完全匹配，否则先原子写入并落盘 phase=`Detected` 的 `startup-recovery-fault.json`，其中保存可解析 marker snapshot、原始内容 SHA-256 和稳定 reason；fault 成功后才把 normal marker 原子改名为 `shutdown-intent.invalid.json`，再把 fault phase 更新为 `MarkerArchived`。任一步失败都保持失败关闭，禁用自动调度并显示主窗口，不操作任何数据库或进程。
- PID 仍存在且 Windows 创建时间匹配时，视为旧进程仍在收尾，新的 GUI 返回 `shutdown_in_progress`；正常单实例 mutex 应更早阻止这一情况。
- PID 不存在，或 PID 已复用且创建时间不匹配时，绝不终止该 PID；按“受控退出被中断”的 `CancelQueued` 语义恢复当前 root：先处理 running 的合法完整结果，否则取消 running，再取消 queued、清理已记录的准确 collector/browser 身份，最后删除 marker。
- 启动判定顺序固定为 fault、invalid marker、normal marker、普通崩溃恢复。只要 fault 或任何孤立 `shutdown-intent.invalid.json` 存在，每次启动都保持失败关闭；invalid 存在而 fault 缺失时先重建 fault，绝不回退普通崩溃恢复。
- UI 提供“保留排队任务并恢复”与“取消排队任务并恢复”两个明确操作，调用 `resolve_startup_recovery_fault(disposition)`。后端以 fault 内 snapshot/hash 为持久证据；外部 normal/invalid marker 若仍存在必须匹配，缺失时也可继续 fault-only 幂等恢复。
- disposition 对当前 root 的 queued/running 处理成功后，先原子更新 fault phase=`RecoveryApplied` 并保存 disposition，再删除/归档 normal 与 invalid marker，最后删除 fault。若在任一步中断：Detected/MarkerArchived 重复执行幂等 disposition，RecoveryApplied 只重复文件清理，不再要求 marker 存在，也不重复改变数据库语义。
- 所有恢复路径先对每个 running job 检查任务身份、payload/hash、结果 envelope 和完整结果文件：合法完整结果优先经原 importer/CAS 导入，这里的“完整”指文件写入完整且通过契约校验，archive result 可以是合法 Complete 或 Incomplete；没有合法结果时，`PreserveQueued` 把该 started job 转 failed，`CancelQueued` 把它转 cancelled，禁止恢复为没有 orchestration 的 running。两种 disposition 下这些已经 started 的终态都按 6.2 的既有完整 task-key 锚点规则参与下次到期计算。
- `PreserveQueued` 只把通过账号、capability、payload/hash、browser/profile 和 automatic config 校验的 queued job恢复到内存队列；校验失败的 queued 仍按稳定 reason 取消。`CancelQueued` 把全部 queued 条件 CAS 为 cancelled，且 `started_at IS NULL` 的任务永远不作为调度锚点。每次终态变化都在同一事务更新关联 batch snapshot/status。
- marker 不存在且 fault 不存在时才按异常崩溃规则执行隐式 `PreserveQueued`；结构损坏同样生成持久 fault，不猜测用户意图。无论使用哪条路径，scheduler worker 启动前都必须断言数据库不存在 running job，随后才重建内存队列并开始 dequeue。

### 7.4 桌面意图握手

`AppState` 保存单调 revision 的 `DesktopIntentState`，至少包含 `show_main` 和 `open_settings`。托盘或第二实例先更新该状态，再发送事件；前端注册事件监听后立即调用 `take_desktop_intents(last_seen_revision)` 拉取，避免 Svelte 尚未挂载时丢失“打开设置”。消费操作按 revision 幂等，重复事件只执行一次。

## 8. 首次引导与设置 UI

### 8.1 引导流程

首次引导是主窗口内的四步全屏覆盖层，不创建额外窗口：

1. **账号**：执行现有“一键登录”，允许跳过；提前关闭浏览器仍可重试。
2. **同步范围**：观看历史设置最近 N 天或到底端；喜欢/收藏显示全部、最近 N 条或记录上限的范围控件，并根据后端 capability 禁用尚未实现的来源；允许选择完成引导后立即同步可用来源一次。
3. **自动同步**：每个来源独立开关和间隔，默认关闭；不可用来源不能开启；同时设置开机后首次同步延迟。
4. **桌面行为**：设置关闭到托盘或退出、是否开机启动、开机后是否静默，最后确认完成。

- 每步完成后保存 `onboarding_step`，中途退出下次从该步继续。
- 跳过账号时仍可完成引导，主界面显示未配置账号状态。
- 没有账号时保存到 `account_defaults`；后续添加账号继承该范围和 schedule。
- “立即同步一次”只在账号登录有效时执行，失败不撤销引导完成状态。
- 引导完成后所有选项都能在设置页面修改。

### 8.2 设置页面

在现有设置浮窗增加：

- 自动同步：观看历史、喜欢、收藏各自的能力状态、范围、启用开关、间隔数值与单位，以及开机后延迟和后端返回的上次/下次同步状态。
- 桌面行为：关闭行为、开机启动、静默启动。
- 当前账号浏览器：自动、Edge、Chrome；实际浏览器锁定与偏好冲突时显示“重新登录后生效”，不影响已有归档。
- 调度控制：暂停/恢复自动同步。

主界面的数据来源切换器旁提供“同步当前来源”命令，调用 `scope = One`；账号菜单或托盘提供“同步当前账号全部来源”。3.4 增加来源页面时必须复用该入口，不另造绕过队列的同步命令。

job/batch 为 partial 时，设置页和同步状态必须展示 `issues` 的稳定中文说明；Favorites 自动同步关闭时，RemoteFolder 设置旁提示“仍会同步已知收藏夹，但不会自动发现新收藏夹”。

账号范围、隐藏浏览器和滚动速率继续属于当前账号；开机启动和静默启动属于本机；关闭行为和启动延迟属于全局 portable settings。

### 8.3 前端事件

- `desktop:intent-changed`：提示前端调用 `take_desktop_intents`，后端状态才是事实源。
- `scheduler:changed`：刷新队列和下次同步状态。
- `sync:changed`：刷新当前 job 与历史列表。

前端不持有后台计时器作为事实源；当前 500ms 同步轮询改为仅在窗口可见且当前 job 为 queued/running 时启用。窗口可见且无活动任务时最多每 60 秒刷新一次 scheduler 状态；窗口隐藏时停止全部非必要前端 interval，只响应后端事件。

## 9. 通知

使用 Rust `tauri-plugin-notification`，通知失败只记录脱敏日志，不回滚状态。

- 登录失效：自动任务检测到最终恢复失败后通知一次。
- 连续自动同步失败：同一完整 task key 连续 3 个 automatic job failed 时通知一次；success、partial 或用户取消会结束本轮失败 streak。
- 首次隐藏到托盘：只提示一次。
- 手动同步成功、普通自动同步成功和单次技术失败默认不发送系统通知。

备份失败通知在 3.7 接入同一通知服务。

## 10. 错误与恢复

- device settings 无效：拒绝静默开机和自动同步，显示主窗口及稳定错误，不覆盖文件。
- portable settings v1 迁移失败：继续保留原文件并停止调度，不用默认值覆盖。
- 注册表写入失败：返回 `autostart_registry_update_failed`，设置 UI 回滚开关；device settings 写入失败时恢复注册表原值，补偿失败返回 `autostart_state_uncertain`。
- 队列数据库事务失败：内存队列不得先行变化。
- executor 启动失败：queued job 原子转 failed，释放活动唯一约束并继续下一任务。
- 数据根迁移：严格按 6.4 admission/barrier 顺序停止入队、等待 running、取消 queued，再取得 runtime maintenance；迁移提交后 worker 只从新 root 读取。
- 账号归档或删除：同事务前取消 queued job；running 继续沿用“账号有任务时禁止修改”的边界。
- 账号登录失效：自动任务不无限恢复；使用现有一次隐藏恢复和一次采集重试，仍失败后标记账号并通知。
- 非更新卸载由复制到当前用户临时目录的长生命周期 uninstall guard 协调。guard 取得 GUI 单实例 mutex 和独立 uninstall mutex；若 GUI 已运行则返回 `application_still_running` 并中止。成功后通过当前用户 ACL 的 named event/父进程 handle 与 NSIS 握手，并持续持有两个 mutex，直到 NSIS 明确发送“完成/中止”或安装器进程退出，覆盖程序文件删除、数据 helper 和最终收尾的整个卸载窗口。
- guard 保存旧 device settings 与 Run 值，先原子写入 `launch_at_startup = false`、`silent_autostart = false`，再删除并回读 HKCU Run 固定值。第一步失败不碰注册表；删除或回读任一失败都必须先恢复旧 Run 值并回读确认，再恢复旧 device settings，让 guard 返回失败并中止非更新卸载。任一补偿步骤失败都原子写 `autostart-cleanup-fault.json` 并中止；补偿仅恢复原状态，不能把失败视为可继续。
- NSIS 和后续数据 helper 必须携带 guard session nonce 并确认 guard 仍持锁；失去 guard 时立即中止卸载，不能在互斥空窗继续删除。下次启动看到 cleanup fault 时强制保持 autostart disabled、尝试删除 Run 并显示错误，绝不根据旧 enabled 字段重新启用。
- 正式打包在最终 NSIS staging 完成后生成版本化 `uninstall-package-manifest-v1.json`，其输入只能是构建系统声明的“允许卸载删除”的 package-owned 文件，不得在用户机器上扫描安装目录临时生成。manifest 使用规范 JSON，固定包含 schema=1、application ID、product name、build ID、semantic version、architecture，以及按安装目录相对路径排序的文件长度/SHA-256；用户数据目录、协调目录、manifest 自身和任何安装时后生成文件都不进入删除清单。manifest 字节的 SHA-256 由打包步骤写入 NSIS 固定 metadata，安装器通过已认证的 nonce/handle 握手把原始 manifest 字节和预期 hash 交给 guard；guard 必须先复核 hash、产品字段和实际 package-owned 文件，再允许写 Preparing journal。运行时目录枚举、仅相信安装目录内可修改的 manifest 副本或根据当前文件反推 build ID 均非法。
- guard 在首次修改前先构造不可变卸载身份快照，再向独立于安装目录和可选 app-data 删除范围的当前用户协调目录原子写入 `uninstall-session.json`。journal 必须包含 schema、nonce、installer PID/创建时间、root ID、旧 Run 存在性/值、旧 device 字段、phase，以及规范安装目录、规范主 EXE 路径、`application_id`、`product_name`、旧安装 build ID、旧安装语义版本、`windows-x86_64` architecture、排序后的受保护路径、用户明确选择的“保留/删除应用数据”、安装文件 manifest 和该 manifest 的规范 SHA-256。manifest 每项固定为安装目录相对路径、字节数和 SHA-256，拒绝绝对路径、`..`、重复项、符号链接和 reparse point；受保护路径至少包含协调目录、选择保留的数据根及任何不属于 manifest 的用户数据目录。目录使用当前用户 ACL。phase 固定为 `Preparing | Prepared | DestructiveCommitted | Completed | RepairCompleted`；上述身份和选择必须与 Preparing 一起先落盘，完成自启清理后才能更新 Prepared。
- DestructiveCommitted 前，guard 使用现有精确 PID/创建时间/EXE/profile 身份规则清理 collector/browser，执行一次 uninstall recovery 事务取消 queued/running、更新 batch snapshot/status，并确认无 collector、browser、GUI、sync monitor 或 DB writer；仅取得 GUI mutex 不算通过。随后 NSIS 发送带 nonce 的 commit，guard 持久更新 DestructiveCommitted 后才允许删除第一个不可逆程序文件。
- Preparing/Prepared 阶段用户取消、guard/NSIS/系统崩溃或父进程异常退出时，当前 guard或下次应用/guard启动必须依据 journal 按“恢复旧 Run 并回读、再恢复旧 device settings”幂等回滚；任一步失败写 cleanup fault。DestructiveCommitted 后中止或崩溃不得恢复自启，保持 disabled 并依据 journal生成 `uninstall-interrupted-fault.json`。
- 后续 NSIS install/uninstall 启动必须先读取 DestructiveCommitted journal并接管为新的 guard session。新 guard 在取得 mutex 后必须重新规范化目标安装目录/EXE，复核 journal 的 `application_id`、`product_name`、旧 build/version、architecture、root ID、manifest 总 hash，并对 manifest 中仍存在的文件逐项验证路径归属、长度和 SHA-256；旧 build/version 通过 journal manifest 与仍存旧 EXE 的嵌入元数据交叉验证，不要求执行修复的新版安装包具有相同版本。候选安装包自身必须具有相同 application/product/architecture，且当前受保护路径和用户数据选择必须与 journal 完全一致。任一字段缺失、产品身份不匹配、路径越界、manifest 外删除目标或保留数据路径落入删除集都失败关闭，只保留 journal，不继续删除或修复。复核通过后，选择继续卸载时才从 journal/文件探测结果向前完成删除并写 Completed；选择安装/修复时先完成并验证全部程序文件、resources、schema/bootstrap 可启动性，保持 autostart disabled，写 RepairCompleted。两个终态都清理 interrupted fault 和 journal；不得仅因新安装开始就删除证据。
- 应用正常启动必须先检查 uninstall journal：Preparing/Prepared 且可执行文件仍存在时完成回滚后才继续；DestructiveCommitted 时保持失败关闭并提示运行安装器“继续卸载或修复”；Completed/RepairCompleted 仅做残留清理。可选“删除应用数据”不得提前删除该协调目录或 journal。
- 所有 installer 模式都必须在分支到 `/UPDATE` 之前检查协调目录。存在 Preparing/Prepared journal 时先完成并验证回滚；存在 DestructiveCommitted journal、`uninstall-interrupted-fault.json` 或 `autostart-cleanup-fault.json` 时，`/UPDATE` 不得跳过 guard，而必须进入同一身份复核和“修复”接管流程，失败则中止更新。只有不存在任何非终态 journal/fault 时，普通 `/UPDATE` 才跳过非更新卸载 helper，并在新版本首次启动时修复 EXE 命令。用户选择删除应用数据时，现有严格 helper 随后删除 system app data；选择保留时保留已关闭自启的 device settings。helper 在 device write 后异常退出但尚未删 Run 时，下次启动的 disabled 双向核对负责删除残留值。

## 11. 测试

### 11.1 3.3 Rust 单元与集成门禁

- portable settings v1 到 v2 的字段保留、默认值、未知字段和边界值。
- device settings 原子读写、损坏文件、引导兼容判定和 revision 递增。
- HKCU Run 命令生成、引号、启停、回读和路径修复；真实注册表测试使用唯一测试值并保证清理。
- schema v5 从 v0 到 v4 的迁移矩阵、原 job ID 与 `watch_records.sync_job_id` 保留、`sqlite_master/foreign_key_list` 最终引用、`sqlite_sequence`、`sync_queue_state`、`remote_folder_registry_state` 初始化/账号新增/溢出、`idx_remote_folders_scheduler`、`sync_batch_targets` 复合外键、`foreign_key_check`、全局 single-running、queued/running 同源和完整 task-key 终态索引及回滚。
- v4 terminal payload 字段保持 NULL，v4 running 安全终态化并清理旧进程，不伪造历史参数。
- payload 完整 hash 与 `SyncExecutionEquivalenceV1` schema v1 golden vectors：固定 tag/字段顺序、显式 null、无空白 UTF-8、override 排序数组、不同 map 插入序同 hash、全部嵌套 RecentDays cutoff、非法 range 组合、纯 frozen time 排除、source/instance 交叉校验；创建、恢复、附着/去重、promotion/replacement、child、dequeue 和 importer 每个生命周期节点都重新计算，settings revision 仅审计而不比较当前 revision。
- `AutomaticSyncConfigV1` schema v1 golden vectors和逐字段变更矩阵：各 source 固定 tag/顺序、History/Likes/Favorites 空 instance、RemoteFolder 有效 override/default 解析、Incremental 常量、Favorites discovery bool、scroll/hide/browser preference 均参与；settings revision、cutoff、resolved browser/profile、通知/显示字段排除。创建、设置协调、恢复、dequeue、promotion/replacement 使用同一构造器；manual 的 `config_sha256 = NULL` 且设置变化不取消冻结任务，source/instance-specific automatic hash 只使对应 task key 失效。
- implicit manual payload 和全部 Favorites discovery child 始终 Incremental；显式 FullReconcile 只允许 `scope=One` 的顶层 Likes、Favorites 或单个 RemoteFolder，验证该顶层 range=All 并用完整 payload SHA-256 confirmation 防止授权后篡改或 bulk 绕过；Favorites defaults/overrides 只有 range，绝不递归携带或授权 run mode。
- remote-folder 的 `source_instance_key` 隔离唯一约束、终态、下次到期和通知 streak；payload 中 folder ID 不匹配时拒绝。
- 队列等价 payload 去重、non-equivalent 冲突、automatic 替换/提升 manual、持久 queue_seq FIFO、重启排序一致性、序号溢出、手动优先、取消、崩溃恢复和受控退出清理；未开始取消不得成为时间锚点。
- IPC scope 字段矩阵、instance/payload 交叉校验、skip/conflict 和 queued position 返回完整。
- batch 共享 entry_seq、首屏 200、active batch 动态追加后的继续分页、cursor 篡改；`sync_jobs` account/source/instance 三字段分别及组合更新均被拒绝且 promotion 不修改 task key；终态 target 插入、target 更新/直接删除被拒绝；`sync_batch_targets` 对同一 batch/task key 的 job/issue 跨表唯一保留；`sync_batch_jobs` 第二关联、初始 null、错 task key、batch/entry/role/identity 重挂、rebind、手工 unlink、直接删除均被拒绝且父 job 删除后只置 null保留审计；conflict 初始 null/错 key job、batch/key/证据修改、rebind、手工 unlink、直接删除被拒绝而父 job 删除后保留 snapshot。删除父 batch 时 target/job/issue 仍完整级联；初始与 discovery 并发唯一冲突重读，以及 active batch/job attachment/batch target/RemoteFolder schedule 四类运行时上限。
- batch task 上限验证 4,999+1 全部接纳、4,999+2 all-or-none aggregate、已有 5,000 时仍可写不计数的第 5,001 个 aggregate entry、5,001 初始 preflight 不建 batch，以及并发 discovery 最终只有一个 aggregate 且 expansion closed。
- batch 初始 job、等价附着、跨 batch 复用、job snapshot 和冲突发生时快照在重启/账号删除后仍能还原；batch queued/running/success/partial/failed/cancelled、warning-only、incomplete 和 terminal trigger 状态矩阵。
- 到期计算、失败/取消锚点、间隔变更、睡眠跨越和最多一次补跑。
- RemoteFolder registry state + portable settings 双 revision cursor、registry 每事务只递增一次、无实际写入不递增、i64 溢出全事务回滚、SQLite read transaction 与 settings double-read 在任一侧变化时稳定拒绝旧页；scheduler 只查询 aggregate 与到期候选。
- 暂停在 device settings 写入成功/失败竞态下都不会启动 queued automatic；running automatic 继续、manual 不受影响、恢复只补一个。
- `OperationAdmission` 同时门禁队列与登录；maintenance 登录冲突不改变 queued job，running 等待超时恢复 admission，路径租约无死锁。
- source settings 更新与 queued→running 最终校验线性化，关闭或改范围后旧 automatic 不能抢跑。
- 登录与队列 worker 共享全生命周期 browser operation lease，不并发启动浏览器。
- resolved browser/profile generation/executable path hash 冻结和 dequeue 复核；存在 active job/lease 时浏览器偏好或 profile reset 被 admission gate 拒绝。
- 命名激活事件、manual/autostart 第二实例差异和退出状态机。
- 全局启动门禁严格证明卸载 journal/fault 在 bootstrap/data root/SQLite 前处理；Preparing/Prepared 回滚、DestructiveCommitted 失败关闭、结构/身份错误最小恢复 UI 均不打开数据库或启动 worker。随后 bootstrap/root、schema、shutdown recovery、无 running 断言、托盘/窗口、scheduler 的唯一顺序不得交换。
- bootstrap/root 读取和 durable shutdown intent 写入失败统一 abort finalizer、恢复 Open 并重置退出状态且不取消任务；在线性化点前后被终止时，启动恢复分别走崩溃恢复或受控取消路径；application/root/PID/创建时间不匹配时不误杀复用进程或取消错误数据根。
- invalid shutdown marker 转 fault 的每个文件操作崩溃点、Detected/MarkerArchived/RecoveryApplied phase、孤立 invalid、fault-only、hash 不匹配和最终删除顺序；跨多次重启保持失败关闭，只有两种 user disposition 成功后才清除。
- startup recovery 的 PreserveQueued/CancelQueued 矩阵：running 有合法完整结果时两者都先导入；无结果时分别 failed/cancelled；queued 分别校验保留或全部取消；started 与未 started 任务的调度锚点不同；worker 启动前数据库始终无 running。
- 托盘菜单决策函数、持久桌面意图握手与通知 streak 规则。
- capability 固定返回全部来源，未实现执行器保持稳定 unavailable 原因；remote-folder payload 与逐收藏夹设置一致。
- 默认隐藏窗口在 startup 决策前不可见；manual、silent autostart、引导未完成和托盘失败四种矩阵。
- autostart 注册表补偿、启动核对、非更新卸载长生命周期 guard/nonce、Preparing/Prepared/DestructiveCommitted/Completed/RepairCompleted journal、构建期 NSIS manifest/固定 metadata hash/拒绝运行时扫描、完整产品/路径/manifest/用户选择身份、接管逐项复核与 mismatch 失败关闭、guard/NSIS/系统崩溃恢复、向前继续卸载/修复、collector/browser/job 清场、device-first 清理、旧 Run 回写回读和补偿 fault；`/UPDATE` 在无非终态 journal/fault 时才允许跳过 helper，否则必须回滚或修复接管。
- 非更新卸载保留数据时仍把 device autostart 字段写为 false；helper 各步骤失败或进程中断后，重装不会恢复自启。
- 所有 payload/range、enqueue/batch entry、execution envelope/result、archive completion/stop 和嵌套 record 的 serde 严格字段；任意层级额外字段、alias、错误 tag、错误 instance 和篡改 hash 均稳定失败。
- result import 与 cancel/exit/recovery 的条件 CAS 竞争，失败方不得写来源数据、覆盖终态或丢失 batch snapshot。
- continuation token 在 maintenance/exit 下允许已获准 importer 收尾但禁止外部 enqueue；SQLite writer 两种先后顺序、maintenance 最终 queued 取消和 exit 最终 queued 清扫均不遗留 child。
- source/device/browser owned operation 在 maintenance/exit 中的拒绝、取消、完成和超时恢复；maintenance 超时不写 marker，最后 owner 收尾后恢复 Open。
- maintenance 在切换 Pending 前已原子登记 owned placeholder；线程启动失败、exit 并发和 abort finalizer 都看见同一 registry。
- exit timeout 与最后一个 owned operation 同时完成的临界区竞态，必须立即或由 finalizer 恢复 Open 并重置单次退出状态。
- `OperationAdmission` 的 maintenance/ExitPending/Exiting 竞态和 owned worker/monitor handle 收尾。

### 11.2 3.4 延后执行的契约门禁

以下测试在 3.3 只冻结接口和测试夹具，不作为 3.3 完成条件；3.4 注册 archive executors/importers 后必须全部通过：

- Favorites 完整 discovery 在单事务中完成 archive 导入、folder upsert、按 task key 一次决策、向全部关联 batch 执行 child 创建/提升/替换/附着/conflict、父 job success 和 batch 重算；事务任一点失败不得留下半个 child 集合。
- 初始 bulk 已关联 queued/running/terminal RemoteFolder、已删除 job 的保留 snapshot、初始 skip/conflict，以及两个并发 discovery 都不能为同一 batch/task key 生成第二个 association、issue 或执行；只有首次新发现 key 可创建 Incremental child。
- PerFolder payload 冻结 mode/defaults/全部已配置 ID overrides；removed folder 重新出现时若仍有配置则使用冻结 override，其余已知/未知 ID 回退 defaults；child 仅继承 frozenAt 并按自己的 RecentDays 重算 cutoff，设置变化不污染 child；registry 与 override 上限分别验证。
- batch attachment 上限按 task key all-or-none、候选 batch 排序无关，以及 aggregate limit skip 后 discovery 不再补 child。
- Favorites discovery 三布尔值/长度的三种唯一合法组合及全部非法组合；不完整或达 1,000 上限时不创建 child、父 job 以 partial 和持久 job issue 完成且不标记收藏夹移除；只有完整组合才允许 child，且只有 manual FullReconcile 与既有完整性证明同时成立时允许远端移除。
- 完整 discovery 标记 folder removed 时取消对应 queued child 并重算 batch，scheduler/bulk 不再展开；重新出现时清除 removed 并恢复调度。
- `ArchiveCollectionSuccessV1` 和 `RemoteFolderDiscoverySuccessV1` 的 identity/field hash、记录上限、规范 URL、连续 position、重复 ID、字符串/数字边界，以及 `notRequested | success` 与 payload discovery flag 的严格契约。
- ArchiveCompletion 与 payload 唯一矩阵：count 达限 stop 优先级、Bottom 严格少于限额、RecentDays 全记录 cutoff、StableKnownBoundary 导入前 DB 尾部连续命中、SafetyRecordCap/UnconfirmedEnd 范围约束，以及语义非法和技术错误 failed/cancelled。
- RecentDays 在 50,000 条处的唯一优先级：第 50,000 条恰好确认底端为 Complete Bottom；第 50,001 个未入数组 guard 越过 cutoff 为 Complete DateBoundary；两者均未确认或 guard 仍在范围内才是 SafetyRecordCap partial，并验证结果数组始终恰好 50,000 条且全部满足 cutoff。
- 四个 result tag、全部嵌套 struct 的 camelCase/未知字段拒绝、source/instance/payload hash 交叉校验；错误 folder ID、非法 SHA-256 和篡改字段稳定失败。
- Archive importer 来源事务失败不污染 job、batch 或其他来源；RemoteFolder capability 在执行器注册后即 available，零发现实例保持合法空列表。

### 11.3 前端测试

- 全新安装显示引导，升级用户不重复显示。
- 四步引导保存与恢复、跳过账号、账号默认设置和立即同步选项。
- 设置页可修改自动同步、间隔、关闭行为、开机启动和静默启动。
- queued/running/terminal 状态、partial/job issues、取消按钮和 scheduler 事件刷新。
- V2 source/range/status 字符串与 Rust serde 完全一致，queued position、skip/conflict 和通用计数可渲染。
- batch entries 和 RemoteFolder scheduler instances 按 200 条分页加载，超限 aggregate error 不冻结设置页。
- 手动单来源、当前账号全部可用来源、全部 active 账号全部可用来源不受 auto enabled 影响。
- 浏览器偏好和 `RecentDays` 根据 capability 正确启用、拒绝和提示重新登录。
- 无账号、登录失效、maintenance 和设置保存失败状态。
- `desktop:intent-changed` 到达前端挂载前，`take_desktop_intents` 仍能打开设置且不会重复执行。

### 11.4 安装包与人工验收

- 手动启动始终显示；静默开机启动无主窗口、白色浏览器窗口或任务栏按钮。
- 第二次手动启动只唤起现有窗口，不创建第二个 WebView 或数据库连接。
- 关闭到托盘、恢复、设置退出、托盘退出和 Windows 重启后的行为正确。
- 设置 15 分钟最小间隔进行两轮自动同步，手动任务优先且无重复浏览器。
- 睡眠跨越多个周期后只运行一次。
- 退出时 running collector/browser 在 2 秒宽限后无残留。
- 空闲 10 分钟不启动系统浏览器；按 5 秒采样应用进程及其 WebView2 子进程树，私有工作集总量中位数不超过 180 MiB、p95 不超过 250 MiB，应用 CPU 平均低于 0.5%、p95 低于 2%，调度器唤醒不超过每分钟 3 次。使用同一窗口内容和采样脚本建立临时 Electron 对照，Emily 中位私有工作集须不高于对照的 80%；报告记录采样时段、进程树、采样数和汇总值，不记录账号或 profile。
- 无 Node PATH 执行 `cargo tauri build`，安装目录扫描继续不包含 Node、NCC、CRI、Playwright 或随包浏览器。

## 12. 交付与阶段边界

实施按以下顺序完成：

1. 设置 schema v2、device settings 与 autostart 注册表。
2. schema v5、持久队列和纯调度计算。
3. history executor 接入队列、取消和启动恢复。
4. 托盘、单实例激活和受控退出。
5. 首次引导、设置页和状态事件。
6. 通知、资源测量、安装包及验收报告。

3.2 的真实目录、覆盖升级、卸载和干净 Windows 人工门禁仍是发布阻塞项；在这些门禁未完成前可以实现和自动测试 3.3，但不得将 3.3 或整个第三阶段标记为最终可交付。

3.3 自动与生命周期门禁通过时只能表述为“观看历史生命周期完成，多来源设置/队列/结果契约已冻结，archive 执行器与最终多来源验收待 3.4”，不得宣称 likes/favorites/remote-folder 已可用。

只有 3.2 人工门禁、3.3 生命周期门禁和 3.4 的 likes/favorites/remote-folder 执行器及 11.2 契约门禁全部通过后，“多来源调度”验收才算关闭。安装包视觉美化不在本阶段执行，待用户下一轮确认。
