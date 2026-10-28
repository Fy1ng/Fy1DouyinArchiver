# 账号登录恢复与受控浏览器验收记录

## 构建信息

- 日期：2026-07-14
- 分支：`codex/stage-three-roadmap`
- 实施基线：`643c5dd4772076821f7ecdaf6ef7d6fdc9bcbc3b`
- Windows：Microsoft Windows 10 专业版 10.0.19045，build 19045
- Edge：150.0.4078.65
- Chrome：150.0.7871.102
- 安装包：`src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe`
- SHA-256：`905c8b904d2c622ac274d69cdaafafb6b6557b5385db5dd19cdff292e4ec9659`

## 自动门禁

| 门禁 | 结果 | 证据 |
| --- | --- | --- |
| 前端测试 | 通过 | 5 个测试文件、18 项测试通过 |
| TypeScript 类型检查 | 通过 | `npm run typecheck` 退出码 0 |
| 前端生产构建与 manifest | 通过 | Vite 构建和 manifest 生成退出码 0 |
| Rust 全套测试 | 通过 | 单元、集成、schema 与 CDP transport 测试通过 |
| Edge 可见/隐藏 | 通过 | `run-browser-constraint-matrix.ps1` 各执行 1 项测试 |
| Chrome 可见/隐藏 | 通过 | `run-browser-constraint-matrix.ps1` 各执行 1 项测试 |
| NSIS 构建 | 通过 | `cargo tauri build` 生成单一 x64 安装包 |
| 实际安装目录扫描 | 通过 | 随机 `%TEMP%` 静默安装、递归扫描并卸载 |
| 禁止运行时 | 通过 | 未发现 Node、Playwright、NCC、CRI、source map 或随包浏览器 |

## 需求状态

| ID | 自动验证 | 安装包人工验证 |
| --- | --- | --- |
| R1 浏览器关闭后恢复 | progress、CDP close、两秒 watchdog 与清理测试通过 | 待关闭窗口实测 |
| R2 无账号设置可用 | Svelte 真实渲染测试通过 | 待安装包首次启动实测 |
| R3 校验成功后才显示账号 | pending 查询过滤、DOM 过滤和成功边界测试通过 | 待真实账号实测 |
| R4 抑制登录/同步提示 | 四模式参数和 profile 合并测试通过 | 待 Windows Microsoft/Google 账号环境观察 |
| R5 禁用扩展与同步 | 参数、偏好和 profile 越界测试通过 | 测试扩展正向对照尚待执行 |
| R6 失败使用全新 profile | prelaunch 失败、延迟清理和启动恢复测试通过 | 待连续失败/重试实测 |
| R7 不影响普通浏览器 | 路径与全局策略零写入实现完成 | 快照脚本因普通 Edge/Chrome 仍在运行而按设计拒绝，待关闭后执行 |

## 待执行人工步骤

1. 关闭全部普通 Edge/Chrome，运行 `verify-browser-boundaries.ps1 -Mode Before`。
2. 从安装包完成 Edge 与 Chrome 的登录、提前关闭、成功添加、可见同步和隐藏同步。
3. 在受控 profile 安装测试扩展，完成无约束正向对照与生产参数禁用验证。
4. 运行 `verify-browser-boundaries.ps1 -Mode After`，要求普通 profile 与四个策略路径摘要不变。
5. 提供上一安装包、SHA、基线 commit 和隔离 app-data，运行升级基线脚本并记录数据/profile 保留结果。

## 结论

自动实现和发布物扫描门禁通过。由于 R4、R5、R7 与升级路径仍包含人工环境验证，本阶段当前状态为“自动门禁完成，安装包验收待完成”，不得标记为发布完成。
