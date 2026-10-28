# Emily UI 样式审计优化实施交接

**状态：** 待主代理实施
**审计日期：** 2026-08-24
**审计基线：** `codex/stage-three-roadmap` / `6b63ceb56ccc8fc74b60b8d4fdec6b3665a293bd`
**关联设计：** `docs/superpowers/specs/2026-08-19-ui-visual-refresh-design.md`
**证据目录：** `<local-ui-audit-evidence>`

## 1. 交接结论

本轮已通过 Browser 真实渲染检查 Emily Web 演示数据，覆盖 12 个主要工作区页面、主要弹窗与下拉菜单，并完成三种主题与三种支持窗口尺寸的交叉检查；年度回顾另做了定向渲染检查：

- 主题：浅色、深色、跟随系统。
- 窗口：`960×640`、`1080×760`、`1440×900`。
- 完整交叉页面：概览、资料库、搜索、观看历史、喜欢、收藏、抖音收藏夹、本地收藏夹、账号管理、任务中心、通知中心、设置。
- 定向附加页面：年度回顾（深色、`1080×760`）。
- 设置：9 个分类、16 个子面板，以及设置搜索。
- 交互：账号菜单、排序和筛选下拉、下载弹窗、加入/新建收藏夹、封面缓存确认和管理、年度回顾、禁用控件、空结果及长文本。

完整交叉共检查 `12 × 3 × 3 = 108` 个页面/主题/尺寸组合，另检查年度回顾定向场景。未发现页面级横向溢出或视口外裁切。当前主要问题不是整体布局崩坏，而是可访问性、状态一致性、最小窗口滚动归属、视觉语义和演示状态保真度。状态覆盖并非全部都能由现有 Web fixture 真实触发，第 3.2 节明确区分了 Browser 渲染与仅源码核对，不能把源码核对写成已通过渲染验收。

问题统计：

- P0：0 项。
- P1：3 项。
- P2：6 项。
- P3：3 项。

主代理应优先完成本文第 4 节的前五项，不要把本轮扩大为信息架构重写、采集器改造或 Tauri/Rust 业务重构。

## 2. 实施边界

### 2.1 本轮可以修改

- CSS 设计令牌、字号、点击目标、响应式规则和滚动策略。
- Svelte 通用控件与页面组件。
- Web 演示 fixture，以及与 UI 状态直接相关的前端测试。
- 必要的 Tauri 窗口 UI 自动化或手工验收说明，但不改变采集和登录业务语义。

### 2.2 本轮不做

- 不自动执行真实抖音登录，不处理或绕过 CAPTCHA。
- 不改变账号提交、隔离浏览器 profile、登录恢复或采集器安全边界。
- 不重写 Rust 下载、同步、数据库或任务模型来修饰演示 UI。
- 不改变资料库来源、账号范围和时间语义。
- 不美化安装器，不自动推送、创建 PR 或发布。

### 2.3 当前工作区风险

审计结束时 UI 源码已进入基线提交 `6b63ceb`，但工作区仍有大量与本任务无关的 Rust 修改。主代理开始实施前必须重新执行并记录现有 dirty 路径及状态快照：

```powershell
git branch --show-current
git rev-parse HEAD
git status --short --branch
```

不得回滚、暂存或提交既有 Rust 改动。每个阶段只能用路径限定的 `git diff -- <UI paths>` 检查本阶段改动，只暂存该阶段直接相关的前端文件和测试；提交前必须运行 `git diff --cached --check`、`git diff --cached --stat` 和 `git diff --cached -- <staged UI paths>`。每阶段创建一个内容单一的提交，结束时确认开始时记录的无关 dirty 路径及状态保持不变。不要把“全局 `git diff` 为空或只含 UI 文件”作为完成条件，因为当前工作区在任务开始前就不是干净状态。

## 3. 审计证据与已知限制

### 3.1 代表性证据

- 最小窗口概览：`83-current-overview-dark-960x640.png`
- 最小窗口搜索与三重滚动：`84-current-search-dark-960x640.png`
- 最小窗口账号页：`85-current-accounts-dark-960x640.png`
- 下载弹窗：`91-current-download-selected-dark-960x640.png`
- 账号菜单：`93-current-account-menu-dark-1080x760.png`
- 浅色设置：`79-current-settings-light-1080x760.png`
- 大窗口概览：`87-current-overview-dark-1440x900.png`
- 封面缓存确认：`115-current-cover-cache-confirm-dark-1080x760.png`
- 取消封面缓存确认后：`116-current-cover-cache-after-close-dark-1080x760.png`
- 封面缓存管理：`117-current-cover-cache-manager-dark-1080x760.png`
- 加入收藏夹：`118-current-folder-picker-dialog-dark-1080x760.png`
- 新建收藏夹长文本：`120-current-folder-create-long-text-dark-1080x760.png`
- 搜索空结果与长查询：`123-current-search-empty-long-dark-1080x760.png`
- 年度回顾：`128-current-annual-review-dark-1080x760.png`

目录中 `65-current-*` 之前的文件仅保留为历史对照，实施验收应优先使用上述当前源码证据。

### 3.2 页面 × 状态覆盖矩阵

