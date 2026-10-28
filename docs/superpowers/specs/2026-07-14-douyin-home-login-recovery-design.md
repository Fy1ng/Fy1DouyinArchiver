# 抖音首页登录与隐藏恢复设计

## 状态

已由用户逐段确认，并按 `gpt-5.6-sol`、`xhigh` 设计审查结论完成实施。2026-08-11 的 Rust-only Edge/Chrome 系统夹具、自动测试、NSIS 构建与安装目录扫描已通过；真实账号 10 轮隐藏恢复、验证码、身份冲突、桌面录制和无 Node 干净系统仍待人工验收，完成前不得标记为发布完成。验收证据见 [首页登录恢复自动验收记录](../../testing/home-login-recovery-verification.md)。

## 背景

应用为每个抖音账号使用独立的系统 Edge/Chrome profile。首次添加账号当前会直达观看历史页，全新 profile 因而更容易触发首次设备验证。用户可以正常完成人工验证码，且同一 profile 后续同步极少再次触发，因此本设计不识别、处理或绕过验证码，而是将首次登录入口改为抖音首页，并为被其他设备挤下线后的场景增加一次受限的隐藏恢复。

## 目标

- 首次添加账号和手动重新登录均从 `https://www.douyin.com/` 首页开始。
- 首页检测到可靠登录身份即可完成账号登录，不再进入观看历史页复核。
- 登录后若出现“是否保存登录信息”弹窗，自动点击弹窗内的“保存”。
- 观看历史同步确认会话失效后，先隐藏尝试“一键登录”，失败后才提示用户。
- 登录和恢复期间阻止视频媒体，并阻止明确属于首页内容区的视频封面和内容图片，同时保证二维码、验证码和未知图片可加载。
- 多阶段同步保持单一逻辑任务，同时具备独立阶段实例、可靠取消和确定性重启恢复。
- 不削弱账号隔离、浏览器进程身份、身份冲突、Cookie 落盘和错误脱敏保护。

## 非目标

- 不识别验证码页面，不自动处理滑块、短信、二维码或其他风控验证。
- 不填写手机号、密码或其他凭据。
- 不复用用户日常浏览器 profile，不导入或复制普通浏览器 Cookie。
- 不通过昵称、头像或普通 DOM 文案推断账号身份。
- 不允许自动恢复无限循环。
- 不改变历史内容表、观看记录契约或前端公开命令签名。

本设计允许为账号状态和同步编排增加必要的数据库约束或持久化任务状态；这不属于历史内容 schema 变更。

## 架构

采集器保留 `collect` 和 `login`，新增内部模式 `recover-login`：

- `login`：可见系统浏览器打开抖音首页，用于首次添加账号和用户主动重新登录。
- `recover-login`：隐藏系统浏览器打开抖音首页，只执行受限的一键恢复。
- `collect`：只负责观看历史采集；明确未登录时返回 `session_expired`，不在采集器内部执行登录。

Tauri 继续对前端暴露现有 `begin_account_login`、`resume_account_login`、`start_sync` 和 `cancel_sync`。隐藏恢复由 `start_sync` 内部编排，不增加前端可直接调用的恢复命令。

## 首页 URL 与身份来源

首页身份读取必须在最终导航 URL 校验后进行。有效首页 URL 必须满足：

- scheme 为 `https`
- host 精确为 `www.douyin.com`
- 无用户名和密码
- 端口为空或默认 `443`
- path 精确为 `/`
- fragment 为空
- 允许普通查询参数

页面脚本只允许从以下版本化路径读取账号状态：

- `SSR_RENDER_DATA.app.user`
- `__INITIAL_STATE__.user`
- `__INITIAL_STATE__.state.user`
- `_ROUTER_DATA.loaderData.user`
- `__NEXT_DATA__.props.pageProps.user`
- 可解析字符串 `RENDER_DATA.app.user`
- `#root` 的 React props 中已由真实夹具确认的 `children.props.user`

每个候选状态必须在同一对象内同时携带布尔 `isLogin/is_login` 与 `info/userInfo/user_info/currentUser/current_user`。身份对象必须至少包含 `uid` 或 `secUid/sec_uid`。

多来源合并规则：

