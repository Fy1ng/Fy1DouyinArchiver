# 托盘、首次引导与后台调度 Stage 3.3 验证记录

## 验收状态

- 日期：2026-08-13
- 分支：`codex/stage-three-roadmap`
- 验证起点：`4d346a0`；本记录包含本轮 WebView2 自愈与 Windows Sandbox 验收改动
- 结论：自动门禁、系统浏览器夹具、无 Node 打包、隔离安装树审计、Windows Sandbox WebView2 自愈、基础桌面生命周期和十分钟资源阈值通过；真实账号生命周期、完整升级/卸载、安装器中断恢复、调度器真实唤醒次数和 Electron 对照仍阻塞发布
- 数据边界：本轮仅使用测试夹具、任务级临时 profile 和随机 `%TEMP%` 安装目录；未读取、迁移或删除真实用户数据库、账号、Cookie、浏览器 profile 或自定义数据目录

## 环境与发布物

- Windows：Microsoft Windows 10 专业版 10.0.19045，build 19045
- Rust：`rustc 1.96.1`，`cargo 1.96.1`
- Tauri CLI：`2.11.4`
- Edge：`151.0.4129.72`
- Chrome：`151.0.7922.108`
- 安装包：`src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe`
- 安装包大小：5,934,950 字节
- SHA-256：`48099d0169bce01eaca225428a6895ff30dda3d8831e55d0acea20786196cca3`
- 签名边界：应用安装包未签名；内嵌的 Microsoft WebView2 Bootstrapper Authenticode 签名有效
- WebView2 Bootstrapper：1,695,960 字节，SHA-256 `8c4a80540b6bbcbef30a4e8c7d1ac504b6fc09db922b4acdfd85c9d5f6f1050e`

## 自动门禁

| 门禁 | 结果 | 证据 |
| --- | --- | --- |
| 前端测试 | 通过 | `npm test`：10 个测试文件、92 项测试通过，0 失败 |
| TypeScript 类型检查 | 通过 | `npm run typecheck` 退出码 0 |
| 前端构建与 manifest | 通过 | `npm run app:build` 成功，`dist/` 与 Git 基线无差异 |
| Rust 全套测试 | 通过 | 30 个测试结果摘要，496 项通过、0 失败、3 项按设计 ignored |
| Edge 系统浏览器夹具 | 通过 | 2 项 ignored 测试单独启用后通过，耗时 23.17 秒 |
| Chrome 系统浏览器夹具 | 通过 | 2 项 ignored 测试单独启用后通过，耗时 16.61 秒 |
| HKCU 自启动集成测试 | 通过 | 使用随机唯一 Run 键名的 1 项 ignored 测试单独启用后通过，测试值已由显式删除和 `Drop` 清理 |
| 无 Node 正式打包 | 通过 | 独立 PowerShell 的 PATH 仅保留 Cargo 和 Windows 必需目录；`node`、`npm`、`npx` 均不可解析，`cargo tauri build` 成功 |
| NSIS 契约 | 通过 | `runtime` 与 `uninstall_lifecycle` 共 59 项通过；模板 CRLF、上游模板哈希、构建身份、manifest、严格 CLI 契约、WebView2 载荷验证和内嵌 Bootstrapper 模式均有覆盖 |
| 安装树审计 | 通过 | 安装包在随机 `%TEMP%` 目录静默安装、递归扫描并通过 `/S /UPDATE` 清理；目录与相关卸载进程均无残留 |
| 禁止生产运行时 | 通过 | Git 跟踪发布输入、生成 NSIS 脚本与测试安装目录未发现 Node、npm、npx、NCC、`index.cjs`、CRI、Playwright、Chromium、Headless Shell、随包浏览器或 source map |
| Windows Sandbox 自愈 | 通过 | 未映射宿主 Runtime；安装前注册表错误声明 `141.0.3537.71`，但 `msedgewebview2.exe`、`msedge.dll`、`icudtl.dat` 均缺失；安装器识别残缺载荷并安装完整 `151.0.4129.78` |
| Windows Sandbox 桌面冒烟 | 通过 | 应用显示启动、二次实例激活、普通静默卸载默认保留数据均通过；退出前捕获的 7 个进程明确包含应用与 `msedgewebview2.exe`，退出后原 PID/创建时间身份及应用相关进程均零残留 |
| 十分钟基础资源阈值 | 通过 | 120 个样本、5 秒间隔；私有工作集中位数 76,271,616 字节，p95 79,589,376 字节；CPU 平均 0.000164%，p95 0%；未观察到系统 Edge/Chrome 采集进程 |

完整 Rust 门禁默认忽略 3 项需修改系统状态或启动系统浏览器的测试。Edge/Chrome 的 2 项 CDP 测试及使用随机唯一键名的 1 项 HKCU 自启动测试均已分别显式运行并通过。

## 功能覆盖

