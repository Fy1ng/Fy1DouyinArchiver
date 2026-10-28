# 账号登录失败恢复与受控浏览器设计

## 状态

已确认，作为后续实施计划的需求基线。

## 背景

当前多账号登录流程在启动浏览器前创建并持久化 `pending_login` 账号。登录浏览器被用户提前关闭时，采集器可能继续等待身份结果，前端长期保持 `accountBusy`，同时失败的 pending 账号会出现在账号管理界面。设置弹窗又位于“已有活动账号”条件分支内，导致首次添加账号前无法打开设置。

应用启动的 Edge/Chrome 目前只使用 `--no-first-run` 和 `--no-default-browser-check` 等基础参数。全新 Edge profile 仍可能继承 Windows Microsoft 账号登录状态并显示同步提示，插件、浏览器账号登录、同步和后台驻留也没有统一关闭。

## 目标

- 登录校验成功前，账号不得出现在账号菜单或账号管理界面。
- 新账号登录失败、取消、超时或浏览器关闭后，立即恢复再次添加能力，并删除全部临时状态和 profile。
- 已有账号重新登录失败时保留账号、历史数据和原 profile。
- 无活动账号时设置按钮、设置弹窗和全局错误提示仍可使用。
- 所有应用启动的 Edge/Chrome 统一禁用扩展、浏览器同步、浏览器账号登录入口和后台驻留，同时保留抖音等普通网站登录。
- 所有约束只作用于应用专属 profile，不修改用户系统浏览器的普通 profile 或机器级、用户级浏览器策略。

## 非目标

- 不在本阶段重新设计完整首次启动引导页。
- 不新增大型登录进度窗口。
- 不修改 HKLM/HKCU 下 Edge/Chrome 企业策略，因为这些策略会影响用户正常使用的系统浏览器。
- 不支持在登录失败后恢复未完成的临时 profile；每次重试都从全新 profile 开始。

## 账号生命周期

### 新账号登录

新账号继续使用数据库内部 `pending_login` 行来取得稳定的 `account_id` 和 `profile_key`，避免重写现有 collector CLI、任务文件和恢复契约。该行只表示内部登录尝试，不是可管理账号。

1. `begin_account_login` 创建 pending 行、任务文件和独立 profile。
2. 所有公开账号列表始终排除 `pending_login`，包括账号管理使用的 `includeArchived=true` 查询。
3. collector 启动可见浏览器并等待经过校验的抖音远端身份。
4. 身份校验成功后，单个数据库事务补全 `uid/sec_uid`、昵称、头像和登录验证时间，并将状态切换为 `active`。
5. 数据库事务提交是不可逆的登录成功点。提交后该账号可以显示，后续任何文件写入失败都不得再进入新账号失败清理或删除 profile。
6. 提交后原子写入 `device-settings.json` 将新账号设为当前账号；首次失败立即重试一次，仍失败则记录非致命 `device_settings_update_failed` 诊断并返回已激活的 `AccountView`。前端将本次添加视为成功，设置文件可由后续切换账号操作再次修复。

### 新账号失败清理

以下情况统一视为新账号添加失败：用户关闭浏览器或 CDP Target、显式取消、十分钟超时、浏览器启动失败、身份不可用、身份冲突、重复远端账号、结果写入失败和 collector 异常退出。

所有清理步骤均为 best-effort：记录首个稳定错误，但不得用 `?` 提前跳过后续步骤。内存状态释放和 pending 行删除位于不可跳过的 guard/finally 路径。清理顺序固定为：

1. 发出协作取消并关闭 CDP 会话。
2. 终止仍存活且身份完全匹配的 collector/browser 进程。
3. 清除内存中的 `running_login`；浏览器操作锁由 RAII guard 在命令返回时释放。
4. 在事务中删除 pending 数据库行，保证失败尝试不可见且不会在后续查询中复活。
5. 删除任务级 identity、PID、cancel、diagnostic 和 log 文件。
6. 删除受控 profiles 根下对应的直接子目录；继续执行既有 symlink/reparse 与路径边界检查。
7. profile 因浏览器文件锁无法在截止时间内删除时，写入不含 Cookie 的 `pending-profile-cleanup-<profile_key>.json` 清理标记，由后台和下次启动重试。
8. 向前端返回稳定错误，前端在 `finally` 中恢复添加按钮。