- 任一来源的非空 `uid` 或 `secUid` 与其他来源冲突时，结果为 `conflicting`。
- 两个来源分别只提供 UID 和 secUid、且没有共享非空 ID 时，不得拼接成一个身份，结果为 `indeterminate`。
- 只有存在共享 ID 或完整字段一致时，才允许将缺失字段补为同一观察身份。
- 昵称和头像只作为附加显示字段，不参与身份匹配。

页面脚本不得遍历整个页面对象图，也不得读取 Cookie、Local Storage、二维码或验证码内容。

## 会话探针

新增独立 `SessionProbeV1`，与历史快照和账号最终身份分离：

```text
authenticated { uid?, secUid?, nickname?, avatarUrl?, source }
unauthenticated { source }
indeterminate { reason }
conflicting
```

只有允许路径中的状态节点明确给出 `isLogin === false`，并且来源结构通过校验时，才返回 `unauthenticated` 并映射为 `session_expired`。字段缺失、页面结构变化、网络失败、脚本异常和来源冲突不得归类为登录失效。

`collect` 在等待历史列表期间并行读取 `SessionProbeV1`。`unauthenticated` 立即结束为 `session_expired`；其余非认证结果继续使用对应技术错误，不再被统一吞并为 `history_page_not_ready`。

## 首次登录与手动重新登录

1. `login` 使用账号独立 profile 启动可见浏览器并导航至抖音首页。
2. `SessionProbeV1` 返回 `authenticated` 后，Rust 将其转换为首页观察身份 `ObservedHomeIdentity`，并严格校验真实 `pageUrl`。
3. 必须至少取得 UID 或 secUid；昵称和头像不能作为身份依据。
4. 首页身份满足条件后进入共享登录后处理，不导航观看历史页。
5. 登录身份出现后最多等待 5 秒查找“是否保存登录信息”弹窗。
6. 5 秒内未出现弹窗时仍可成功；弹窗一旦出现，必须等其消失后才能完成登录。首次可见登录继续受原十分钟和取消边界约束。
7. 完成后按现有重复账号和身份冲突矩阵写入账号，并正常关闭浏览器，确认 profile 为正常退出且 Cookie 已落盘。

`ObservedHomeIdentity` 必须包含真实首页 `pageUrl`，但不允许页面提供 `historyUrl`。Rust 在首页 URL 校验成功后，将规范常量 `https://www.douyin.com/user/self?showTab=record` 写入现有 `AccountIdentityV1.historyUrl`。`AccountIdentityV1` schema 保持 V1，首页 URL 不持久化到该结构。

现有浏览器选择与登录验证 marker 继续保存规范观看历史目标；marker 的成立依据改为“首页身份校验成功且浏览器正常退出”，不宣称身份读取发生在历史页。已有有效 marker 保持可读。

## 自动点击安全边界

“一键登录”和“保存”必须通过独立自包含页面脚本发现，但实际点击由 CDP 输入事件完成，不直接调用可被页面改写的 `element.click()`。

候选按钮必须同时满足：

- 当前最终 URL 仍是有效抖音首页
- 位于真实夹具确认的顶层登录组件或顶层 modal
- 容器可见，按钮可见、启用、非隐藏
- 按钮文本精确匹配
- 当前页面只有一个合格候选
- 按钮中心点的 `document.elementFromPoint` 命中按钮或其后代，证明未被遮挡

点击前重新读取并验证 URL、容器和文本。候选数量不是 1、元素被遮挡或结构不匹配时失败开放，不点击。

- “一键登录”每个恢复阶段最多点击一次。
- “保存”只允许位于包含“是否保存登录信息”的顶层 modal 内，不点击“取消”或页面其他同名按钮。
- 点击后必须等待目标状态变化；多个可见同名按钮、弹窗持续不消失或页面转为其他验证方式时不得继续自动操作。

## 错误码与账号状态

采集器结果、恢复结果和持久账号状态使用不同命名：

- 采集器检测结果：`session_expired`
- 隐藏恢复失败：`login_recovery_failed`
- 账号普通登录失效状态：`login_expired`
- 账号身份隔离故障状态：`identity_conflict`
- 身份冲突错误码：沿用 `account_identity_conflict`

恢复到其他账号不是普通登录失效。出现远端身份与本地身份不一致时：

