# UniDL

[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri)](https://tauri.app/)
[![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.3-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![License](https://img.shields.io/badge/License-MIT-green.svg)](./LICENSE)

**UniDL**（Unified Download Manager）是一款统一的桌面下载管理应用：在本地集中管理 HTTP/FTP、磁力链接与种子等任务，并可通过浏览器扩展将网页中的下载请求发送到本机服务。

---

## 功能概览

- **多协议来源**：HTTP、FTP、`magnet:` 磁力链接、`.torrent` 种子文件  
- **多下载引擎**：aria2、yt-dlp、qBittorrent（可在应用内配置多套引擎与优先级）  
- **任务管理**：创建、暂停/恢复、批量操作、进度与速度展示、打开已下载文件  
- **种子细节**：解析种子内文件列表、按需选择子文件  
- **系统集成**：`magnet:` 深度链接（见 `tauri.conf.json`）、`.torrent` 文件关联（配置于 Tauri bundle）  
- **浏览器扩展**：Chromium 系 Manifest V3 扩展，通过本机 `http://127.0.0.1:18080` 与桌面端通信  
- **Web 模式**：可通过浏览器访问程序界面，便于将程序反代实现随时访问

---

## 技术栈

| 层级 | 技术 |
|------|------|
| 桌面壳 | [Tauri 2](https://tauri.app/) |
| 前端 | React 19、Vite 6、TypeScript、Tailwind CSS |
| 后端（Rust） | SQLite（rusqlite）、reqwest、内置轻量 HTTP 服务等 |

---

## 环境要求

开发前请准备：

- **Node.js**（建议当前 LTS）与 **npm**
- **Rust** 工具链（[`rustup`](https://rustup.rs/)），并安装各平台 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)  
- 构建 **Windows** 桌面端时，需按 Tauri 文档安装 **Microsoft C++ Build Tools** 等

---

## 快速开始

```bash
# 安装依赖
npm install

# 启动 Web 前端（默认 http://127.0.0.1:1450）
npm run dev

# 启动 Tauri 桌面开发模式（会拉起前端 dev 命令）
npm run tauri dev
```

---

## 浏览器扩展

1. 先运行 **UniDL 桌面应用**（本机服务默认监听 **18080** 端口，与 `extension/manifest.json` 中 `host_permissions` 一致）。  
2. 在 Chrome / Edge 等浏览器中打开 `chrome://extensions`，开启「开发者模式」，选择「加载已解压的扩展程序」，指向仓库中的 **`extension/`** 目录。  
3. 或使用 `npm run package:extension` 生成 zip，按浏览器说明以打包扩展方式安装。

---

## 仓库结构（节选）

```
UniDL/
├── extension/          # Chromium MV3 扩展（与本地 18080 通信）
├── scripts/            # 打包与发布校验脚本
├── shared/             # 前后端共享类型（如 types.ts）
├── src/                # React 前端
├── src-tauri/          # Tauri + Rust 后端
└── package.json
```

---

## 开源协议

本项目以 **MIT** 协议发布，全文见仓库根目录 [`LICENSE`](./LICENSE)。

---

## 贡献与反馈

欢迎通过 Issue / Pull Request 提交缺陷报告、功能建议或代码改进。提交前建议运行 `npm run lint` 与 `npm run build`，确保桌面端相关改动可在本机 `npm run tauri dev` 下验证。