profile 或任务文件清理失败不得让 pending 行重新显示为账号，也不得继续占用浏览器操作锁。错误诊断只记录路径类别和错误码，不记录 Cookie、身份结果原文或浏览器账号信息。

### 启动恢复

应用启动先读取数据库账号状态，再处理任务文件。数据库 pending 枚举独立于 attempt 文件扫描，避免损坏或缺失任务文件掩盖 pending 行。

| 数据库状态 | 进程处理 | 任务文件 | Profile | 数据库行 |
| --- | --- | --- | --- | --- |
| `pending_login` | 仅终止完整匹配 V2 记录、profile 和 Windows 创建时间的进程 | 删除 | 删除；锁定时登记延迟清理 | 删除 |
| `active` / `login_expired` | 仅终止完整匹配的残留登录进程 | 删除本次 attempt 文件 | 保留 | 保留 |
| `archived` | 仅终止完整匹配的残留登录进程 | 删除本次 attempt 文件 | 保留 | 保留 |
| 账号缺失或记录损坏 | 不终止未知进程 | 仅删除能证明属于受控任务的普通文件 | 不根据损坏记录删除 | 不处理 |

清理过程幂等；文件已不存在或进程已退出视为成功。所有 `pending-profile-cleanup-*.json` 标记在正常启动完成前重试，失败时保留标记供下次启动继续处理。

### 已有账号重新登录

`login_expired` 账号继续复用原 profile。提交前的重新登录失败只清理本次任务文件和运行状态，账号保持 `login_expired`，数据库事件、收藏夹和 profile 均保留。只有用户明确删除账号时才删除原 profile。

重新登录与新账号使用相同成功边界：将账号状态提交为 `active` 的数据库事务是不可逆成功点。提交后的 `device-settings.json` 写入失败只记录非致命诊断并返回已激活账号，不得把命令改写为重新登录失败，也不得把状态回退为 `login_expired`。

## 浏览器关闭检测

`CdpConnection` 增加只读关闭状态通道。Browser 连接先执行 `Target.setDiscoverTargets(discover=true)`；Page WebSocket、Browser WebSocket、匹配当前 `target_id` 的 `Target.targetDestroyed`、浏览器进程退出或底层连接结束时立即广播关闭。其他 Target 的销毁事件不得结束本次登录。

登录 CLI 新增任务级 `--login-progress-file`。collector 一旦观察到关闭、显式取消或超时，必须先原子写入 `LoginProgressV1`，再执行自身清理：

```json
{
  "schemaVersion": 1,
  "accountId": 1,
  "loginAttemptId": "...",
  "state": "closing",
  "reason": "browser_closed",
  "observedAtMs": 0,
  "deadlineAtMs": 2000
}
```

`reason` 固定为 `cancelled | browser_closed | timeout | failed`；`deadlineAtMs` 是绝对 Unix 毫秒时间。Tauri 必须重新校验 accountId/attemptId，不信任来自文件的路径、PID 或 profile。

身份等待循环同时监听：

- 身份脚本结果；
- CDP/目标/浏览器进程关闭事件；
- 取消文件；
- 登录硬超时。

关闭原因必须保留到 Tauri 边界：显式取消或应用退出使用 `account_login_cancelled`；Page/Browser/进程关闭使用 `account_login_browser_closed`；十分钟到期使用 `account_identity_unavailable`。清理失败只写附加诊断，不覆盖原始终止原因。