- 不覆盖、不切换、不补全本地账号 ID
- 终止自动恢复和同步
- 在同一数据库事务中将账号标记为 `identity_conflict`，并将同步任务标记为失败
- 显示账号身份冲突提示，不显示普通“请重新登录”提示
- 后续同步禁止运行，直到用户通过可见重新登录恢复原身份，或归档/删除该账号

恢复身份匹配规则：本地已有的每个非空 ID 都必须与远端对应非空 ID 一致；远端缺少另一字段可以接受，但隐藏恢复不得借机写入或补全新的账号 ID。

## 隐藏恢复流程

1. 首次 `collect` 返回 `session_expired` 后，Tauri 保持同一逻辑同步任务和原同步参数，暂不显示登录失效。
2. 启动一次 `recover-login` 隐藏阶段。
3. `recover-login` 打开抖音首页。若 profile 已自行恢复登录，可直接校验身份；否则只在明确登录组件内点击一次精确文本“一键登录”。
4. 不操作二维码、验证码、手机号、密码或其他登录方式。
5. 浏览器发现、启动、CDP 连接、导航、按钮查找、点击、身份恢复、“保存”处理和正常关闭共享一个从任务开始计算的 30 秒绝对 deadline。
6. 每个内部等待和 CDP 请求只使用剩余时间，不得重新获得完整 30 秒或默认 60 秒超时。
7. deadline 到达后进入现有两阶段终止流程；Tauri 外层看门狗同样以该 deadline 为准，并保留两秒强制结束上限。
8. 恢复身份必须按上一节矩阵与本地账号一致。
9. 恢复成功后执行“保存登录信息”处理，正常关闭浏览器并确认 profile 正常退出，然后按原参数自动重试 `collect` 一次。
10. 重试 `collect` 同时承担跨进程登录持久化验证；再次返回 `session_expired` 说明恢复未持久化，任务停止且不启动第二次恢复。

## 阶段执行实例与文件协议

一次用户同步对应一个 `sync_job`，但每个子进程阶段必须拥有独立的 `phase_attempt_id`，使用 32 位小写无连字符 UUID。

持久阶段枚举：

- `collecting`
- `recovering_login`
- `retrying_after_login`

每个阶段实例的结果、进度、诊断、浏览器 PID 和日志路径必须同时包含：

- `account_id`
- `sync_job_id`
- `phase`
- `phase_attempt_id`

任务级取消文件与阶段文件分离。取消文件只按 `(account_id, sync_job_id)` 定位，由 Tauri 创建、持有和删除；任何 collector 子进程都不得删除或覆盖任务取消文件。

阶段文件规则：

- 每个阶段只读写自己的 result/progress/diagnostic/PID/log。
- 所有 JSON 契约包含 job、phase 和 phase attempt 身份，并拒绝未知字段。
- Tauri 只接受当前持久阶段和实例 ID 完全匹配的输出。
- 旧阶段迟到写入不得覆盖、推进或终结当前阶段。
- 阶段子进程不得清理其他阶段或任务级文件。
- Tauri 在确认上一阶段子进程已退出、浏览器已关闭、输出已消费后，才清理该阶段文件并进入下一阶段。

## 持久编排状态

为避免无必要修改历史内容表，新增原子写入的任务级 `SyncOrchestrationV1` 文件，位于 app data 的任务目录，至少包含：

- schema version
- account ID 与 sync job ID
- 当前 phase 与 phase attempt ID
- phase lifecycle state：`preparing`、`running` 或 `consuming`
- login recovery count
- cancel requested at
- 原同步参数的规范化副本或其 SHA-256
- updated at

该文件与数据库 `sync_jobs.status` 共同构成恢复证据。阶段转换使用比较并交换语义：只有内存当前身份、持久编排身份和数据库 running 任务完全一致时，才允许从实例 A 转换到实例 B。

`RunningSync` 不再永久绑定首个 `Child`。它保存当前阶段和可替换的子进程句柄；阶段转换、取消状态检查、新子进程注册和持久编排更新必须在同一互斥临界区完成，不允许出现取消已请求但下一阶段仍被启动的无锁间隙。

阶段启动顺序固定为：

