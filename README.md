<p align="center">
  <img src="assets/logo.png" width="96" alt="CairnLauncher logo">
</p>

<h1 align="center">CairnLauncher</h1>

<p align="center">
  A lightweight, high-performance Minecraft launcher for Windows, built in Rust.<br>
  English | <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows" alt="Platform">
  <img src="https://img.shields.io/badge/Rust-2024_Edition-black?logo=rust" alt="Rust 2024">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-LGPL_v3-blue.svg" alt="License: LGPL v3"></a>
  <img src="https://img.shields.io/badge/UI-Pure_Native_(No_Electron)-blueviolet" alt="Pure Native UI">
</p>

---

**CairnLauncher** is an open-source Minecraft launcher designed for speed, low resource usage, and a clean desktop experience.

Instead of bundling a heavy browser engine like Electron or WebView, CairnLauncher is powered by `cairn-ui`—a lightweight retained-mode GUI engine built directly in Rust. It utilizes GPU hardware acceleration for real-time background blur and glass effects (Mica / Acrylic style). Cold startup takes around 100 ms, idle memory consumption stays near 15 MiB, and the release executable is roughly 6 MiB.

From classic alpha versions to the latest snapshots, CairnLauncher handles version installs, mod loader bootstrapping, dependency resolution, and runtime management out of the box.

---

## Features

### 🚀 Installation & Launching
- **Full Version Support**: Supports every official Minecraft release from early Alpha to the latest snapshots and new year-based versions.
- **Mod Loaders**: First-class support for Fabric, Quilt, NeoForge, and OptiFine (standalone or as a companion mod). Forge installer data and processors are executed natively inside the launcher, eliminating the need to pop up official Java installer GUIs.
- **Smart Memory & GC Tuning**: Automatically calculates optimal heap allocation and garbage collection flags based on installed mod count and system RAM, with per-instance override support.
- **Integrity Verification**: Validates libraries, assets, and game jars prior to launch, automatically redownloading missing or corrupted files.
- **Bundled Java Agent**: Includes custom offline skins and cape patches, Log4j mitigations, and Windows IME input fixes.

### ☕ Intelligent Java Management
- **Automatic Runtime Discovery**: Scans registry entries, path variables, and standard install locations for installed JREs/JDKs.
- **Compatibility Inference**: Selects the appropriate Java version using release metadata, publication date, and loader constraints. If no compatible JRE is detected, it automatically downloads official Mojang runtimes.

### ⚡ Resilient Multi-Source Downloads
- **Mirror Failover**: Out-of-the-box support for Mojang official servers, BMCLAPI, MCIM, and custom mirror endpoints with concurrent latency testing and seamless failover.
- **Transfer Engine**: Multi-stream chunking for large files, adaptive bandwidth throttling, resume support, and on-the-fly SHA-1 hashing with atomic file replacement.

### 📦 Mods & Modpacks
- **Mod Browsing & Updates**: Browse, search, and download mods directly from Modrinth and CurseForge (via MCIM mirror, requiring no separate API key), with automatic dependency resolution.
- **MCmod.cn Chinese Wiki Integration**: Translates mod titles and retrieves localized overviews automatically.
- **Pack Import & Export**: Import `.mrpack`, CurseForge zip archives, and MultiMC / Prism instance formats; export instances as `.mrpack`.
- **Resource Management**: Toggle and configure mods, resource packs, shaders, and world saves on a per-instance basis.

### 🔑 Account Management
- **Multiple Providers**: Sign in via Microsoft accounts (OAuth device-code flow), third-party Yggdrasil authentication servers (LittleSkin or self-hosted), or offline profiles.
- **Secure Storage**: Tokens and authentication state are encrypted locally via Windows DPAPI.
- **Instance Binding**: Pin specific accounts to specific game instances for automatic switching.

### 🔍 Plain-Text Crash Diagnostics
- **Automated Analysis**: Automatically analyzes game logs, crash reports, and JVM `hs_err` core dumps when a game fails to start or exits abnormally, highlighting root causes based on a curated pattern database.
- **Safe Export**: Export diagnostic bundles as a single zip archive with usernames and auth tokens automatically redacted.

### 🎨 Clean Desktop Interface & Extensions
- **Customizable Dashboard**: Flexible home board featuring launch tiles, playtime heatmaps, quick shortcuts, and YAML-defined widgets.
- **Native Styling**: GPU-accelerated blur, custom accent colors, dark/light modes, HiDPI scaling, and reduced motion toggles.
- **Multi-language**: Built-in Simplified Chinese, English, and Japanese translations.
- **Isolated Plugin System**: Extend functionality using JavaScript plugins powered by QuickJS-NG. Plugins execute in an isolated worker process so errors won't bring down the launcher. Launch with `--no-plugins` to bypass extensions entirely. See [docs/plugins](docs/plugins/README.md) for plugin specs and API details.