图例：`R` 表示本轮已由 Browser 真实渲染；`S` 表示现有 Web fixture 无法稳定触发，仅完成源码检查；`—` 表示该页没有独立的该类状态或本轮未触发。`R` 只表示已观察，不代表没有问题，问题结论以第 5 节为准。

| 页面 | 正常 | 空 | 加载 | 错误 | 禁用 | 长文本/溢出 | 弹窗/下拉 |
|---|---|---|---|---|---|---|---|
| 概览 | R | S | S | S | — | R | R（全局账号菜单） |
| 资料库 | R | S | S | S | R | R | R（排序、下载、收藏夹） |
| 搜索 | R | R | S | S | — | R | R（筛选） |
| 观看历史 | R | S | S | S | R | R | R（排序、下载、收藏夹） |
| 喜欢 | R | S | S | S | R | R | R（下载、收藏夹） |
| 收藏 | R | S | S | S | R | R | R（下载、收藏夹） |
| 抖音收藏夹 | R | S | S | S | — | R | — |
| 本地收藏夹 | R | S | S | S | — | S | R（范围菜单） |
| 账号管理 | R | S（无账号） | S（登录序列） | R | S | S（长账号名） | R（账号菜单） |
| 任务中心 | S（有数据） | R | S | S | S | R（视口） | — |
| 通知中心 | S（有数据） | R | S | S | — | S | — |
| 设置 | R | R（搜索无匹配） | S | S | R | R | R（缓存、备份等） |
| 年度回顾 | R（定向） | S（稀疏数据） | S | S | — | S | — |

关键状态证据包括：空任务/通知 `75-current-task-center-dark-1080x760.png`、`76-current-notifications-dark-1080x760.png`；账号错误 `122-current-account-error-dark-1080x760.png`；搜索空结果和长查询 `123-current-search-empty-long-dark-1080x760.png`；长收藏夹名称 `120-current-folder-create-long-text-dark-1080x760.png`；下载、账号和收藏夹弹层 `91-current-download-selected-dark-960x640.png`、`93-current-account-menu-dark-1080x760.png`、`118-current-folder-picker-dialog-dark-1080x760.png`。

### 3.3 未由 Web 演示完整证明的状态

以下状态因为 Web API 同步完成或入口依赖真实 Tauri 环境，只完成源码检查，不能宣称已经通过真实渲染验收：

- 可持续观察的加载态和保存中状态。
- 后端错误、下载提交错误、设置保存失败及回滚。
- 启动恢复的 recovering/blocked/failed 状态。
- 无账号、首次登录和登录浏览器状态序列。
- Windows 原生文件选择器、托盘恢复、跨显示器 DPI 和 WebView2 强制颜色。
- 真实 Tauri 下载预览的候选数量与去重结果。

实现时应先补可控 Web fixture，再做真实 Tauri 人工复核；不能用真实抖音登录代替确定性 UI 测试。

## 4. 最值得优先实施的五项

| 顺序 | 工作项 | 验收重点 | 建议提交边界 |
|---|---|---|---|
| 1 | 修正主操作按钮对比度和语义色令牌（UI-01/UI-07） | 所有 enabled 主按钮 normal/hover/focus 文字对比度 `>= 4.5:1` | CSS token + 数学/真实样式两层对比度测试 |
| 2 | 建立统一 Dialog/ConfirmDialog 焦点管理 | 初始焦点、Tab 约束、Esc、背景 inert、焦点恢复 | 通用控件 + 迁移现有弹窗 + 测试 |
| 3 | 修复封面缓存取消后仍显示启用 | 关闭所有确认路径后仍为未选中且未保存 | App 状态 + 回归测试 |
| 4 | 收敛 `960×640` 的滚动所有权 | 页面只保留一个主滚动区，不再出现 7px 假 scrollbar 按钮 | 滚动组件/CSS + 页面布局测试 |
| 5 | 统一字号、点击目标和颜色语义（UI-05/UI-07） | 关键文字不低于 12px，高频目标不低于 32px，主动作/选择/焦点/危险/滚动条不混用 | CSS token + 视觉回归 |

建议按上表逐项完成最小闭环：实现、运行直接相关测试、修复，再进入下一项。不要一次性重写 `App.svelte`。

## 5. 详细问题与实施建议

### UI-01 P1：主操作按钮文字对比度不足

**渲染证据**

- 深色：前景 `rgb(29, 29, 29)`、背景 `#d21b46`，对比度 `3.20:1`。
- 浅色和当前系统浅色：白色前景、背景 `#0f9f91`，对比度 `3.29:1`。
- 证据：`78-current-settings-dark-1080x760.png`、`79-current-settings-light-1080x760.png`。

**源码位置**

- `app/src/styles.css:427`：`.primary` 继承 `color: var(--surface)`。
- `app/src/styles.css:2205`：深色 `--brand-action`。
- `app/src/styles.css:4923`：后置 `.primary` 背景覆盖没有同步覆盖文字颜色。

**建议实现**

将动作按钮从通用表面色中解耦：