1. 持有编排互斥锁，检查数据库任务仍为 running 且没有取消证据。
2. 生成新 phase attempt ID，并将编排文件原子转换为该阶段的 `preparing` 状态。
3. 在仍持有同一锁时 spawn 子进程并注册当前 `Child`。
4. spawn 成功后将编排状态更新为 `running`；spawn 失败则直接完成任务失败，禁止遗留 preparing 阶段继续执行。

应用若在 preparing 与 running 之间退出，持久 phase attempt ID 仍可约束启动恢复只检查该实例的 PID 文件；没有匹配进程时只结束任务，不扩大进程清理范围。

## 取消

- 用户取消时，Tauri 先原子写入 `cancel_requested_at` 和任务级取消文件，再处理当前阶段子进程。
- 每次启动新阶段前必须在同一临界区重新检查取消状态；存在取消证据时禁止 spawn。
- 当前 collector 每 200 毫秒读取任务级取消文件，但无权删除它。
- 当前阶段进程和浏览器按现有协作取消、浏览器终止、两秒 collector 强制结束顺序处理。
- 取消发生在上一阶段退出和下一阶段注册之间时，CAS 转换失败，任务直接结束为 `cancelled`。
- 账号状态在取消时保持不变。

## 重启恢复矩阵

应用重启后不继续隐藏恢复或自动重试。恢复顺序固定为：

1. 读取数据库 running 任务和匹配的 `SyncOrchestrationV1`。
2. 只按当前 phase、phase attempt ID、PID、Windows 创建时间、浏览器路径和 profile 精确终止当前实例进程树；损坏、缺失或不匹配的身份文件不得杀进程。
3. 若当前 collecting/retrying 实例已有完整、有效且身份匹配的最终采集结果，优先按现有事务规则导入并标记成功。
4. 否则若已有 `cancel_requested_at`，标记任务 cancelled。
5. 其余运行任务标记为 failed/interrupted，不自动续跑任何阶段。
6. recovering 阶段即使已恢复 profile，也不在重启后自动启动 collect；账号保持原状态，用户下次同步重新判断。
7. 只有在恢复失败结果已由运行时完成数据库事务后，账号才是 `login_expired` 或 `identity_conflict`；孤立临时结果不得单独修改账号状态。
8. 最后清理该任务全部阶段文件、任务取消文件和编排文件。

## 任务最终状态与账号事务

- `login_recovery_failed` 或重试再次 `session_expired`：在一个数据库事务中将账号更新为 `login_expired`，并将 sync job 更新为 failed。
- `account_identity_conflict`：在一个事务中将账号更新为 `identity_conflict`，并将 sync job 更新为 failed。
- 浏览器未安装、启动失败、CDP 失败、Target 关闭、磁盘错误和脚本异常：只令 sync job 失败，账号保持原状态。
- 用户取消：sync job 为 cancelled，账号保持原状态。
- 数据库事务成功后才发送应用内提示和系统通知；通知失败不得回滚状态。

## 资源拦截

资源策略只用于 `login` 和 `recover-login`。CDP `Fetch.requestPaused` 分类器必须是纯函数，输入为当前阶段、资源类型和规范化 URL。

- `Media` 一律 `Fetch.failRequest`。
- `Image` 默认 `Fetch.continueRequest`。
- 只阻止真实页面夹具确认属于首页推荐内容区的视频封面和内容图片规则。
- 二维码、验证码、账号头像、登录 modal 资源、`data:`、`blob:`、无法解析 URL 和未知图片全部继续。
- 规则匹配规范化 origin/path，不依赖会变化的签名查询参数，不使用整个 `douyinpic.com` 等宽泛域名封禁。
- 重定向后的每次暂停请求重新分类。
- 分类器异常必须继续请求。
- 每个暂停请求必须恰好收到一次 `continueRequest` 或 `failRequest`；不能遗漏或重复响应。
- Fetch 在导航前启用，在关闭页面前 disable。
- 每条拒绝规则必须有对应的首页封面拒绝夹具和二维码/验证码放行夹具。

## 页面脚本与诊断

首页会话探针、按钮发现和弹窗发现拆为自包含原始 JavaScript，通过 `include_str!` 嵌入。参数通过序列化 JSON 传入，禁止字符串拼接代码。

诊断只记录：状态来源枚举、候选数量、按钮匹配数量、阶段、耗时、错误码和脱敏 URL 分类结果。不得记录账号 ID 原值、Cookie、二维码、验证码、DOM 文本快照或响应体。

