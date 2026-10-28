# 参与贡献

欢迎提交可复现的问题、文档修正和范围明确的改动. 建议先阅读 [README](README.md)、[数据与隐私](docs/privacy.md) 和 [依赖安全记录](docs/dependency-security.md).

## 问题与建议

- 功能问题请使用问题反馈模板, 附版本、Windows/浏览器环境、复现步骤和预期结果.
- 功能建议请说明实际场景, 避免用真实账号资料作为示例.
- 安全问题按 [SECURITY.md](SECURITY.md) 私下报告, 不要直接公开利用细节或用户数据.

项目仍处于公开源码准备阶段. 不要把旧测试安装包的行为与当前源码或尚未发布的版本混为一谈.

## 开发环境

完整桌面回归使用 Windows x64. 安装 Visual Studio C++ Build Tools 和 Windows SDK, 并准备:

| 工具 | 验证版本 |
| --- | --- |
| Node | 24.15.0 |
| npm | 11.12.1 |
| Rust | 1.96.1 |

在仓库根目录执行:

```powershell
npm ci
npm run typecheck
npm test
npm run app:build
cargo +1.96.1 test --manifest-path src-tauri/Cargo.toml --locked -- --test-threads=1
```

如果尚未安装指定 Rust 工具链, 先执行 `rustup toolchain install 1.96.1 --profile minimal --no-self-update`.

浏览器开发预览使用模拟 API, 不提供真实账号登录或采集. 系统浏览器夹具和真实账号验收有额外环境要求, 见 README 的原生浏览器夹具说明. 不要在常规测试中加入真实账号凭据.

## 前端产物约定

Git 中的 `dist/` 是正式桌面构建的输入. 修改 `app/src/`、前端配置或 npm 包元数据后, 使用上述 Node/npm 版本运行 `npm run app:build`, 将源码、产物和 `dist/build-manifest.json` 一同提交.

可在本地检查产物是否已经提交:

```powershell
git diff --exit-code -- dist
git ls-files --others --exclude-standard -- dist
```

第一条应通过, 第二条应没有输出. 不要手工修改 manifest 哈希, 也不要提交 node_modules、缓存或源码映射文件. 数据根、登录 profile 和导出的备份同样不属于源码仓库.

## CI 检查

[CI 工作流](.github/workflows/ci.yml) 在 main 的推送、目标为 main 的 PR 和手动触发时运行. 两个作业都使用 Windows 2025 托管环境:

- **Frontend**: 固定 Node/npm, 执行锁文件安装、类型检查、前端测试、生产构建, 并拒绝与已提交 dist 不一致的结果.
- **Windows Rust**: 固定 Rust 1.96.1, 使用已提交 dist 执行完整串行 Cargo 回归; build.rs 同时校验前端 manifest.

工作流只读取仓库内容, 不使用项目 secrets、不运行真实账号测试, 也不生成或发布安装包. Action 引用固定到官方仓库的完整提交 SHA, 升级时应同时更新版本注释并重新验证. GitHub 托管执行仍需在仓库发布且启用 Actions 后确认; 本地检查不能证明托管作业已经运行成功.

本地依赖审计命令和当前上游 low 风险见依赖安全记录. 未修复的开发工具告警不会通过全局忽略规则隐藏.

## 提交 PR

每个 PR 尽量解决一个明确问题. 说明行为变化、实际执行的检查和未验证范围; 更新相关文档. 界面截图使用演示数据或完成脱敏.

提交前检查 `git diff` 和暂存区, 确认没有 Cookie、令牌、私钥、登录二维码、真实数据库、原始备份或本机个人路径. 依赖变更还需要更新锁文件、第三方声明和库存.

贡献代码沿用项目的 [GPL-3.0-only](LICENSE) 授权, 第三方材料保留其原有声明. 请只提交自己有权贡献的代码、文档和素材.