Tauri 不再只用 `spawn_blocking(wait_for_child)` 等待登录 collector，而是每 50 毫秒同时轮询 collector 退出、`login-progress-file` 和 V2 记录对应的浏览器进程是否仍存活。读取到合法 `closing` 后，以 `min(deadlineAtMs, 本地当前时间 + 2 秒)` 建立父进程看门狗；浏览器进程先退出但 progress 尚未写入时，Tauri 以本地时间合成 `browser_closed` 终止原因并启动同一看门狗。文件缺失且浏览器仍存活时保留十分钟登录硬超时。collector 与 Tauri 的清理操作必须幂等，允许双方同时尝试关闭。

从首次关闭信号开始建立覆盖整条取消链的两秒截止时间：

1. 立即关闭发送端并向全部在途 CDP waiter 返回关闭错误，不再等待普通 60 秒命令超时。
2. 最多等待 500 毫秒协作关闭 CDP、collector 和浏览器。
3. collector 仍正常响应但浏览器未退出时，由 collector 依据 V2 进程身份与自身 Job Object 强制结束浏览器进程树。
4. 在剩余截止时间内等待进程退出并释放句柄；不得串行执行两个普通 60 秒 `Target.closeTarget` / `Browser.close` 调用。
5. collector 未在父进程截止时间内退出时，Tauri 强制结束 collector，再依据 V2 记录终止浏览器进程树。
6. 两秒到期前清除 `running_login`、删除 pending 行并让 Tauri 命令返回；浏览器操作 guard 随命令返回释放，前端恢复按钮。
7. 未完成的普通文件与 profile 删除转为带清理标记的后台任务，不继续阻塞 UI。

浏览器参数增加 `--disable-background-mode`，使用户关闭最后一个可见窗口后浏览器不会因后台应用继续驻留。“两秒内恢复”只约束操作锁、pending 可见性和按钮状态；延迟 profile 清理通过后台与启动恢复完成。

## 受控浏览器

### 统一启动参数

登录、可见同步和隐藏同步必须调用同一个参数生成器，并固定加入：

```text
--disable-extensions
--disable-component-extensions-with-background-pages
--disable-default-apps
--disable-sync
--disable-background-mode
--no-first-run
--no-default-browser-check
```

现有 `--user-data-dir`、回环 CDP 地址、随机端口、窗口尺寸和隐藏模式参数保持不变。参数测试必须覆盖 Edge、Chrome、可见与隐藏四种组合。

本设计中的“禁用插件/扩展”明确指：用户安装扩展、默认应用以及带后台页的组件扩展不得加载或执行。浏览器渲染和 CDP 运行必需的内建组件（例如 PDF 查看器、网络错误页和浏览器设置页）允许保留，不作为用户扩展能力。验收使用预先安装在应用 profile 中的测试扩展确认其后台脚本和内容脚本均未运行。

### Profile 偏好

每次启动某个应用 profile 前，幂等执行两类原子 JSON 合并；这既覆盖全新 profile，也约束升级前已经存在的应用 profile。

`Default/Preferences` 固定写入：

- `signin.allowed_on_next_startup = false`；
- `signin.allowed = false`，仅作为兼容旧版 Chromium 的 best-effort 键，不作为验收依据；
- `sync.requested = false`；
- `sync.has_setup_completed = false`；

user-data-dir 根下的 `Local State` 固定写入：

- `background_mode.enabled = false`。

欢迎页和首次运行界面只由已验证的 `--no-first-run`、`--no-default-browser-check`、`--disable-default-apps` 与 `--disable-sync` 参数抑制，不写入未经浏览器源码或真实页面验收确认的私有偏好键。

合并器只更新上述受控键，不重建整个 `Preferences` 或 `Local State`，不触碰 Cookies、Local Storage、IndexedDB 或其他抖音会话数据。文件必须同目录原子替换，拒绝 symlink/reparse、非常规文件、损坏 JSON 和越出当前账号 profile 的路径。`--disable-background-mode`、`--disable-sync` 和扩展禁用参数是本次进程的主要约束，偏好键用于补强并保持 profile 状态。

