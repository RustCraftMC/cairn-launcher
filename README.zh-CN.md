<p align="center">
  <img src="assets/logo.png" width="96" alt="CairnLauncher logo">
</p>

<h1 align="center">CairnLauncher</h1>

<p align="center">
  基于 Rust 打造的轻量级 Windows 原生 Minecraft 启动器。<br>
  <a href="README.md">English</a> | 简体中文
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows" alt="Platform">
  <img src="https://img.shields.io/badge/Rust-2024_Edition-black?logo=rust" alt="Rust 2024">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-LGPL_v3-blue.svg" alt="License: LGPL v3"></a>
  <img src="https://img.shields.io/badge/UI-Pure_Native_(No_Electron)-blueviolet" alt="Pure Native UI">
</p>

---

**CairnLauncher** 是一款基于 Rust 开发的现代化 Minecraft 启动器。

它没有采用 Electron 或 WebView 等网页套壳技术，而是基于自研的轻量保留模式图形引擎 `cairn-ui`，利用 GPU 硬件加速实时合成高斯模糊视觉效果。整机冷启动仅需约 100 毫秒，日常运行时内存占用仅 15 MiB 左右，打包体积仅约 6 MiB。

从最早期的远古版本、正式版到最新快照，CairnLauncher 均提供开箱即用的安装、加载器配置、依赖管理与极速启动体验。

---

## 特性一览

### 🚀 安装与启动

- **全版本覆盖**：支持自 Minecraft Alpha 以来所有官方历史版本、快照，以及年份制新版本号。
- **主流加载器无缝支持**：
  - Fabric、Quilt、NeoForge、OptiFine（独立安装或搭配 Forge / Fabric 作为模组加载）。
  - 内置 Forge 安装器处理器执行引擎，直接在启动器内完成新旧版 Forge 的库补全与数据处理，无需唤起官方 Java 安装弹窗。
- **智能内存与 GC 调优**：根据机器物理内存与当前整合包模组规模，自适应推荐最佳堆内存及垃圾收集器参数；也支持按实例单独覆盖。
- **自动文件完整性校验**：启动前静默校验游戏核心、资源文件与依赖库，自动修复损坏或缺失项。
- **内置 Java Agent**：提供离线皮肤与披风加载、Log4j 漏洞修补与中文字符输入法兼容优化。

### ☕ 智能 Java 管理

- **环境自动探测**：自动检索注册表、环境变量及常见安装路径中的 JRE/JDK。
- **版本智能推导**：根据版本 JSON 元数据、发布时间及 Loader 约束自动匹配最佳 Java 版本；本地无匹配环境时，可一键自动下载 Mojang 官方运行时。

### ⚡ 高速多源下载

- **多镜像自适应测速**：支持 Mojang 官方源、BMCLAPI、MCIM 以及自定义镜像，自动并发测速并具备失败无缝灾备切换。
- **高性能传输引擎**：大文件多线程分块下载、自适应限流降速与恢复、断点续传，并在下载流中实时计算 SHA-1，校验无误后原子重命名落地。

### 📦 模组与整合包

- **模组一键检索与更新**：直接在启动器内浏览与搜索 Modrinth 和 CurseForge 模组，自动分析并一键补齐前置依赖（无需 CurseForge API Key，通过 MCIM 镜像透明代理）。
- **中文百科深度集成**：内置 MC 百科（mcmod.cn）本地索引，为外文模组自动展示中文译名与功能概述。
- **多平台整合包导入导出**：支持一键导入 `.mrpack`、CurseForge 整合包及 MultiMC / Prism 实例，并支持导出为 `.mrpack`。
- **资源一键开关**：直观管理当前实例的模组、光影包、材质包与世界存档。

### 🔑 账号管理

- **多认证体系支持**：支持微软官方账号（OAuth 设备码登录）、第三方 Yggdrasil 外置认证（如统一通行证、LittleSkin 或自建认证服务器）及离线账号，支持任意多账号切换。
- **系统级凭据保护**：所有登录凭证使用 Windows 原生 DPAPI 硬件级加密存储在本地。
- **实例账号绑定**：可为特定游戏实例指定偏好账号，启动时自动切换。

### 🔍 智能崩溃诊断

- **直白的原因分析**：游戏发生异常退出时，自动捕获日志、崩溃报告及 JVM `hs_err` 转储，匹配已知崩溃特征库，直接用中文指出崩溃诱因与修复思路。
- **安全报告导出**：一键导出包含运行环境与日志的 Zip 诊断包，自动对账号令牌和系统用户名进行脱敏处理，便于向他人求助。

### 🎨 现代化界面与插件

- **模块化主页看板**：支持自由排布游戏启动卡片、时钟、游玩时长统计与热力图、快捷动作小组件及 YAML 自定义组件。
- **视觉主题定制**：GPU 硬件加速毛玻璃与透明模糊效果、自定义主题强调色、深浅色模式切换、高 DPI 缩放及减弱动效选项。
- **多语言支持**：原生支持简体中文、English 及 日本語。
- **安全沙箱插件系统**：支持通过 JavaScript 编写扩展脚本。插件运行在独立的 QuickJS-NG 子进程中，即便插件出错崩溃也绝不会拖垮启动器主程序。启动时可使用 `--no-plugins` 一键跳过所有插件。插件规范请参考 [docs/plugins](docs/plugins/README.md)。

---

## 快速上手

### 下载与使用

