# 首页登录恢复自动验收记录

## 验收状态

- 日期：2026-08-11
- 分支：`codex/stage-three-roadmap`
- 结论：自动门禁完成，真实账号与干净系统人工验收待完成
- 数据边界：仅使用隔离临时 profile、脱敏固定身份和本地 HTTP 资源夹具；未记录账号标识、Cookie、二维码、验证码或页面文本快照

## 环境

- Windows：Microsoft Windows 10 专业版 10.0.19045，build 19045
- Edge：151.0.4129.72
- Chrome：151.0.7922.108
- 安装包：`src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe`
- 安装包大小：3,301,485 字节
- SHA-256：`f608f93f34a1db9f2a4b5baccea1db6c4ea7347378bc022f3d553b89d133a976`

## 自动门禁

| 门禁 | 结果 | 证据 |
| --- | --- | --- |
| Rust 全套测试 | 通过 | 221 项非忽略测试通过，0 失败 |
| 前端测试 | 通过 | 8 个测试文件、43 项测试通过 |
| TypeScript 类型检查 | 通过 | `npm run typecheck` 退出码 0 |
| 前端构建与 manifest | 通过 | Vite 构建及 `dist/build-manifest.json` 生成成功 |
| Edge 系统浏览器夹具 | 通过 | 2 项 ignored 验收测试通过，耗时 22.95 秒 |
| Chrome 系统浏览器夹具 | 通过 | 2 项 ignored 验收测试通过，耗时 16.30 秒 |
| NSIS 构建 | 通过 | `cargo tauri build` 生成单一 Windows x64 安装包 |
| 安装目录扫描 | 通过 | 随机 `%TEMP%` 静默安装、递归扫描并卸载成功 |
| 禁止生产运行时 | 通过 | 未发现 Node、NCC、CRI、Playwright、Chromium、Headless Shell 或 source map |

## 系统浏览器夹具覆盖

- `EMILY_TEST_BROWSER` 未设置时默认 Edge，设置为 `chrome` 时强制 Chrome；不会在浏览器锁定失效时自动切换。
- 可见会话拥有普通桌面窗口；隐藏会话位于 `EmilyCollector-*` 私有桌面，顶层可见窗口枚举为 0。
- 首页夹具仅在规范 `https://www.douyin.com/` URL 上注入脱敏状态：未登录、一次“一键登录”、已登录、可选保存弹窗和身份冲突均通过。
- “一键登录”与“保存”各只产生一次真实 CDP 鼠标点击；按钮消失后不重复点击。
- 二维码与验证码图片请求到达本地夹具；媒体请求被 Fetch 阻止，未到达服务器。
- 正常 Fetch 终态满足 `paused == decided`、`duplicates == 0`、`pending == 0`；重复暂停 transport 测试只发送一次决策并记录一次 duplicate。
- 关闭前显式执行 `Fetch.disable`；正常完成后浏览器进程退出、调试端口关闭、profile 可删除且 `exit_type` 为 `Normal`。
- 隐藏恢复期间写入取消标记后返回 `collector_cancelled`，不产生结果文件，保留任务取消证据，并清理浏览器进程和调试端口。

## 复现命令

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
npm test
npm run typecheck
npm run app:build

$env:EMILY_TEST_BROWSER='edge'
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1 --nocapture

$env:EMILY_TEST_BROWSER='chrome'
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1 --nocapture

cargo tauri build
& scripts/audit-installed-package.ps1 `
  -Installer 'src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe' `
  -ExpectedSha256 'f608f93f34a1db9f2a4b5baccea1db6c4ea7347378bc022f3d553b89d133a976'
```

## 待人工验收

1. Edge 与 Chrome 各执行 10 次真实账号隐藏恢复，每次总耗时不超过 30 秒。
2. 验证正常同步不触发恢复，首次 `session_expired` 仅恢复一次，成功后只重试一次采集。
3. 在真实二维码或验证码中间页确认应用不自动操作，并允许用户完成验证。
4. 使用另一账号制造真实身份冲突，确认不导入、不覆盖任何历史数据。
5. 从恢复阶段开始至终态后 2 秒进行 10 fps 桌面录制和 HWND/任务栏枚举，确认无白色或浏览器 composition surface。
6. 在无 Node、npm、npx 和项目源码的干净 Windows x64 环境安装并验证登录、同步、取消与重启恢复。

## 结论

代码、系统浏览器夹具、自动测试、NSIS 构建和安装目录扫描均已通过。真实账号被挤下线、验证码、身份冲突、10 轮性能和干净系统运行仍需人工环境配合，因此当前不得标记为发布完成。