```css
:root {
  --action-primary-bg: #08786e;
  --action-primary-bg-hover: #06645d;
  --action-primary-fg: #ffffff;
}

:root[data-theme="dark"] {
  --action-primary-bg: #2dd4bf;
  --action-primary-bg-hover: #5eead4;
  --action-primary-fg: #0b0d10;
}

.primary {
  color: var(--action-primary-fg);
  border-color: var(--action-primary-bg);
  background: var(--action-primary-bg);
}
```

`data-theme="system"` 必须在暗色媒体查询中复用深色令牌，在浅色系统中复用浅色令牌。候选值实测：

- 深色正常：`#2dd4bf` 与 `#0b0d10` 为 `10.45:1`。
- 深色 hover：`#5eead4` 与 `#0b0d10` 为 `13.15:1`。
- 浅色正常：`#08786e` 与白色为 `5.35:1`。
- 浅色 hover：`#06645d` 与白色为 `7.03:1`。

深色主动作继续使用 Emily 青色。玫红只保留给少量品牌或选择强调，删除、清空等危险动作使用独立红色，不能再把玫红同时当作主动作和危险提示。

**验收标准**

- 所有 enabled 主按钮在 normal、hover 和 focus 状态下，普通文字与背景均满足 WCAG AA `4.5:1`。
- disabled 控件不宣称属于 WCAG 1.4.3 的强制适用范围；项目仍单独要求其文字与背景至少达到 `3:1`，并同时使用透明度、图标或文案表达不可用状态。
- 浅色主题仍只使用 Emily 青色，不引入玫红主操作。
- 自动化分两层：先对四组 token 做数学对比度计算，再在浅色、深色、跟随系统浅色和跟随系统深色中读取真实 `.primary` computed style；跟随系统还必须覆盖运行时由浅切深和由深切浅。

**收益 / 成本 / 风险：** 全局可访问性收益高；成本 S；风险低。

### UI-02 P1：弹窗没有统一焦点管理和背景隔离

**渲染证据**

- 下载弹窗打开后 `document.activeElement` 为 `BODY`。
- 封面缓存确认打开后焦点仍在背景 checkbox。
- 收藏夹弹窗打开后焦点仍在背景触发按钮。
- 背景仍有 30 个以上可聚焦元素，未设置 `inert`。
- 证据：`91-current-download-selected-dark-960x640.png`、`115-current-cover-cache-confirm-dark-1080x760.png`、`118-current-folder-picker-dialog-dark-1080x760.png`。

**源码位置**

- `app/src/download/DownloadStartDialog.svelte:50`
- `app/src/CoverCacheManager.svelte:163`
- `app/src/App.svelte:3604-3855` 的多个确认和收藏夹弹窗
- `app/src/App.svelte:2817-4201`：当前弹窗和页面内容都位于 `.workspace-main` 内。

这些弹窗普遍只有 `role="dialog"`、`aria-modal="true"` 和 `tabindex="-1"`，没有初始焦点、焦点约束、Esc 或恢复逻辑。

**建议实现**

新增统一控件：

- `app/src/controls/Dialog.svelte`
- `app/src/controls/ConfirmDialog.svelte`
- 可选 `app/src/controls/dialog-focus.ts`，只承载可测试的焦点算法。

先修正宿主结构：不能直接给当前 `.workspace-main` 设置 `inert`，因为活动弹窗也是它的后代，这会连同弹窗本身一起禁用。应在 `AppShell` 内增加与背景内容并列的稳定 `ModalHost`，或把所有 Dialog portal 到该宿主；一级侧栏和页面内容组成一个背景 root，`ModalHost` 作为其兄弟节点。打开弹窗时只给背景 root 和其他背景兄弟设置 `inert`，绝不能给包含活动弹窗的祖先设置 `inert`。

统一行为：

1. 打开前保存触发元素。
2. `tick()` 后聚焦调用方指定的安全控件；未指定时聚焦关闭/取消按钮或对话框容器。
3. 打开期间只让弹窗可聚焦，对 `ModalHost` 的背景兄弟设置 `inert`；必要时同步 `aria-hidden`。
4. Tab/Shift+Tab 在弹窗内循环。
5. 非忙碌状态下支持 Esc 关闭。
6. 关闭后恢复到原触发元素；若触发元素已移除，退回到相邻稳定控件。
7. 所有图标关闭按钮必须有 `aria-label="关闭…"`，不能只依赖 `title`。
8. 原生目录选择器打开和返回后仍保持正确弹窗焦点。

`ModalHost` 还必须维护弹窗栈或引用计数：只有栈顶弹层处理 Tab/Esc；打开嵌套确认时，下层弹窗不可交互但不能被销毁；关闭栈顶后恢复到下层稳定控件；最后一个弹窗关闭后才移除背景 `inert` 和滚动锁。Popover 若能与 Dialog 同时存在，也必须定义其入栈或关闭策略。

不要在同一 Svelte 组件中随意混用旧式 `on:click` 和属性式 `onclick`；修改时遵循该组件当前使用的事件语法，必要时整组件一次性迁移并运行 typecheck。

**优先迁移顺序**

1. 下载弹窗。
2. 封面缓存启用和管理。
3. 加入/新建收藏夹。
4. 导入/导出备份、安装更新、删除旧数据根。
5. 第 UI-09 项的删除确认。

**验收标准**