浏览器账号限制只针对 Microsoft/Google 的浏览器身份功能，不阻止 douyin.com 或其他普通网站登录。

### 能力边界

`--disable-sync`、扩展禁用参数和 Chromium profile 偏好是应用范围内可使用的控制手段。Microsoft 官方文档说明，Edge 的 `BrowserSignin`、`SyncDisabled` 和 `HideFirstRunExperience` 企业策略可以提供更强约束，但 Windows 注册表策略会影响用户的正常 Edge，故本项目禁止使用该回退。

Edge/Chrome 真实页面验收是发布门禁，不定义无法由运行时可靠探测的 `browser_profile_constraints_unsupported` 命令错误码。若某个系统浏览器版本忽略应用 profile 的登录限制或仍显示同步提示，则在验收报告中记录浏览器版本、Windows 账号状态和失败现象，并保持本阶段未完成；不得静默修改全局策略，也不得宣称该浏览器已满足约束。

参考：

- Chromium `chrome_switches.cc`：`disable-sync`、`disable-extensions`、`disable-background-mode`、`disable-component-extensions-with-background-pages`。
- Chromium `prefs::kSigninAllowed`：`signin.allowed`。
- Microsoft Edge `HideFirstRunExperience`、`BrowserSignin` 和 `SyncDisabled` 策略文档。

## 前端行为

- 设置弹窗移到活动账号条件分支之外，无账号、登录中和登录失败状态均可打开。
- 全局错误与通知区域同样移出活动账号分支。
- 所有添加入口共享 `accountBusy`；登录中显示“等待登录…”，防止重复提交。
- 设置按钮和非账号相关界面不因 `accountBusy` 禁用。
- 新账号失败显示“登录已取消，未添加账号”或对应中文错误，随后立即恢复“添加账号”。
- 账号列表与账号管理不渲染 pending 项，也不提供“继续登录/取消 pending”入口。
- 保留内部取消命令供应用退出和后续登录进度界面复用，本阶段不新增大型浮窗。

## 接口调整

- `begin_account_login() -> Result<AccountView, String>` 前端签名保持不变，只在激活成功时返回账号。
- `list_accounts(includeArchived)` 无论参数为何都不返回 `pending_login`。
- 登录 collector CLI 增加 `--login-progress-file`，只接受由 Tauri 构造的任务级应用数据路径；`LoginProgressV1` 使用 `deny_unknown_fields` 并绑定 accountId/attemptId。
- 新增内部/前端可调用的 `cancel_account_login()`，目标为当前唯一运行中的登录尝试，不接受前端路径或 profile 参数。
- `cancel_pending_account(accountId)` 不再用于普通新账号失败恢复，可暂时保留兼容已有开发数据，后续确认无调用后删除。
- 新增稳定终止错误码：`account_login_cancelled` 和 `account_login_browser_closed`，由前端映射为中文提示。`account_login_cleanup_failed` 只作为附加诊断，不覆盖原始终止原因，也不延迟按钮恢复。
- 数据库从 `pending_login` 提交为 `active` 后，`begin_account_login` 必须返回成功；后续 `device_settings_update_failed` 通过日志/通知报告，不得改写为添加失败。

## 测试

### 自动测试

