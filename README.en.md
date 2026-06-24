# sing-box-desktop

A cross-platform (Windows / macOS) GUI proxy client powered by the [sing-box](https://github.com/SagerNet/sing-box) core, built with Tauri v2 + Vue 3, featuring a native title bar and a frosted-glass UI.

> [中文](./README.md) | English

## Features

- ✅ **Subscription management** — Clash YAML / V2Ray Base64 / SIP008 / single-node links, with auto-update
- ✅ **Node management** — grouped display, one-click latency / speed test, sort by latency
- ✅ **Dynamic auto-select** — continuously picks the fastest node via URLTest, with "all nodes" and "per-subscription" groups; test URL / interval / tolerance are configurable in settings; the dashboard and tray show the concrete node currently in use
- ✅ **Minimal proxy control** — the dashboard exposes only two mutually-exclusive switches: **System Proxy** and **TUN Mode**; turning either on starts the proxy. Rule / Global / Direct modes supported
- ✅ **Realtime dashboard** — up/down speed chart, uptime, memory usage
- ✅ **Traffic statistics** — cumulative upload / download since the service started (sourced from the core, so it persists across page navigation and resets automatically on restart)
- ✅ **Connections viewer** — live active-connections list with rules, proxy chains, and up/down bytes (Clash API compatible)
- ✅ **Log viewer** — realtime sing-box logs with level filtering
- ✅ **Routing rule editor** — visually edit routing rules (domain / GeoSite / GeoIP / IP / port / process)
- ✅ **System tray** — close-to-tray, quick menu (mutually-exclusive System Proxy / TUN switches, status, restore main window)
- ✅ **TUN mode** — system-wide traffic capture; on Windows includes a UAC elevation prompt and automatic WinTun driver download
- ✅ **Auto-start / restore on launch** — launch on login and optionally restore the previous proxy state on next start
- ✅ **Core updates** — check / one-click download of the sing-box core with progress; periodic checks after launch with a sidebar red-dot indicator
- ✅ **Theme** — follow system / light / dark

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop framework | Tauri v2 (Rust) |
| Frontend | Vue 3 + TypeScript |
| Build | Vite 6 + Tailwind CSS v4 |
| Routing | Vue Router 4 |
| State | Pinia |
| Charts | Chart.js + vue-chartjs |
| UI | Native title bar + Fluent / frosted-glass style, light & dark themes |

## Requirements

- Node.js >= 18
- Rust >= 1.88 (installed via rustup)
- OS: Windows 10/11 x64 or macOS 11+
- [sing-box binary](https://github.com/SagerNet/sing-box/releases) — place it under `src-tauri/binaries/`:
  - Windows: `src-tauri/binaries/sing-box.exe`
  - macOS / Linux: `src-tauri/binaries/sing-box`
  - Note: a core downloaded via the in-app "Core update" feature is stored in the user data directory and takes priority over the bundled one.

## Quick Start

```bash
# 1. Clone the repository
git clone https://github.com/radiumCN/sing-box-desktop.git
cd sing-box-desktop

# 2. Install frontend dependencies
npm install

# 3. Download the sing-box binary
# Get the build for your platform from
# https://github.com/SagerNet/sing-box/releases
# Windows -> src-tauri/binaries/sing-box.exe
# macOS   -> src-tauri/binaries/sing-box

# 4. Run in development mode
npm run tauri dev

# 5. Build a release
npm run tauri build
```

## Project Structure

```
sing-box-desktop/
├── src/                      # Vue 3 frontend
│   ├── App.vue               # Root component (sidebar + content layout)
│   ├── main.ts               # Entry point
│   ├── router/index.ts       # Routes
│   ├── stores/app.ts         # Pinia store (global state / traffic monitor poller)
│   ├── styles/main.css       # Global styles (Tailwind + custom variables)
│   ├── components/
│   │   └── Sidebar.vue       # Side navigation + status footer
│   └── views/
│       ├── Home.vue          # Dashboard (switches, mode, traffic, chart)
│       ├── Subscriptions.vue # Subscription management
│       ├── Nodes.vue         # Node list / auto-select
│       ├── Connections.vue   # Active connections
│       ├── Logs.vue          # Runtime logs
│       ├── Rules.vue         # Routing rule editor
│       └── Settings.vue      # Settings
├── src-tauri/                # Tauri Rust backend
│   └── src/
│       ├── lib.rs            # App entry (tray, window events, command registry)
│       ├── commands.rs       # IPC commands (frontend ↔ backend)
│       ├── singbox.rs        # sing-box process lifecycle
│       ├── subscription.rs   # Subscription parsing & sing-box config generation
│       ├── config.rs         # Config persistence
│       ├── proxy.rs          # System proxy (Windows registry, etc.)
│       ├── tun.rs            # TUN mode / WinTun driver handling
│       ├── rules.rs          # Routing rule model
│       ├── updater.rs        # sing-box core download / update
│       ├── auto_update.rs    # Periodic update checks after launch
│       └── types.rs          # Shared type definitions
└── package.json
```

## Supported Subscription Formats

| Format | Description |
|--------|-------------|
| Clash YAML | YAML with a `proxies:` field; supports ss/vmess/vless/trojan/hysteria2 |
| V2Ray Base64 | Base64-encoded list of node links |
| Single-node link | `vmess://` `vless://` `ss://` `trojan://` `hysteria2://` `hy2://` |
| SIP008 | Shadowsocks standard JSON subscription format |

## License

MIT