---

## Getting Started

### Download
1. Grab the latest release executable from the [Releases page](https://github.com/RustCraftMC/cairn-launcher/releases).
2. CairnLauncher is **fully portable**: all instances, profiles, and runtime settings reside in a local `.cairn` directory created next to the executable. No registry clutter, easy to carry on a USB drive.

### Optional Environment Variables

| Variable | Description |
| --- | --- |
| `CAIRN_HOME` | Custom data directory (defaults to `.cairn` next to the executable). |
| `CAIRN_MS_CLIENT_ID` | Custom Microsoft OAuth application Client ID. |
| `CAIRN_CURSEFORGE_API_KEY` | Custom CurseForge API key. |
| `CAIRN_UI_SOFTWARE` | Force CPU software rendering (disables OpenGL presentation). |
| `CAIRN_UI_SCALE_FACTOR` | Override system UI scaling factor (0.5 to 4.0). |

---

## Command-Line Interface (`cairn`)

CairnLauncher includes a standalone CLI binary sharing the exact same core backend, ideal for headless environments, scripts, and automation:

```powershell
# List available Minecraft versions (including snapshots)
cairn versions --snapshots

# Query compatible Fabric versions for Minecraft 1.21.1
cairn loaders fabric 1.21.1

# Install Minecraft 1.21.1 with Fabric loader named "demo"
cairn install 1.21.1 --name demo --fabric

# List local instances
cairn instances

# Dry-run launch pipeline and print the full launch command
cairn launch demo --dry-run

# List detected local Java runtimes
cairn java list

# Authenticate a Microsoft account via the system browser
cairn account microsoft
```

---

## Building from Source

### Prerequisites
- Windows 10 or newer
- Rust toolchain 1.91 or newer (2024 edition)

### Build Commands
```powershell
# Run desktop client
cargo run -p cairn-app

# Build optimized desktop executable
cargo build --release -p cairn-app
# Output binary: target\release\CairnLauncher.exe

# Build CLI tool
cargo build --release -p cairn-cli
# Output binary: target\release\cairn.exe

# Run workspace tests
cargo test --workspace
```

---

## Architecture

| Crate | Role |
| --- | --- |
| `crates/cairn-core` | Core backend engine: download managers, installer runners, Java probes, launch arguments, and crash analyzers. Exposes async APIs and event streams with zero UI dependencies. |
| `crates/cairn-bridge` | Frontend state models and non-blocking operation channels connecting core to the UI. |
| `crates/cairn-app` | Desktop GUI application (`CairnLauncher`). |
| `crates/cairn-cli` | Command-line tool (`cairn`). |
| `crates/cairn-plugin` | QuickJS-NG supervisor, IPC protocol, and plugin runtime. |
| `crates/cairn-i18n` | Internationalization (`zh-CN`, `en-US`, `ja-JP`). |
| `crates/cairn-icons` | Vector and bitmap icon assets. |
| `crates/cairn-ui*` | Retained-mode GUI framework: layout engine, platform abstraction, and rendering pipeline. |
| `docs/plugins` | Plugin documentation, specification, and example scripts. |

---

## Acknowledgements

- **Plain Craft Launcher 2**: Special thanks to [Plain Craft Launcher 2](https://github.com/Meloong-Git/PCL), its author Hex-Dragon, and its maintainers. Select block and entity icon assets in `crates/cairn-icons/assets` originate from PCL resources; its exceptional UX design has been a valuable inspiration for this project.
- **Hello Minecraft! Launcher**: Special thanks to the [Hello Minecraft! Launcher](https://github.com/HMCL-dev/HMCL) team for their long-standing contributions to the open-source Minecraft launcher community.
- **b-launcher**: Special thanks to [b-launcher](https://github.com/cubk1/b-launcher) and its author cubk1 for providing `launcher-agent.jar`, used by CairnLauncher for in-game offline skin/cape rendering and IME compatibility patches.
- **cairn-ui**: GUI engine adapted from [xiaojiang233/cairn-ui](https://github.com/xiaojiang233/cairn-ui), dual-licensed under MIT or Apache-2.0.

---

## License

CairnLauncher is licensed under the **GNU Lesser General Public License v3.0** ([LGPL-3.0](LICENSE)).

- The sub-crates powering the GUI engine (`crates/cairn-ui*`) are adapted from [xiaojiang233/cairn-ui](https://github.com/xiaojiang233/cairn-ui) and remain dual-licensed under **MIT** or **Apache-2.0**.
- The bundled Java agent (`launcher-agent.jar`) is provided by [b-launcher](https://github.com/cubk1/b-launcher).

---

## Disclaimer

Minecraft is a trademark of Mojang Studios and Microsoft. CairnLauncher is an independent open-source launcher and is not affiliated with, endorsed by, or associated with Mojang Studios or Microsoft.