- pending 账号在普通和管理查询中均不可见。
- 新账号浏览器关闭、取消、超时、身份冲突和 collector 启动失败后，数据库行、profile 与任务文件全部删除。
- 启动恢复清理残留 pending，且不会终止身份不匹配或 PID 复用的浏览器。
- 已有账号重新登录失败不删除账号、历史和 profile。
- 数据库激活提交后设备设置写入失败仍返回成功，不删除 active 行或 profile；提交前失败仍执行完整 pending 清理。
- Page/Browser WebSocket 关闭会立即结束身份等待；故障注入覆盖 CDP close 调用挂起、Browser WebSocket 继续存活、其他 Target 被销毁和普通 60 秒命令仍在 pending map 中。
- `LoginProgressV1` 拒绝未知字段和错误 accountId/attemptId；已过去的 deadline 立即触发强制清理，过远的 deadline 被父进程钳制为本地当前时间加两秒，合法关闭状态会启动 Tauri 父进程看门狗。
- collector 不退出或不响应 cancel 时，父进程两秒截止会强制终止 collector/browser、释放运行状态和操作锁；profile 锁延迟释放时生成清理标记并在后台/启动恢复删除。
- 浏览器四种启动组合包含全部统一约束参数。
- `Preferences` 与 `Local State` 在每次启动前幂等合并；升级前已有 profile 也被约束，同时保持无关 JSON 和抖音会话文件不变，并拒绝链接、越界路径与损坏 JSON。
- 已安装测试扩展的后台脚本和内容脚本均不能运行，允许的内建组件仍可正常加载。
- 无活动账号时可渲染并打开设置弹窗和全局错误提示。
- 登录失败后添加按钮恢复，pending 项不进入账号管理 DOM。

### 安装包验收

1. 无账号状态可打开设置。
2. 添加账号后提前关闭浏览器，两秒内恢复按钮，账号管理为空。
3. 成功校验后账号才出现，重启后仍可使用。
4. Edge 不显示 Microsoft 登录或同步提示，Chrome 不显示 Google 登录或同步提示。
5. 应用启动的浏览器不能加载用户扩展、默认应用或后台扩展，不能使用浏览器账号登录或同步。
6. 抖音网页登录、Cookie 持久化、可见同步和隐藏同步正常。
7. 任务结束后无 collector、浏览器进程、调试端口或 pending profile 残留；被锁 profile 有清理标记且重启后删除。
8. 分别验证冷启动 profile、从上一安装包升级的既有 profile 和 Windows 已登录 Microsoft 账号的设备环境。
9. 验收前后比较应用外 Edge/Chrome profile 及 `HKCU/HKLM\Software\Policies\Microsoft\Edge`、`HKCU/HKLM\Software\Policies\Google\Chrome`，确认应用未修改它们。

验收报告必须记录 Windows、Edge、Chrome 和安装包版本，以及 Windows Microsoft 账号登录状态。

## 需求追踪

| ID | 需求 | 自动门禁 | 安装包验收 |
| --- | --- | --- | --- |
| R1 | 关闭登录浏览器后可立即重试 | CDP 关闭、两秒截止、锁释放和延迟清理测试 | 关闭窗口后两秒内按钮恢复 |
| R2 | 无账号时设置可用 | 无活动账号渲染测试 | 首次启动打开设置 |
| R3 | 成功校验后才显示账号 | pending 查询、DOM 隐藏和激活成功边界测试 | 失败管理列表为空，成功后出现 |
| R4 | 抑制 Edge/Chrome 登录与同步提示 | 参数和 profile 合并测试 | Edge/Chrome 冷启动、升级 profile 实测 |
| R5 | 禁用用户扩展、浏览器账号登录与同步 | 测试扩展、参数和偏好测试 | 测试扩展不运行，浏览器账号功能不可用 |
| R6 | 失败删除临时 profile 并全新重试 | 失败矩阵、文件锁和启动恢复测试 | 连续失败/重试不复用 Cookie 或 profile |
| R7 | 不影响系统普通浏览器 | 路径/注册表写入边界测试 | 应用外 profile 与策略键前后不变 |

## 成功标准

- R1–R7 均有通过的自动回归测试和安装包验收记录。
- 以下命令全部退出码为 0：`npm test`、`npm run typecheck`、`npm run app:build`、`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo tauri build`。
- `dist` manifest 校验拒绝未声明文件、运行时目录、浏览器二进制和 source map；安装包扫描不包含 Node、Playwright、NCC、CRI 或浏览器二进制。
- Edge 与 Chrome 的真实安装包验收全部通过；任一浏览器未通过时不得标记该能力完成。
