# Rust CDP 观看历史采集器开发说明

生产采集器完全由 Rust 实现，不依赖 Node、Playwright 或 `chrome-remote-interface`，也不下载或随安装包分发 Chromium。运行时通过 CDP 控制系统安装的 Microsoft Edge 或 Google Chrome。

## 浏览器与 profile

- Windows x64 自动选择顺序：Edge、Chrome。
- Edge profile：`<browser-data-dir>/profiles/edge/`。
- Chrome profile：`<browser-data-dir>/profiles/chrome/`。
- 活动浏览器：`<browser-data-dir>/browser-selection.json`。
- 两种浏览器不共享登录态；首次迁移到 CDP 必须重新登录。
- 不读取、迁移或删除旧 Playwright profile。

## 登录

```powershell
cargo run --manifest-path src-tauri/Cargo.toml -- --collector login `
  --browser-data-dir data/cdp-browser `
  --browser auto
```

登录命令打开可见系统浏览器。检测到观看历史页面后写入浏览器选择和登录验证标记，然后关闭浏览器。

开发验收可将 `--browser auto` 改为 `edge` 或 `chrome`，分别验证两种浏览器。

## 采集

```powershell
cargo run --manifest-path src-tauri/Cargo.toml -- --collector collect `
  --job-id 1 `
  --result data/prototype/result.json `
  --progress data/prototype/progress.json `
  --diagnostic data/prototype/diagnostic.json `
  --cancel-file data/prototype/cancel `
  --browser-pid-file data/prototype/browser.pid `
  --browser-data-dir data/cdp-browser `
  --browser auto `
  --days 7 `
  --headless `
  --block-images
```

隐藏采集必须复用已验证的同一浏览器和 profile。图片与视频媒体默认阻止加载；React 快照仍从页面状态读取封面 URL、记录顺序和观看时间。

隐藏同步在私有 Windows Desktop 中启动浏览器，并以后台、非聚焦 CDP Target 采集。完整 Chromium 即使创建平台窗口，也不会出现在当前交互桌面。

## 测试与构建

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1
cargo tauri build
```

浏览器夹具默认使用 Edge。设置 `EMILY_TEST_BROWSER=chrome` 可强制验收 Chrome。

Svelte 前端开发仍可选择安装 Node 依赖并运行：

```powershell
npm run app:dev
npm test
npm run typecheck
npm run app:build
```

`npm run app:build` 会同时更新受 Git 跟踪的 `dist/` 与 `dist/build-manifest.json`。正式打包不执行 Node、npm 或 Vite，只读取已提交产物，并由 Rust `build.rs` 复核输入、输出、长度和 SHA-256。