- 每个弹窗打开后焦点位于弹窗内。
- 连续 Tab 不会进入背景；Esc 和关闭按钮行为一致。
- 关闭后焦点回到触发器。
- 背景控件不可被键盘或辅助技术操作。
- 背景设为 `inert` 后，活动弹窗的按钮、输入框和文件选择入口仍可操作。
- 新增组件级测试覆盖上述行为，并覆盖双层弹窗栈、关闭栈顶后的焦点恢复和最后一层关闭后的 `inert` 清理；不能只检查 `aria-modal` 存在。

**收益 / 成本 / 风险：** 键盘和读屏器主流程收益极高；成本 M；风险中，重点防止嵌套弹层和文件选择器回归。

### UI-03 P1：取消封面缓存确认后复选框仍显示选中

**渲染证据**

- 打开前 checkbox 为 `false`。
- 点击后确认框打开，原生 checkbox 已变为 `true`。
- 点击关闭后确认框消失，checkbox 仍为 `true`，但用户并未选择缓存范围。
- 证据：`115-current-cover-cache-confirm-dark-1080x760.png`、`116-current-cover-cache-after-close-dark-1080x760.png`。

**源码位置**

- `app/src/App.svelte:1696`：`requestCoverCacheEnabled()` 只打开确认框。
- `app/src/App.svelte:3268`：原生 checkbox 的 change 已先改变 DOM 状态。
- `app/src/App.svelte:3789`：关闭路径只修改 `coverCacheEnableOpen`。

**建议实现**

首选将该控件改为完全受控行为：

- 用户请求从关闭变为开启时，阻止原生 checkbox 先行提交视觉状态。
- 仅在选择“仅后续新增”或“补齐全部历史”后，把 `coverCacheSettings.enabled` 更新为 `true` 并保存。
- 关闭、点击背景、Esc 和取消都保持 `false`。
- 关闭已启用缓存时仍可直接切换为 `false`，但保存失败需要回滚已确认值。

**验收标准**

- 测试“点击未选中 → 打开确认 → 关闭”后 checkbox 仍未选中，且未调用保存启用。
- 两种确认选项分别产生正确 `backfillMode` 并只保存一次。
- 保存失败时恢复后端已确认值并显示可操作错误。

**收益 / 成本 / 风险：** 消除持久状态与界面视觉不一致；成本 S；风险低。

### UI-04 P2：最小窗口存在多重高饱和滚动条

**渲染证据**

`960×640` 搜索页同时存在：

- 侧栏导航滚动容器。
- `.workspace-main` 滚动容器。
- `.search-results` 内部滚动容器。

三条固定粉色 scrollbar 的宽度均为 7px，最短 thumb 为 `7×28`。证据：`84-current-search-dark-960x640.png`。

**源码位置**

- `app/src/styles.css:4577-4604`：多个滚动 viewport。
- `app/src/styles.css:4606-4614`：隐藏原生滚动条。
- `app/src/styles.css:4633-4662`：7px 固定自绘滚动条。
- `app/src/controls/CustomScrollbars.svelte`：扫描所有 overflow 容器并向 body 注入 button。

**建议实现**

优先采用“一个页面一个主垂直滚动所有者”：

- `.workspace-main` 作为内容页主滚动容器。
- 搜索、资料库和列表结果自然参与主滚动，不再在普通页面高度下建立第二个垂直滚动区。
- 只有任务详情、超长弹窗列表等需要固定头尾的组件保留局部滚动，并给出清楚边界。
- 侧栏可独立滚动，但应降低视觉强调。

推荐移除全局扫描式 `CustomScrollbars`，恢复 WebView2/Windows 原生滚动行为，并通过 `scrollbar-color`/WebKit 样式做克制定制。若产品决定保留自绘 scrollbar，则必须：

- 视觉宽度至少 12px，实际命中区至少 24px。
- 使用 `role="scrollbar"`、`aria-controls`、`aria-valuemin/max/now`。
- 支持方向键、PageUp/PageDown、Home/End。
- 不为每个滚动容器向 body 注入同名 button。

**验收标准**

- `960×640` 搜索页最多保留主内容和侧栏两条垂直滚动路径；结果列表不再形成第三条。
- 鼠标滚轮和触控板在结果区域不会因嵌套容器突然截获。
- 键盘滚动与辅助技术语义完整。
- `960×640`、`1080×760`、`1440×900` 均无页面级横向滚动。

**收益 / 成本 / 风险：** 最小窗口使用体验和 Windows 桌面适配收益高；成本 M；风险中，需回归长列表位置、滚动锚点和分页。

### UI-05 P2：10–11px 文字和小点击目标过多

**渲染证据**

- 概览 `1080×760` 一次检测到至少 20 个小于 12px 的可见文字。
- `960×640` 侧栏按钮压缩为 30px 高。
- sidebar collapse 为 `30×30`，checkbox 为 `16×16`。
- 证据：`66-current-overview-dark-1080x760.png`、`83-current-overview-dark-960x640.png`。

**源码位置**

- `app/src/styles.css:2441`：侧栏分组 10px。
- `app/src/styles.css:2538`：页面说明 11px。
- `app/src/styles.css:2651`：概览辅助信息 11px。
- `app/src/styles.css:2469`：低高度侧栏按钮 30px。
- `app/src/styles.css:4855`：checkbox/radio 16px。

