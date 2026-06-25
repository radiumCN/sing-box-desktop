# Skylark（云雀）

基于 [sing-box](https://github.com/SagerNet/sing-box) 内核的跨平台（Windows / macOS）图形化代理客户端，使用 Tauri v2 + Vue 3 构建，原生标题栏、毛玻璃质感界面。

> 中文 | [English](./README.en.md)

## 功能特性

- ✅ **订阅管理** — 支持 Clash YAML / V2Ray Base64 / SIP008 / 单节点链接，支持自动更新
- ✅ **节点管理** — 分组展示、一键测延迟 / 测速、延迟排序
- ✅ **自动选优（动态）** — 基于 URLTest 持续选择最快节点，支持「全部节点」与「按订阅」分组；测试地址 / 间隔 / 容差可在设置中配置；仪表盘与托盘实时显示当前命中的真实节点
- ✅ **极简代理控制** — 仪表盘仅两个互斥开关：**系统代理** 与 **TUN 模式**，开启任一即启动代理；支持 规则 / 全局 / 直连 模式切换
- ✅ **实时仪表盘** — 上 / 下行速率曲线、运行时长、内存占用
- ✅ **流量统计** — 统计「启动服务后」的累计上传 / 下载流量（数据源自内核，切换页面不丢失，重启代理自动清零）
- ✅ **连接查看** — 实时活动连接列表，含规则、代理链、上下行（Clash API 兼容）
- ✅ **日志查看** — sing-box 实时日志，支持级别过滤
- ✅ **分流规则编辑器** — 可视化编辑路由规则（域名 / GeoSite / GeoIP / IP / 端口 / 进程）
- ✅ **系统托盘** — 关闭到托盘、托盘快捷菜单（系统代理 / TUN 互斥开关、状态显示、快速唤起主界面）
- ✅ **TUN 模式** — 全局流量接管；Windows 含 UAC 提权请求 + WinTun 驱动自动下载
- ✅ **开机自启 / 启动恢复** — 登录自动启动，并可在下次启动时恢复上次代理状态
- ✅ **内核更新** — 检查 / 一键下载更新 sing-box 内核，带进度显示；启动后定期检查，侧边栏红点提示
- ✅ **主题切换** — 跟随系统 / 浅色 / 深色

## 技术栈

| 层次 | 技术 |
|------|------|
| 桌面框架 | Tauri v2 (Rust) |
| 前端 | Vue 3 + TypeScript |
| 构建 | Vite 6 + Tailwind CSS v4 |
| 路由 | Vue Router 4 |
| 状态 | Pinia |
| 图表 | Chart.js + vue-chartjs |
| 界面 | 系统原生标题栏 + Fluent / 毛玻璃风格，支持深浅主题 |

## 开发环境要求

- Node.js >= 18
- Rust >= 1.88（通过 rustup 安装）
- 操作系统：Windows 10/11 x64 或 macOS 11+
- [sing-box 二进制文件](https://github.com/SagerNet/sing-box/releases) — 放置到 `src-tauri/binaries/`：
  - Windows：`src-tauri/binaries/sing-box.exe`
  - macOS / Linux：`src-tauri/binaries/sing-box`
  - 提示：应用内「内核更新」下载的二进制会保存到用户数据目录，并优先于打包内核被使用。

## 快速开始

```bash
# 1. 克隆仓库
git clone https://github.com/radiumCN/skylark.git
cd skylark

# 2. 安装前端依赖
npm install

# 3. 下载 sing-box 二进制文件
# 从 https://github.com/SagerNet/sing-box/releases 下载对应平台版本
# Windows 放置到 src-tauri/binaries/sing-box.exe
# macOS  放置到 src-tauri/binaries/sing-box

# 4. 开发模式启动
npm run tauri dev

# 5. 构建发布版本
npm run tauri build
```

## 项目结构

```
skylark/
├── src/                      # Vue 3 前端
│   ├── App.vue               # 根组件（侧栏 + 内容布局）
│   ├── main.ts               # 入口
│   ├── router/index.ts       # 路由配置
│   ├── stores/app.ts         # Pinia 状态管理（含全局状态 / 流量监控轮询）
│   ├── styles/main.css       # 全局样式（Tailwind + 自定义变量）
│   ├── components/
│   │   └── Sidebar.vue       # 侧边导航 + 底部状态
│   └── views/
│       ├── Home.vue          # 仪表盘（开关、模式、流量、图表）
│       ├── Subscriptions.vue # 订阅管理
│       ├── Nodes.vue         # 节点列表 / 自动选优
│       ├── Connections.vue   # 活动连接
│       ├── Logs.vue          # 运行日志
│       ├── Rules.vue         # 分流规则编辑器
│       └── Settings.vue      # 设置
├── src-tauri/                # Tauri Rust 后端
│   └── src/
│       ├── lib.rs            # Tauri 应用入口（托盘、窗口事件、命令注册）
│       ├── commands.rs       # IPC 命令（前后端通信接口）
│       ├── singbox.rs        # sing-box 进程生命周期管理
│       ├── subscription.rs   # 订阅解析与 sing-box 配置生成
│       ├── config.rs         # 配置持久化
│       ├── proxy.rs          # 系统代理设置（Windows 注册表等）
│       ├── tun.rs            # TUN 模式 / WinTun 驱动处理
│       ├── rules.rs          # 分流规则模型
│       ├── updater.rs        # sing-box 内核下载 / 更新
│       ├── auto_update.rs    # 启动后定期检查更新
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
