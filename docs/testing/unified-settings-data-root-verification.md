# 统一设置与数据根自动验收记录

## 验收状态

- 日期：2026-08-11
- 分支：`codex/stage-three-roadmap`
- 结论：自动门禁与安装包扫描完成，真实账号、真实升级和干净 Windows 人工验收待完成
- 数据边界：自动测试仅使用任务级临时目录、固定测试标识和夹具数据库；未读取、迁移或删除真实用户数据库、账号、Cookie、profile 或自定义数据目录

## 发布物

- 安装包：`src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe`
- 安装包大小：3,461,913 字节
- SHA-256：`12e0f481c299943b7e358c75ff6bdb09d9e3345c17dad97ffd57627486880f0b`

## 自动门禁

| 门禁 | 结果 | 证据 |
| --- | --- | --- |
| Rust 全套测试 | 通过 | 268 项非忽略测试通过，0 失败；2 项真实系统浏览器测试按设计 ignored |
| 前端测试 | 通过 | 8 个测试文件、55 项测试通过 |
| TypeScript 类型检查 | 通过 | `npm run typecheck` 退出码 0 |
| 前端构建与 manifest | 通过 | Vite 构建成功，`dist/build-manifest.json` 重新生成且工作区无前端产物差异 |
| 无 Node 正式打包 | 通过 | 构建子进程 PATH 排除 Node 相关目录后，`node`、`npm`、`npx` 均不可解析，`cargo tauri build` 仍成功 |
| NSIS 安装目录扫描 | 通过 | 安装包在随机临时目录静默安装、递归扫描、静默卸载和清理成功 |
| 禁止生产运行时 | 通过 | 安装目录未发现 Node、NCC、CRI、Playwright、Chromium、Headless Shell、随包浏览器或 source map |

## 数据根覆盖

| 范围 | 自动验证证据 |
| --- | --- |
| bootstrap 与 marker | 默认数据根创建和重复解析稳定；bootstrap 的 `rootId` 必须与数据根 marker 匹配；原子替换保留 current 与 retained roots |
| 路径规则 | 普通临时目录可用；安装目录根规范化为 `Data`；磁盘根、UNC、mapped remote、重叠路径、非空目录、reparse point 和非 Unicode 路径被拒绝 |
| 权限探测 | 文件创建、读写、重命名、删除及 SQLite WAL/SHM 探测无残留；真实只读 ACL 目录映射为稳定权限错误且不请求提权 |
| 迁移原子性 | 迁移前执行 inventory 与关键哈希校验；bootstrap 提交前故障继续使用旧根；bootstrap 已写入后的恢复能够提交新 runtime；维护租约阻止并发访问 |
| legacy-flat 升级 | 夹具数据库保留 1 条记录和 `user_version = 4`；旧受管条目登记为 retained legacy-flat；第二次解析仍使用同一数据根；浏览器选择及登录验证标记仍可读取 |
| 删除边界 | 当前根不可作为 retained root 删除；标准根按 marker 身份删除；legacy-flat 仅删除 allowlist；内部 junction、身份变化和未列出条目均失败关闭 |
| 安装器生命周期 | `/UPDATE` 跳过数据删除；卸载默认保留数据；仅用户显式勾选时调用严格 helper；helper 失败会中止卸载数据清理 |

## 复现命令

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
npm test
npm run typecheck
npm run app:build

$filteredPath = @($env:PATH -split ';' | Where-Object {
  $_ -and $_ -notmatch '(?i)(fnm|nodejs|\\npm(?:\\|$)|\\node(?:\\|$))'
})
$env:PATH = $filteredPath -join ';'
if (@(Get-Command node,npm,npx -ErrorAction SilentlyContinue).Count -ne 0) {
  throw 'Node commands are still available.'
}
cargo tauri build

& scripts/audit-installed-package.ps1 `
  -Installer 'src-tauri/target/release/bundle/nsis/抖音观看历史归档器_0.1.0_x64-setup.exe' `
  -ExpectedSha256 '12e0f481c299943b7e358c75ff6bdb09d9e3345c17dad97ffd57627486880f0b'
```

## 待人工验收

1. 从正式安装包选择真实普通目录和安装目录根，确认后者实际使用 `<安装目录>\Data`，重启后仍打开同一数据库。
2. 选择当前用户确实不可写的真实目录，确认应用不触发 UAC、不留下空目录，并提示更换位置。
3. 在真实文件复制中和 bootstrap 切换前分别终止进程，确认重启后只打开旧数据库且目标目录不被继续写入。
4. 使用真实账号完成数据根迁移后的隐藏同步、重启和再次查询，确认数据库、profile 与登录状态都来自新根。
5. 使用上一版本安装包执行覆盖升级，确认数据库、统一设置、安装目录 `Data` 和 profiles 不被覆盖或删除；再分别验证卸载时保留与显式删除数据。
6. 在未安装 Node、npm、npx 且没有项目源码的干净 Windows x64 环境安装，验证启动、设置读取、目录迁移、登录、同步、取消和重启恢复。

## 结论

统一设置、数据根迁移、legacy-flat 升级、卸载数据边界、无 Node 正式构建和安装目录扫描的自动门禁均已通过。真实账号迁移、真实进程中断、上一版本覆盖升级和干净 Windows 运行仍需要人工环境，因此 3.2 当前只能标记为“自动门禁完成”，不能标记为发布验收完成。