**建议实现**

建立离散尺寸令牌并逐步替换散落字号：

```css
:root {
  --text-xs: 12px;
  --text-sm: 13px;
  --text-base: 14px;
  --control-hit-compact: 32px;
  --control-hit-default: 36px;
}
```

- 仅对非关键信息保留 12px，正文和表单说明优先 13px。
- 高频导航和按钮至少 32px；默认保持 36px。
- checkbox/radio 可视尺寸建议 18px，标签整体点击区域至少 32px。
- 长文本通过可用宽度、换行或 ellipsis 解决，不再依赖缩小字号。

**验收标准**

- 业务关键信息不低于 12px。
- 主要鼠标目标不小于 `32×32`；紧凑表格中例外需有足够的整行点击区。
- 在 Windows 100% 和 125% 缩放下不重叠、不裁切。

**收益 / 成本 / 风险：** 可读性与误触改善明显；成本 M；风险中，需回归最小窗口密度。

### UI-06 P2：账号菜单缺少菜单语义和键盘模型

**渲染证据**

账号菜单打开后：

- trigger 只有 `aria-expanded`，没有 `aria-haspopup` 或 `aria-controls`。
- `.account-menu` 没有 menu/listbox 角色。
- 菜单项没有 roving tabindex，焦点仍留在 trigger。
- 证据：`93-current-account-menu-dark-1080x760.png`。

**源码位置**

- `app/src/shell/AccountSwitcher.svelte:38-80`
- 可参考已有 `app/src/controls/SelectField.svelte:44-118` 的基础键盘处理，但不要直接混淆“选择值”和“执行命令”两种语义。

**建议实现**

- 采用单一 `menu` 模型，不在同一 trigger 下混用 listbox 和 menu。
- trigger 增加 `aria-haspopup="menu"`、稳定 `aria-controls` 和正确的 `aria-expanded`。
- 弹层容器使用 `role="menu"`；账号选项使用 `role="menuitemradio"` 和 `aria-checked`；同步、添加、管理使用 `role="menuitem"`；账号区与操作区之间使用 `role="separator"`。
- 使用 roving tabindex。打开时聚焦当前账号项；上下方向键、Home/End 移动；Enter/Space 执行；Esc 关闭并恢复 trigger。
- 点击外部关闭，但不吞掉下一目标的正常焦点。

**验收标准**

- 仅用键盘可切换账号并执行同步/添加/管理。
- 读屏器能区分当前账号、未选账号、登录状态、分隔区和命令项。
- DOM 中不存在同一弹层混合 `listbox/option` 与 `menu/menuitem` 的含糊模型。

**收益 / 成本 / 风险：** 账号主入口可访问性提升；成本 S-M；风险低。

### UI-07 P2：深色视觉语义过度集中在粉红/红色

**渲染证据**

深色主题中主按钮、选中导航、checkbox、滚动条和危险按钮同时使用相近的玫红/红色，导致“当前选择”“主要动作”“危险动作”之间区分不足。证据：`84-current-search-dark-960x640.png`、`85-current-accounts-dark-960x640.png`。

**源码位置**

- `app/src/styles.css:2200-2210`
- `app/src/styles.css:4620-4660`
- `app/src/styles.css:4923-4944`

**建议实现**

将颜色按语义拆分：

- `action-primary`：页面主要提交动作。
- `selection`：当前导航、选中列表、checkbox。
- `focus`：键盘焦点。
- `danger`：删除、清空和不可逆操作。
- `scrollbar`：中性灰，不承担品牌或危险语义。

遵守已确认设计：深色以青色表达正常、成功和归档可用；玫红只用于少量品牌强调；危险操作保持独立红色；浅色不出现玫红。

**验收标准**

- 不看文字也能区分选择状态和危险操作。
- 状态颜色同时配合文字或图标，不单独承担含义。
- focus ring 与背景对比度达到至少 `3:1`。

**收益 / 成本 / 风险：** 改善全局层级和风险识别；成本 M；风险中，视觉变化范围较广。

### UI-08 P2：Web 下载预览 fixture 固定返回 0

**渲染证据**

背景显示“已选择 1 条”，下载弹窗却显示“共 0 个作品”。证据：`91-current-download-selected-dark-960x640.png`。

**源码位置**

- `app/src/download/api.ts:24-29`：非 Tauri 环境固定返回 `{ candidateCount: 0 }`。
- `app/src/download/DownloadStartDialog.svelte:53`：直接展示 preview 数量。

**判断边界**

这是 Web 演示和测试保真度问题，不代表真实 Tauri 后端已确认存在计数错误。

**建议实现**

为 Web preview 提供确定性 fixture：

- `checked`：根据选择 ID 返回候选数量。
- `filtered/all`：返回与演示记录一致的候选和内容类型。
- 支持测试配置 `success / empty-after-dedup / loading / error`。
- Web 环境创建任务仍应保持明确的模拟或禁用边界，不能伪装成真实下载成功。

**验收标准**

- 选择一条可下载演示记录时，弹窗候选数与选择一致。
- 可单独渲染去重后 0 条、加载和错误状态。
- 真实 Tauri 最终仍以 Rust preview 返回为准。