| 范围 | 自动验证证据 |
| --- | --- |
| 首次引导与设置 | 四步引导按序提交；portable/device 设置分离；失败可重试；已有用户兼容；设置读取、CAS 和跨字段约束严格 |
| 托盘与单实例 | 六项固定托盘菜单；手动二次启动激活已有窗口；静默自启不抢前台；关闭决策区分退出、托盘和首次引导状态 |
| 后台调度 | Rust worker 生命周期、提前唤醒、最多 30 秒空闲等待、启动延迟、最近终态锚点、暂停及来源门禁均有测试 |
| 队列与批次 | 手动优先、任务键互斥、批量分页、配置变化替换排队自动任务、运行中快照冻结和取消边界均通过 |
| 通知 | 登录失效、连续第三次自动同步失败、成功后重置和终态去重均由桌面通知测试覆盖 |
| 退出与恢复 | 退出 marker 作为线性化点；先停调度再取消和清理；硬超时保留证据；启动恢复按固定顺序处理运行任务和故障 |
| 安装器生命周期 | 从未启动安装、manifest 删除后的安装根清理、prepare/commit/abort/takeover 状态机、进程身份和数据边界均有确定性测试 |
| 多来源 3.3 边界 | 观看历史 executor 已接入；来源设置、队列、结果 envelope、archive/folder 契约与能力注册表已冻结；喜欢与收藏执行器仍不在本阶段注册 |

## 安装包审计边界

`scripts/audit-installed-package.ps1` 使用 `/S /UPDATE` 清理随机测试安装，只用于验证安装文件树、禁止运行时和无残留进程，不读取或修改开发者真实 app-data。它不等同于普通用户卸载、保留数据、删除数据或跨版本升级验收，这些项目仍列为发布阻塞。

源码模板中唯一的 `Chromium` 文本是上游 NSIS 更新器文档 URL 注释；安装配置无 bundle resources，生成 NSIS 脚本和实际安装目录均无对应运行时文件。

## 复现命令

```powershell
npm test
npm run typecheck
npm run app:build
cargo test --manifest-path src-tauri/Cargo.toml

$env:EMILY_TEST_BROWSER='edge'
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1 --nocapture

$env:EMILY_TEST_BROWSER='chrome'
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1 --nocapture

cargo test --manifest-path src-tauri/Cargo.toml --test autostart real_hkcu_round_trip -- --ignored --test-threads=1 --nocapture
```

正式打包在独立 `pwsh -NoProfile` 子进程中执行；其 PATH 仅包含 `%USERPROFILE%\.cargo\bin`、Windows 系统目录和 Git，先断言 `Get-Command node,npm,npx` 全部失败，再执行：

```powershell
cargo tauri --version
cargo tauri build

& scripts/audit-installed-package.ps1 `
  -Installer 'src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe' `
  -ExpectedSha256 '48099d0169bce01eaca225428a6895ff30dda3d8831e55d0acea20786196cca3'

& scripts/run-stage33-sandbox.ps1 `
  -Installer 'src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe' `
  -ExpectedSha256 '48099d0169bce01eaca225428a6895ff30dda3d8831e55d0acea20786196cca3' `
  -IdleMinutes 10 `
  -OutputRoot 'data/verification/stage33-sandbox-webview2-self-heal-long-final'
```

长测证据：`data/verification/stage33-sandbox-webview2-self-heal-long-final/8e3a1b5c912045048b8a925b454bd3fa/output/stage33-sandbox-result.json`。该文件包含安装前后 Runtime 状态、120 个原始资源样本、受控退出进程身份和默认保留数据卸载事件。

## 待人工验收与发布阻塞

| 门禁 | 当前状态 | 最小验收范围 |
| --- | --- | --- |
| 真实账号桌面生命周期矩阵 | 未执行，阻塞发布 | 手动显示、静默自启、二次启动、关闭到托盘、托盘恢复/设置/退出、暂停/恢复、15 分钟两周期自动同步、手动优先、睡眠恢复、登录失效、第三次失败通知、退出后零采集器/浏览器残留 |
| 真实升级与普通卸载 | 未执行，阻塞发布 | 同路径升级；默认保留数据；显式删除数据；安装目录 `Data` 与自定义数据根；升级/卸载后数据库、设置和 profile 边界正确 |
| 安装器中断恢复矩阵 | 未执行，阻塞发布 | Preparing/Prepared 中断回滚、DestructiveCommitted 中断失败关闭、继续卸载、repair takeover，以及存在非终态 fault 时 `/UPDATE` 拒绝 |
| 干净 Windows x64 | 部分通过，仍阻塞发布 | Windows Sandbox 已验证 WebView2 自愈、启动、二次实例、退出和默认保留数据卸载；首次引导交互、登录、可见/隐藏同步、取消、重启恢复、自启和托盘仍需完整人工矩阵 |
| 十分钟资源测量 | 基础阈值通过，仍阻塞发布 | 120 个样本的内存与 CPU 阈值通过且无系统浏览器残留；尚未取得调度器真实唤醒次数，也未完成同窗口 Electron 对照 |
| Stage 3.2 真实安装门禁 | 仍未完成，阻塞发布 | 真实账号数据根迁移、真实进程中断、上一版本覆盖升级、保留/删除数据和干净系统运行 |

资源目标中的绝对阈值已通过：中位私有工作集约 72.7 MiB，p95 约 75.9 MiB，平均 CPU 0.000164%，p95 CPU 0%。调度唤醒不超过每分钟 3 次和 Emily 中位内存不超过 Electron 对照 80% 尚无实测证据，因此资源门禁整体仍未关闭。

## 结论

观看历史生命周期完成，多来源设置/队列/结果契约已冻结，archive 执行器与最终多来源验收待 3.4。

当前安装包是通过自动门禁、Windows Sandbox WebView2 自愈、基础桌面生命周期和十分钟绝对资源阈值的 Stage 3.3 候选包，不是已经完成发布验收的正式发布包。真实账号桌面生命周期、真实升级/卸载、安装器中断恢复、干净 Windows 完整矩阵、调度器唤醒/Electron 对照和 Stage 3.2 真实安装门禁仍需完成，并继续阻塞发布。