## 进度与前端

前端公开命令签名保持不变。同步状态支持：

- `recovering_login`：正在恢复登录
- `retrying_after_login`：登录已恢复，正在继续同步

隐藏恢复期间取消按钮保持可用。第一次 `session_expired` 不显示瞬时错误；只有最终事务完成后才显示登录失效或身份冲突提示。

账号状态新增 `identity_conflict` 的显示和操作规则：禁止同步，允许可见重新登录、归档和删除。

## 自动测试

- `SessionProbeV1` 覆盖 authenticated、unauthenticated、indeterminate 和 conflicting。
- 首页非 HTTPS、子域名、非默认端口、用户名密码、非根 path、fragment 和错误重定向均拒绝。
- 多来源完整身份、部分身份、无共享 ID 和冲突 ID 矩阵全部覆盖。
- 页面不能提供或伪造 `AccountIdentityV1.historyUrl`；Rust 只写规范常量。
- “一键登录”和“保存”候选必须唯一、可见、启用、未遮挡且位于正确容器；相似文本、隐藏按钮、多个按钮和持续弹窗不点击。
- 隐藏恢复只点击一次“一键登录”，恢复身份与本地账号不一致时进入 `identity_conflict`。
- 30 秒绝对 deadline 能打断卡住的浏览器启动、导航、evaluate 和弹窗等待。
- 三阶段使用独立实例文件；旧阶段迟到写入不能覆盖当前阶段。
- 在每个阶段转换点取消，包括进程退出与新进程注册之间；取消后绝不启动下一阶段。
- 任务取消文件不得被子进程删除。
- collecting、recovering_login、retrying_after_login 各阶段重启时，按 phase instance、PID 创建时间和 profile 精确清理。
- 旧 PID、PID 复用、损坏文件和实例不匹配不得终止进程。
- 恢复成功但浏览器未正常退出时不得启动重试 collect；Cookie 是否跨进程持久化由随后唯一一次 retry collect 验证。
- `login_recovery_failed`、`account_identity_conflict` 的账号状态与 sync job 最终状态在同一事务内更新。
- `Media` 全部阻止；已知首页内容图片阻止；二维码、验证码和未知图片放行。
- 每个 `Fetch.requestPaused` 恰好收到一次响应，分类异常时页面仍可加载。
- 失败和取消不得留下浏览器进程、开放调试端口、不完整结果或阶段文件。

## 实机与安装包验收

Edge 和 Chrome 分别验证：

1. 添加账号打开抖音首页，登录期间不自动进入观看历史页。
2. 登录成功后自动点击目标 modal 中的“保存”，账号正常添加。
3. profile 以 `Normal` 状态退出，登录 Cookie 在重启后仍可使用。
4. 正常同步不触发隐藏恢复。
5. 模拟网页端被其他设备挤下线后，隐藏点击“一键登录”并自动继续原同步。
6. 需要二维码或验证码时不自动操作，30 秒总 deadline 后才提示登录失效。
7. 登录期间视频与已知首页封面不加载，二维码和验证码正常显示。
8. 恢复到其他账号时进入身份冲突状态，不采集或覆盖任何账号数据。
9. 在 collecting、recovering_login、retrying_after_login 三个转换窗口分别取消和强制退出应用，重启后无续跑、误杀或残留。
10. 隐藏恢复没有可见顶层窗口，任务结束后没有浏览器残留进程或开放调试端口。
11. NSIS 安装包在无 Node 环境中保持登录、恢复、同步和取消行为一致。

## 成功标准

- 首次登录不再直达观看历史页。
- 自动保存登录信息稳定命中目标 modal 且不误点其他按钮。
- 会话失效时先执行一次不可见、受限、30 秒绝对 deadline 内结束的恢复。
- 恢复成功自动继续同一逻辑同步，失败后才事务化写入登录失效状态。
- 阶段文件、取消和重启恢复具备实例级身份，不存在跨阶段覆盖或取消后续跑。
- 身份冲突与普通登录失效分离，绝不采集到其他账号名下。
- 验证码和二维码始终由用户处理，应用不尝试绕过抖音风控。
- 资源拦截不会造成登录二维码或验证码缺失，也不会令暂停请求悬挂。