**收益 / 成本 / 风险：** 提高后续 UI 审计和自动化可信度；成本 M；风险低。

### UI-09 P2：危险确认混用 `window.confirm` 和自定义弹窗

**渲染/源码证据**

Browser 实测“删除全部”触发原生 confirm，取消后演示数据仍为 2 条。以下操作使用 `window.confirm`：

- `app/src/App.svelte:965`：切换浏览器。
- `app/src/App.svelte:2038-2060`：删除观看记录。
- `app/src/App.svelte:2470-2471`：账号归档/删除。
- `app/src/CoverCacheManager.svelte:107,122`：缓存规则和清理。
- `app/src/FolderPage.svelte:256`：删除收藏夹。

**建议实现**

迁移到 UI-02 的统一 `ConfirmDialog`：

- 明确动作对象、影响范围和是否会影响抖音远端。
- API 明确区分 `destructive`、`warning`、`neutral` 三种语义，而不是所有确认都使用 danger 按钮。
- `destructive` 用于真正删除或清空，例如删除记录、删除账号数据、删除收藏夹和清空缓存；确认按钮使用独立 danger 语义，安全取消为默认焦点。
- `warning` 用于切换浏览器等有影响但并非删除的操作；使用警告图标和琥珀语义，默认焦点仍为取消。
- `neutral` 用于可恢复归档等普通确认；使用普通主动作语义，并在文案中说明恢复入口。
- 异步执行期间禁止重复提交，关闭行为与 busy 状态一致。
- 支持测试确认、取消、执行失败和焦点恢复。

**验收标准**

- 产品内不再直接调用 `window.confirm`。
- 三种确认语义在三主题中视觉一致但可明确区分，键盘行为完整。
- 自动化逐一断言当前 `window.confirm` 调用点被映射到正确语义，防止切换浏览器或归档误用 danger。

**收益 / 成本 / 风险：** Windows/Tauri 交互一致性提高；成本 M；风险中，必须逐条回归破坏性流程。

### UI-10 P3：设置搜索无匹配时没有反馈

**渲染证据**

输入不存在的长关键词后仍显示上一个设置面板，用户无法判断搜索没有命中。证据：`114-current-settings-search-empty-long-dark-1080x760.png`。

**源码位置**

- `app/src/settings-workspace/SettingsWorkspace.svelte:109-130`
- `if (!match) return` 直接静默结束。

**建议实现**

- 增加 `noMatch` 状态和带 `aria-live="polite"` 的“未找到设置”反馈。
- 提供“清空搜索”按钮。
- 无结果时保留导航结构，但弱化当前面板，避免看起来像命中结果。
- 有结果时继续定位具体子面板并短暂高亮。

**验收标准**

- 空查询、命中、无命中、快速连续输入均有确定行为。
- 搜索反馈不会错误保存设置或改变已选值。

**收益 / 成本 / 风险：** 降低设置查找误解；成本 S；风险低。

### UI-11 P3：任务和通知页标题重复，空状态缺少行动方向

**渲染证据**

- 页面通用 topbar 已显示 H1“任务中心/通知中心”。
- 子页面再次显示同名 H2。
- 空状态占据大面积但缺少直接下一步。
- 证据：`75-current-task-center-dark-1080x760.png`、`76-current-notifications-dark-1080x760.png`。

**源码位置**

- `app/src/App.svelte:2819-2821`
- `app/src/TaskCenterPage.svelte:252-273`
- `app/src/NotificationsPage.svelte:133-150`

**建议实现**

- 子标题改为“全部任务”“待处理通知”，或保留语义 region 名但不重复可见标题。
- 任务为空时提供“同步当前账号”或前往账号管理的单一相关动作。
- 通知为空时保留健康说明，不额外堆叠无关按钮。
- 空状态高度随内容收敛，不强制撑满画布。

**验收标准**

- 页面只有一个主 H1，后续标题层级有明确语义。
- 空状态最多一个直接相关的主要动作。

**收益 / 成本 / 风险：** 信息层级更清楚；成本 S；风险低。

### UI-12 P3：大窗口内容宽度缺少按页面类型约束

**渲染证据**

`1440×900` 概览内容宽 1176px，主分析列约 763px；演示数据较少时形成大面积空白。证据：`87-current-overview-dark-1440x900.png`。

**源码位置**

- `app/src/styles.css:2519-2522`：`.workspace-main` 没有阅读宽度约束。
- 设置页已有 `width: min(1120px, 100%)`，可作为按页面类型约束的参考。

**建议实现**

- 增加 `PageContainer` 或页面修饰类。
- 概览、账号和设置等阅读/配置页面居中限制在约 `1040-1120px`。
- 资料库、搜索结果和高密度任务表继续允许全宽。
- 空图表降低固定最小高度；有大量数据时再自然扩展。

**验收标准**

- `1440×900` 下视线不被过度拉散。
- 高密度列表不因通用 max-width 丢失有效空间。

**收益 / 成本 / 风险：** 大屏扫描效率提高；成本 S-M；风险低。

## 6. CSS 与组件治理目标

### 6.1 设计令牌

