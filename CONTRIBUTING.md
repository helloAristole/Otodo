# 贡献指南

感谢你愿意帮助改进 Otodo。

## 提交问题

- Bug 请使用 Bug 模板，并提供系统版本、应用版本和复现步骤。
- 功能建议请说明使用场景、现有问题和期望结果。
- 提交前请先搜索已有 Issue，避免重复。
- 请勿在 Issue、日志或截图中提交密钥、个人数据等敏感信息。

## 本地开发

环境要求：

- Node.js LTS
- Rust stable
- Windows 10 或 Windows 11
- [Tauri 2 开发环境依赖](https://v2.tauri.app/start/prerequisites/)

启动开发环境：

```bash
npm install
npm run tauri dev
```

构建安装包：

```bash
npm run tauri build
```

## 提交代码

1. 从 `main` 创建独立分支。
2. 一次提交只处理一个明确问题。
3. 保持代码易读，不做与当前问题无关的重构。
4. 保持现有业务行为、数据兼容性和异常语义。
5. 提交前运行 `npm run build`，涉及桌面端能力时再运行 `npm run tauri build`。
6. Pull Request 中写清楚改动原因、验证方式和界面变化。

较大的功能建议先创建 Issue 讨论，确认方向后再开始实现。
