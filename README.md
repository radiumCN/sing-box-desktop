# sing-box-win

基于 [sing-box](https://github.com/SagerNet/sing-box) 的 Windows 64 位图形化管理工具。

## 功能特性

- ✅ **订阅管理** — 支持 Clash YAML / V2Ray Base64 / SIP008 格式订阅链接
- ✅ **节点管理** — 分组展示、一键测速、延迟排序
- ✅ **代理控制** — 系统代理自动设置、规则/全局/直连/TUN 模式切换
- ✅ **实时仪表盘** — 上/下行速率图表、连接数、运行时长
- ✅ **连接查看** — 实时活动连接列表（Clash API 兼容）
- ✅ **日志查看** — sing-box 实时日志，支持级别过滤
- ✅ **系统托盘** — 最小化到托盘，托盘快捷菜单
- ✅ **开机自启** — Windows 登录自动启动
- ✅ **TUN 模式** — 全局流量接管，含 UAC 提权请求 + WinTun 驱动自动下载
- ✅ **分流规则编辑器** — 可视化编辑路由规则（域名/GeoSite/GeoIP/IP/端口/进程）
- ✅ **内核更新** — 检查最新版本、一键下载更新、进度显示
- ✅ **自动更新检查** — 启动后定期检查新版本，侧边栏红点提示

## 技术栈

| 层次 | 技术 |
|------|------|
| 桌面框架 | Tauri v2 (Rust) |
| 前端 | Vue 3 + TypeScript |
| 构建 | Vite 6 + Tailwind CSS v4 |
| 路由 | Vue Router 4 |
| 状态 | Pinia |
| UI 风格 | Windows 11 Fluent Design（毛玻璃/Mica 效果）|
| 图表 | Chart.js + vue-chartjs |

## 开发环境要求

- Node.js >= 18
- Rust >= 1.88 (通过 rustup 安装)
- Windows 10/11 x64
- [sing-box 二进制文件](https://github.com/SagerNet/sing-box/releases) — 放置在 `src-tauri/binaries/sing-box.exe`

## 快速开始

```bash
# 1. 克隆仓库
git clone https://gitee.com/luoleitest/sing-box-win.git
cd sing-box-win

# 2. 安装前端依赖
npm install

# 3. 下载 sing-box 二进制文件
# 从 https://github.com/SagerNet/sing-box/releases 下载 Windows x64 版本
# 放置到 src-tauri/binaries/sing-box.exe

# 4. 开发模式启动
npm run tauri dev

# 5. 构建发布版本
npm run tauri build
```

## 项目结构

```
sing-box-win/
├── src/                      # Vue 3 前端
│   ├── App.vue               # 根组件（布局）
│   ├── main.ts               # 入口
│   ├── router/index.ts       # 路由配置
│   ├── stores/app.ts         # Pinia 状态管理
│   ├── styles/main.css       # 全局样式（Tailwind + 自定义变量）
│   ├── components/
│   │   ├── Titlebar.vue      # 自定义标题栏
│   │   └── Sidebar.vue       # 侧边导航
│   └── views/
│       ├── Home.vue          # 仪表盘
│       ├── Subscriptions.vue # 订阅管理
│       ├── Nodes.vue         # 节点列表
│       ├── Connections.vue   # 活动连接
│       ├── Logs.vue          # 运行日志
│       └── Settings.vue      # 设置
├── src-tauri/                # Tauri Rust 后端
│   └── src/
│       ├── lib.rs            # Tauri 应用入口
│       ├── commands.rs       # IPC 命令（前后端通信接口）
│       ├── singbox.rs        # sing-box 进程生命周期管理
│       ├── subscription.rs   # 订阅解析引擎
│       ├── config.rs         # 配置持久化
│       ├── proxy.rs          # Windows 系统代理（注册表）
│       └── types.rs          # 共享类型定义
└── package.json
```

## 支持的订阅格式

| 格式 | 描述 |
|------|------|
| Clash YAML | 包含 `proxies:` 字段的 YAML 文件，支持 ss/vmess/vless/trojan/hysteria2 |
| V2Ray Base64 | Base64 编码的节点链接列表 |
| 单节点链接 | `vmess://` `vless://` `ss://` `trojan://` `hysteria2://` `hy2://` |
| SIP008 | Shadowsocks 标准 JSON 订阅格式 |

## License

MIT