建议最终收敛到以下语义组，页面不得继续直接复用颜色值表达多个含义：

- 背景和表面：`background`、`surface-1`、`surface-2`。
- 文字：`text-primary`、`text-secondary`、`text-muted`。
- 动作：`action-primary-bg/fg/hover`。
- 选择：`selection-bg/fg/soft`。
- 焦点：`focus-ring/soft`。
- 状态：`success`、`warning`、`danger`、`info`。
- 滚动条：`scrollbar-thumb/hover/track`。
- 字体：`text-xs/sm/base/lg`。
- 控件：`control-hit-compact/default`、`radius-control/panel`。

主动作令牌的浅/深色值以 UI-01 为固定起点。UI-07 的治理不是可选美化：主动作使用青色，选择/品牌强调与 danger 分离，焦点独立，scrollbar 保持中性；页面不得回退为直接复用 `brand-action` 表达多个含义。应先定义 token，再机械替换；不要在同一提交中同时改变信息架构和所有视觉值。

### 6.2 通用组件

| 组件 | 职责 | 禁止承担 |
|---|---|---|
| `Dialog` | 遮罩、标题关联、焦点、Esc、恢复、滚动锁定 | 业务保存和删除逻辑 |
| `ConfirmDialog` | 对象、说明、确认/取消、busy、`destructive/warning/neutral` 语义 | 直接执行任意全局副作用 |
| `Popover/Menu` | trigger 关系、单一 ARIA 模型、方向键、外部关闭、焦点恢复 | 在同一弹层混用 listbox 与 menu 角色 |
| `EmptyState` | 图标、标题、说明、最多一个主动作 | 大面积装饰和多个并列 CTA |
| `PageContainer` | 阅读型/全宽型页面宽度策略 | 单个页面的业务布局 |
| `ScrollViewport` | 明确局部滚动所有权 | 全局扫描所有 overflow DOM |

### 6.3 静态治理建议

- 在 `ui-contract.test.ts` 或独立测试中禁止新增直接 `window.confirm`。
- 扫描无 `aria-label`/可见文本的图标按钮。
- 扫描任意新增 10px/11px 字号，要求显式豁免理由。
- 对核心 token 执行对比度计算。
- 保持 `prefers-reduced-motion` 行为。

## 7. 建议实施与提交顺序

### 提交 1：动作色、视觉语义与对比度（UI-01/UI-07）

可能文件：

- `app/src/styles.css`
- `app/src/ui-contract.test.ts` 或新增可访问性 token 测试

验证：

```powershell
npm test -- --run app/src/ui-contract.test.ts
npm run typecheck
npm run app:build
```

### 提交 2：Dialog 基元与封面缓存状态

可能文件：

- `app/src/shell/AppShell.svelte`
- `app/src/controls/ModalHost.svelte`，或等价的稳定 portal 宿主
- `app/src/controls/Dialog.svelte`
- `app/src/controls/ConfirmDialog.svelte`
- `app/src/download/DownloadStartDialog.svelte`
- `app/src/CoverCacheManager.svelte`
- `app/src/App.svelte`
- 对应测试

先迁移下载和封面缓存，完成焦点和 checkbox 回归后再迁移其余确认框。

### 提交 3：滚动、字号和点击目标

可能文件：

- `app/src/controls/CustomScrollbars.svelte`，或删除其接入
- `app/src/styles.css`
- 搜索/资料库页面的少量布局类

必须重新跑三种窗口，重点看 `960×640` 搜索、概览、账号和设置。

### 提交 4：账号菜单、统一确认和信息层级

可能文件：

- `app/src/shell/AccountSwitcher.svelte`
- `app/src/App.svelte`
- `app/src/FolderPage.svelte`
- `app/src/CoverCacheManager.svelte`
- `app/src/TaskCenterPage.svelte`
- `app/src/NotificationsPage.svelte`
- `app/src/settings-workspace/SettingsWorkspace.svelte`

### 提交 5：Web 状态 fixture 与回归矩阵

可能文件：

- `app/src/download/api.ts`
- `app/src/app-api.ts` 或独立 demo fixture
- UI 测试和视觉验收脚本

新增可控 `normal / empty / loading / error / disabled / long` 状态，不调用真实登录。

实际提交可进一步拆小，但不要把五个阶段压成一个大提交。

## 8. 验证矩阵

### 8.1 自动化最低要求

每个阶段先运行直接相关测试，终审再运行：

```powershell
npm test
npm run typecheck
npm run app:build
```

只有修改 Rust/Tauri 命令或契约时才追加相应 Rust 测试；纯 UI 修复不应顺手触碰现有 Rust 脏文件。

新增测试至少覆盖：

- 主按钮两层验证：四组 normal/hover token 的数学对比度；浅色、深色、跟随系统浅色、跟随系统深色下真实 `.primary` normal/hover/focus computed style。跟随系统需模拟运行时切换。
- Dialog 初始焦点、Tab 约束、Esc、关闭恢复；`ModalHost` 背景兄弟 inert 后活动弹窗仍可操作；双层弹窗栈只让栈顶响应，并在最后关闭时清理 inert。
- 封面缓存确认取消和两种确认路径。
- 账号菜单的 `menu` / `menuitemradio` / `menuitem` / `separator` 语义与完整键盘操作。
- `ConfirmDialog` 的 destructive、warning、neutral 映射、按钮色和默认焦点。
- 设置搜索命中和无命中。
- Web 下载 preview 的 normal/empty/loading/error。
- 任务/通知标题层级和空状态动作。

