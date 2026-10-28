# 抖音观看历史归档器

Windows x64 桌面应用，用于将抖音观看历史持久化到本地 SQLite，并提供搜索、分页、删除、同步范围及观看数据展示。

## 技术栈

- Tauri 2、Rust、SQLite
- Svelte 5、Vite、TypeScript
- Rust 原生 CDP 客户端
- 系统 Microsoft Edge 或 Google Chrome

安装后的应用和正式打包流程不依赖 Node、npm、Playwright、NCC 或内置 Chromium。Node 仅作为可选的前端开发工具。

## 正式构建

需要 Windows x64、Rust 1.95 或更新版本、Cargo、Visual Studio C++ Build Tools、`tauri-cli 2.11.4`，以及系统 Edge 或 Chrome。本次依赖升级使用 Rust 1.96.1 验证。

```powershell
cargo install tauri-cli --version 2.11.4 --locked
cargo test --manifest-path src-tauri/Cargo.toml
cargo tauri build
```

NSIS 安装包生成到：

```text
src-tauri/target/release/bundle/nsis/
```

正式构建直接使用 Git 中的 `dist/`。Rust `build.rs` 会验证 `dist/build-manifest.json`，因此前端源码变化后必须重新生成并一同提交发布产物。

## 前端开发

```powershell
npm install
npm run app:dev
npm test
npm run typecheck
npm run app:build
```

## 原生浏览器夹具

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1
$env:EMILY_TEST_BROWSER="chrome"
cargo test --manifest-path src-tauri/Cargo.toml --test cdp_system_browser -- --ignored --test-threads=1
```

浏览器 profile、数据库、任务文件和诊断均保存在 Tauri `app_data_dir`，不会写入安装目录或源码目录。

详细设计与迁移边界见 [Node-Free Runtime And Packaging Design](docs/superpowers/specs/2026-07-11-node-free-runtime-packaging-design.md)。

## 许可证

Copyright (C) 2026 LifeArchiveProject

除另有说明的第三方材料外, 本项目以 GNU General Public License version 3 only (`GPL-3.0-only`) 发布, 不包含自动适用后续版本的授权. 完整条款见 [LICENSE](LICENSE).

GPL 允许商业使用. 分发本项目或基于本项目的衍生作品时, 必须遵守 GPL 第 3 版, 包括保留相关声明, 并按许可证要求提供对应源代码. 软件按现状提供, 不作任何担保.

第三方代码和素材保留各自的版权及许可证, 见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). 本项目许可证不授予抖音平台内容的版权或再分发权限.