1. 前往项目的 [Releases 页面](https://github.com/RustCraftMC/cairn-launcher/releases) 下载最新的 `CairnLauncher.exe`。
2. CairnLauncher 为**纯便携软件**，直接将可执行文件放置在任意目录运行即可。所有数据与配置均保存在同级目录下的 `.cairn` 文件夹内，随时随地拷入 U 盘即可带走。

### 常用环境变量（可选）

| 变量名                        | 说明                              |
| -------------------------- | ------------------------------- |
| `CAIRN_HOME`               | 自定义数据存放目录（默认位于程序同级 `.cairn`）。   |
| `CAIRN_MS_CLIENT_ID`       | 自定义微软登录 Client ID。              |
| `CAIRN_CURSEFORGE_API_KEY` | 自定义 CurseForge API 密钥。          |
| `CAIRN_UI_SOFTWARE`        | 强制使用纯 CPU 软件渲染（禁用 OpenGL 硬件加速）。 |
| `CAIRN_UI_SCALE_FACTOR`    | 强制指定界面缩放比例（0.5 - 4.0）。          |

---

## 命令行工具 (`cairn`)

CairnLauncher 附带了与图形界面共用核心的轻量 CLI 工具 `cairn`，适合极客、脚本自动化或调试：

```powershell
# 列出可安装的 Minecraft 版本（包含快照）
cairn versions --snapshots

# 查看 1.21.1 可用的 Fabric 加载器版本
cairn loaders fabric 1.21.1

# 一键安装 1.21.1 Fabric 实例并命名为 demo
cairn install 1.21.1 --name demo --fabric

# 查看本地所有实例
cairn instances

# 预检准备并打印启动命令行（不真正拉起游戏）
cairn launch demo --dry-run

# 扫描并列出本机已识别的 Java 环境
cairn java list

# 通过浏览器登录微软正版账号
cairn account microsoft
```

---

## 从源码构建

### 环境要求

- Windows 10 或更高版本
- Rust 1.91 或更高版本（2024 edition）

### 编译与运行

```powershell
# 运行桌面客户端
cargo run -p cairn-app

# 编译桌面客户端 Release 二进制
cargo build --release -p cairn-app
# 输出路径: target\release\CairnLauncher.exe

# 编译命令行工具 Release 二进制
cargo build --release -p cairn-cli
# 输出路径: target\release\cairn.exe

# 运行工作区测试
cargo test --workspace
```

---

## 模块结构

| 目录                    | 说明                                                                     |
| --------------------- | ---------------------------------------------------------------------- |
| `crates/cairn-core`   | 启动器核心后端：下载引擎、加载器安装器、Java 匹配、游戏参数组装、崩溃诊断等。对外暴露纯异步 API 与事件流，不依赖任何 UI 框架。 |
| `crates/cairn-bridge` | 前端状态层：为界面提供响应式数据模型与非阻塞调度桥梁。                                            |
| `crates/cairn-app`    | CairnLauncher 桌面端主程序。                                                  |
| `crates/cairn-cli`    | `cairn` 命令行工具。                                                         |
| `crates/cairn-plugin` | 基于 QuickJS-NG 的插件独立宿主与 IPC 通信管理。                                       |
| `crates/cairn-i18n`   | 国际化支持（`zh-CN`、`en-US`、`ja-JP`）。                                        |
| `crates/cairn-icons`  | 版本与加载器矢量/位图图标库。                                                        |
| `crates/cairn-ui*`    | 自研保留模式图形库（包含 layout 布局、platform 窗口抽象与渲染管线）。                            |
| `docs/plugins`        | 插件开发文档、引擎规范与示例。                                                        |

---

## 鸣谢与生态

- **Plain Craft Launcher 2**：感谢 [Plain Craft Launcher 2](https://github.com/Meloong-Git/PCL) 及其作者 Hex-Dragon 与维护团队。`crates/cairn-icons/assets` 中的部分方块及实体位图图标引自 PCL 资源库，其优秀的设计与交互理念为本项目提供了重要的启发。
- **Hello Minecraft! Launcher**：感谢 [Hello Minecraft! Launcher](https://github.com/HMCL-dev/HMCL) 团队在开源 Minecraft 启动器领域的长期深耕与技术先例。
- **b-launcher**：感谢 [b-launcher](https://github.com/cubk1/b-launcher) 及其作者 cubk1，本项目内置的 `launcher-agent.jar`（用于游戏内离线皮肤与披风加载、输入法兼容优化等）来自 b-launcher 项目。
- **cairn-ui**：本项目界面渲染库衍生自 [xiaojiang233/cairn-ui](https://github.com/xiaojiang233/cairn-ui)，采用 MIT / Apache-2.0 双许可。

---

## 开源许可证 (License)

CairnLauncher 核心遵循 **GNU Lesser General Public License v3.0** ([LGPL-3.0](LICENSE)) 协议开源。

- 底层图形界面库（`crates/cairn-ui*`）源自 [xiaojiang233/cairn-ui](https://github.com/xiaojiang233/cairn-ui)，保持 **MIT** 或 **Apache-2.0** 双许可。
- 内置 Java Agent（`launcher-agent.jar`）由 [b-launcher](https://github.com/cubk1/b-launcher) 提供。

---

## 免责声明

Minecraft 是 Mojang Studios 和 Microsoft 的注册商标。CairnLauncher 为第三方独立开源启动器，与 Mojang Studios 或 Microsoft 无任何隶属、合作或背书关系。
