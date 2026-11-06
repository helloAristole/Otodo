# Otodo

一款轻量、透明、始终在手边的 Windows 日历待办应用。

Otodo 将日历和待办放在桌面上，方便快速查看日程、记录任务和保持专注。应用无需连接后端，数据仅保存在本机。

![Otodo 周视图](doc/image-1.png)

## 功能特点

- 日视图、周视图和月视图，快速切换不同时间范围
- 显示农历和节气信息
- 支持每天、每周、每月等重复待办
- 未完成待办可自动顺延到当天
- 内置可自定义时长的番茄钟，托盘菜单可查看剩余时间
- 关闭窗口后驻留系统托盘，需要时快速唤起
- 支持窗口置顶、位置锁定和开机启动
- 使用本地 SQLite 存储，无需注册或登录

## 下载

[下载最新版](https://github.com/helloAristole/Otodo/releases/latest)

当前版本：[`v0.1.21`](https://github.com/helloAristole/Otodo/releases/tag/v0.1.21)

- [下载 Windows x64 安装程序（EXE）](https://github.com/helloAristole/Otodo/releases/download/v0.1.21/Otodo_0.1.21_x64-setup.exe)

目前仅提供 Windows 版本。

## 数据与隐私

Otodo 是纯桌面应用，不依赖远程后端服务。待办和设置保存在本机，不会自动上传或同步到云端。

卸载应用前，如需保留数据，请先自行备份本地数据。

## 本地开发

### 环境要求

- Node.js LTS
- Rust stable
- Windows 10 或 Windows 11
- [Tauri 2 开发环境依赖](https://v2.tauri.app/start/prerequisites/)

### 启动开发环境

```bash
npm install
npm run tauri dev
```

### 构建安装包

```bash
npm run tauri build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 技术栈

- React
- TypeScript
- Vite
- Tauri 2
- Rust
- SQLite
