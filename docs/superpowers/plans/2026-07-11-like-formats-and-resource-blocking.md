# Like Formats And Resource Blocking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 固定阻止同步图片和视频媒体请求，并提供五种点赞量格式及正确的缺失观看进度显示。

**Architecture:** 显示格式由前端纯函数负责，设置页面只保存用户可调字段；同步资源策略由 Rust 调用构造器统一强制，避免前端配置绕过。旧本地设置保持容错读取。

**Tech Stack:** Svelte 5、TypeScript、Vitest、Rust、Serde、Tauri 2

## Global Constraints

- 图片和视频媒体请求默认且始终阻止。
- 点赞量选项文字固定为“准确数字 / W / 万 / K / 千”。
- 缺失观看进度不显示任何百分比。
- 现有 `exact`、`wan`、`k` 本地设置继续有效。

---

### Task 1: 点赞量与观看进度格式化

**Files:**
- Modify: `tests/app/formatters.test.ts`
- Modify: `app/src/types.ts`
- Modify: `app/src/formatters.ts`

**Interfaces:**
- Produces: `LikeCountFormat = "exact" | "w" | "wan" | "k" | "qian"`
- Produces: `formatLikeCount(value, format)` 和 `formatWatchProgress(value)`

- [ ] **Step 1: 写失败测试**

增加 `W`、`千`、五种阈值和 `undefined` 进度断言，同时保留明确数值 `0` 显示“已看 0%”。

- [ ] **Step 2: 验证测试失败**

Run: `npm test -- tests/app/formatters.test.ts`
Expected: `w`、`qian` 类型或格式断言失败。

- [ ] **Step 3: 最小实现**

扩展联合类型，并按 10,000/1,000 阈值复用一位小数格式化函数。

- [ ] **Step 4: 验证测试通过**

Run: `npm test -- tests/app/formatters.test.ts`
Expected: PASS

### Task 2: 设置界面与持久化

**Files:**
- Modify: `app/src/App.svelte`
- Modify: `app/src/types.ts`

**Interfaces:**
- Consumes: Task 1 的 `LikeCountFormat`
- Produces: 五项 segmented control；不含资源拦截设置的 `SyncOptions`

- [ ] **Step 1: 删除前端资源开关状态、UI、保存和同步参数**

`buildSyncOptions()` 只返回范围、滚动速率和浏览器可见性。

- [ ] **Step 2: 添加五个点赞量选项**

按钮标签依次为 `准确数字`、`W`、`万`、`K`、`千`，加载设置白名单同步扩展。

- [ ] **Step 3: 验证类型和前端构建**

Run: `npm run typecheck && npm run app:build`
Expected: PASS

### Task 3: Rust 固定资源拦截

**Files:**
- Modify: `src-tauri/tests/runtime.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: 不含资源开关的 `SyncOptions`
- Produces: 始终包含 `--block-images` 且永不包含 `--no-block-media` 的采集器调用

- [ ] **Step 1: 写失败测试**

修改运行时测试，使默认 `SyncOptions` 也必须包含 `--block-images`，并断言不存在 `--no-block-media`。

- [ ] **Step 2: 验证测试失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml bundled_invocation_uses_node_resource_and_task_files -- --nocapture`
Expected: 默认调用缺少 `--block-images`。

- [ ] **Step 3: 最小实现**

从 Rust `SyncOptions` 删除两个字段，调用构造器无条件添加 `--block-images`。

- [ ] **Step 4: 完整验证与安装包**

Run: `npm test`
Run: `npm run typecheck`
Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Run: `npm run tauri:bundle`
Expected: 全部退出码为 0，并生成 Windows x64 NSIS 安装包。