### 8.2 Web 真实渲染

页面级回归继续覆盖：

| 维度 | 值 |
|---|---|
| 核心主题 | 浅色、深色、跟随系统 |
| 跟随系统 | system light、system dark、应用运行时浅→深和深→浅切换 |
| 核心窗口 | `960×640`、`1080×760`、`1440×900` |
| 定向 smoke 窗口 | `1280×800`、`1600×900` |
| 数据形态 | 空账号、单账号、多账号、长账号名、无封面、大数据分页 |
| 状态 | 正常、空、加载、错误、禁用、长文本、溢出 |
| 交互 | 下拉、弹窗、键盘焦点、滚动、危险确认 |

核心页面继续按三主题和三核心窗口验收。`1280×800`、`1600×900` 只需对本阶段改动页面、共享 AppShell、Dialog/Menu 和宽度策略做定向 smoke，无需把全部页面扩成五尺寸全交叉。数据形态也按受影响页面定向组合，不要求与所有尺寸做笛卡尔积。

每轮不一定保存 108 张新截图，但必须至少保存每个变更点的前后证据，并自动检查 body 横向溢出和视口外裁切。system light、system dark 与运行时切换都要留下可核对结果，不能只把“跟随系统”当作当前机器的一次静态主题。截图继续放在仓库外。

### 8.3 真实 Tauri 人工复核

Web 验收完成后，单列验证：

- 自绘标题栏拖动、双击最大化、最小化、关闭和 Snap Layout。
- Windows `100% / 125% / 150% / 200%` 缩放及跨显示器 DPI。
- WebView2 字体、高对比度、强制颜色和系统主题实时切换。
- 原生文件选择器返回后的弹窗焦点。
- 托盘隐藏/恢复后的窗口焦点。
- 首次引导、启动恢复和真实加载/失败状态。
- 真实下载 preview 的数量和去重说明。

登录窗口只能人工验证，不得自动登录或处理 CAPTCHA。

## 9. Windows 安装包与测试状态清理边界

普通 `npm run app:build` 是前端生产构建，不等同于 Windows 安装包构建。

若主代理执行并成功完成 Windows 安装包构建，必须遵守项目 `AGENTS.md` 的生产构建后清理规则：

1. 确认构建成功且安装包实际存在并记录路径。
2. 只根据应用身份、完整可执行路径、命令行、PID/创建时间停止 Emily 管理的应用、采集器和隔离浏览器。
3. 读取并校验 `bootstrap.json` 和所有受管数据根。
4. 只清理已验证的受管配置、登录状态、数据库、缓存、日志、WebView2/profile、开机自启值。
5. 不删除安装包、源码、安装目录或受管根外用户文件。
6. 构建失败时不得清理现有测试状态。

## 10. 完成定义

只有同时满足以下条件，才能宣称本文前五项优先优化完成：

- UI-01 至 UI-05 以及 UI-07 的令牌治理已实施并通过对应自动化；UI-07 不能因编号不连续而遗漏。
- P1 三项全部关闭，不能只做视觉遮盖。
- 三主题、三核心窗口无页面级横向溢出和视口外裁切。
- `1280×800`、`1600×900` 定向 smoke，以及空账号、单账号、多账号、长账号名、无封面和大数据分页均有明确结果。
- Web fixture 可稳定展示正常、空、加载、错误、禁用、长文本和溢出状态。
- 键盘可完成主要导航、菜单、弹窗确认和取消，焦点始终可见。
- 真实 Tauri 待复核项有明确结果；未验证项保持为待验收，不能写成已通过。
- 已保存任务开始时的 dirty 路径/状态快照；每阶段均以 `git diff -- <UI paths>` 和 staged diff 核对范围，提交中只含本阶段文件；开始时已有的无关 Rust dirty 路径及状态保持不变。
- `npm test`、`npm run typecheck`、`npm run app:build` 均通过；若未运行或失败，必须如实记录。

## 11. 终审清单

主代理在最后一次集中终审时检查：

- [ ] 主操作按钮四组 token 数学对比度，以及浅色、深色、system light、system dark 四个上下文的真实 computed style。
- [ ] 所有 modal/confirm 的 `ModalHost`、焦点、弹窗栈和背景隔离。
- [ ] 封面缓存取消状态。
- [ ] `960×640` 搜索页滚动所有权。
- [ ] 小字号、小目标和危险/选择颜色语义。
- [ ] 账号菜单单一 menu 模型、三类 ConfirmDialog 语义、设置搜索和空状态层级。
- [ ] Web demo 状态与真实 Tauri 状态的边界说明。
- [ ] 三主题 × 三核心窗口 Browser 回归；system light/dark 运行时切换和两种扩展窗口定向 smoke。
- [ ] Tauri 标题栏、DPI、文件选择器和托盘人工复核。
- [ ] 路径限定的 Git diff、staged diff、既有 dirty 路径不变、测试结果和剩余风险。
