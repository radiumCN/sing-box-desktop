# 系统代理 / TUN 模式：开启·关闭·自动恢复 问题分析与修复计划

> 范围：sing-box 持久核心模型下，系统代理与 TUN 模式的开启、关闭、退出清理、开机自启 / 升级后自动恢复的一整条状态链路。
> 目标：让「记住代理状态」在 **正常重启 / 应用内升级 / 开机自启** 三个场景下都真实有效，并消除状态被污染、被误判的隐患。

---

## 1. 现状架构速览（持久核心模型）

- **核心常驻**：启动即拉起 sing-box。无系统代理、无 TUN 时为 *idle core*（仅本地 mixed 入站，对系统网络无副作用），切换代理因此「秒开」。
- **两种出口互斥**：系统代理（写 Windows 注册表 / macOS networksetup / Linux gsettings）与 TUN（接管全部流量）二选一。
- **统一入口**：`apply_connection_mode(mode)`，`mode ∈ {"system","tun","off"}`（`commands.rs:182`）。仪表盘、托盘、快捷键最终都汇聚到它。
- **状态持久化**：`AppConfig` 里三个字段决定恢复行为（`types.rs:120-127`）：
  - `restore_proxy_on_startup`：用户开关「记住代理状态」。
  - `last_proxy_running`：上次是否在代理（后端在开/关时写）。
  - `last_system_proxy`：上次出口是系统代理(true)还是 TUN(false)。
- **启动恢复**：`lib.rs:209-303` 的 setup 异步块，延迟 1s 后判断
  `restore = restore_proxy_on_startup && last_proxy_running`，再据 `last_system_proxy / tun_enabled` 决定恢复 `system` 还是 `tun`。

### 关键路径地图

| 动作 | 入口 | 写 `last_proxy_running` / `last_system_proxy` |
|------|------|------|
| 仪表盘开/关 | 前端 `setConnectionMode` → `cmd_set_connection_mode` → `apply_connection_mode` | 是（`commands.rs:199/237`） |
| 托盘勾选 | `lib.rs:392/406` → `apply_connection_mode` | 是 |
| 快捷键 | 前端 `setConnectionMode` | 是 |
| 停止 | `cmd_stop_singbox` | 置 false（`commands.rs:167`） |
| 退出 / 关窗 | `shutdown_core`（`commands.rs:281`） | **否**（只清 OS 代理，保留 last_*，供下次恢复） |
| 升级拆核 | `shutdown_core_forced`（`commands.rs:297`） | **否** |
| 保存设置 | 前端 `saveConfig` → `cmd_save_app_config`（`commands.rs:1216`） | **整体覆盖 ← 问题根源** |

恢复逻辑本身设计是完整的：正常启动、升级（含 TUN 5 次重试 + `heal_tun_after_upgrade` 自愈）、开机自启都共用同一段代码，没有分叉。**问题不在恢复逻辑，而在于喂给它的状态被污染了。**

---

## 2. 问题清单

### P0 ——「记住代理状态」整体失效（根因）

**`cmd_save_app_config` 用前端快照整体覆盖后端运行时字段。**

```rust
// commands.rs:1216
pub fn cmd_save_app_config(new_config: AppConfig, state: ...) -> Result<(), String> {
    config::save_app_config(&new_config)?;
    *state.app_config.lock().unwrap() = new_config;   // ← 整体替换内存 + 磁盘
    Ok(())
}
```

前端传入的是 `Settings.vue` 进页面时拍的快照（`Settings.vue:113` `localConfig = { ...store.config }`），`watch(localConfig, deep)` 在任意设置项变动 600ms 后触发保存。而 `last_proxy_running / last_system_proxy` 是**后端运行时字段**，前端快照里几乎永远是过期值（常为 `false`）。

**后果链：**
1. 用户进设置勾「记住代理状态」→ 快照里 `last_proxy_running=false`。
2. 保存 → 磁盘变成 `restore_proxy_on_startup=true` 但 `last_proxy_running=false`。
3. 即便先开了代理（后端已写 true），之后再动任何一个设置项又被快照覆盖回 false。
4. 下次启动 `restore = true && false = false` → **不恢复**。

这条链同时解释了用户报告的三个现象：**升级后不恢复、正常重启不恢复、开机自启后不恢复**——同一根因。

**附带：`last_app_version` 被清空。** 前端 `AppConfig` 接口（`app.ts:67-97`）**根本没有** `last_app_version` 字段，整体覆盖时该字段经 `#[serde(default)]` 被重置为 `""`。这会让下次正常重启被误判为 `just_upgraded`（`lib.rs:221`），无谓触发升级自愈与 5 次 TUN 重试（多次断流抖动）。

**第二个污染现场（同根因，阶段一不覆盖）：`apply_config_bundle`。** 配置导入 / 命名 profile 切换走 `apply_config_bundle`（`commands.rs:379-387`），同样 `*state.app_config = cfg` 整体覆盖，`cfg` 来自外部 bundle（导出时机不定，甚至来自别的机器 / 版本）。后果：导入后 `last_proxy_running / last_system_proxy` 被替换成 bundle 里的陈旧值（可能误触发或误抑制恢复），`last_app_version` 被替换成 bundle 的版本（误判 / 漏判 `just_upgraded`）。**因此阶段一只修 `cmd_save_app_config` 并不彻底——必须把同样的「保留后端运行时字段」处理一并应用到 `apply_config_bundle`，或在导出 bundle 时就剥离这三个字段。**

---

### P1 —— 开机自启场景下 TUN 恢复成功率存疑

autostart 注册无特殊参数（`lib.rs:129-132` `Some(vec![])`），应用无法区分「手动启动」与「开机自启」，恢复走同一段代码。问题在于：

- 开机自启比手动启动更早，**系统网络栈 / 路由表可能尚未就绪**。
- 恢复只有固定 `sleep(1s)`，且 **非升级路径下 `tun_attempts = 1`**（`lib.rs:266`），单次失败即降级到 idle core，且**不再重试**。
- 结果：P0 修复后，开机自启 + TUN 仍可能「起来了但没恢复」，用户依旧要手动开。

> 当前被 P0 掩盖（`restore` 根本不触发）。修完 P0 后此问题会浮现。

---

### P2 —— 配置保存是「全量回写」架构，对任何后端运行时字段都不安全

`cmd_save_app_config(new_config: AppConfig)` 接收整份配置并整体替换，是结构性隐患：任何「后端运行时拥有、前端不应改写」的字段（现有 `last_proxy_running / last_system_proxy / last_app_version`，以及未来新增字段）都会被前端的过期快照污染。这是 P0 的更深层成因，单点修 P0 之外应一并加固。

---

### P3 —— 前端 `Settings.vue` 编辑模型基于快照，易与运行时状态漂移

`localConfig` 在挂载时一次性快照，页面打开期间若代理状态经托盘 / 快捷键 / 恢复发生变化，`store.config` 已更新但 `localConfig` 不会跟随；下次 `scheduleSave` 仍以旧快照回写。即使后端按 P0/P2 加固（忽略运行时字段），前端也不应把这些字段纳入保存载荷。

---

### P4 —— 状态一致性的边角

1. **外部改动系统代理不被感知**：用户 / 其它软件 / 重启后未自启等，可能让 OS 实际代理状态与 `last_*` 记录不一致；恢复时可能基于陈旧判断。优先级低。
2. **`apply_connection_mode("system")` 中 `set_system_proxy` 失败**：在写 `last_*` 之前 `return Err`，配置不被更新、前端报错刷新——处理基本正确，仅需确认错误对用户可见。
3. **退出清 OS 代理而保留 `last_*`** 是有意设计（供下次恢复），但与「外部观察到代理被关」存在认知差，需在文档 / 注释中明确，避免被误当 bug「修掉」。

---

## 3. 修复计划

### 阶段一（P0，必须，最小改动即可让功能生效）

**只改 `cmd_save_app_config`：保留后端运行时字段，不让前端快照覆盖。**

```rust
// commands.rs:1216
#[tauri::command]
pub fn cmd_save_app_config(
    new_config: AppConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut guard = state.app_config.lock().unwrap();
    // 后端运行时字段以内存中的真实值为准，忽略前端快照里的陈旧值
    let merged = AppConfig {
        last_proxy_running: guard.last_proxy_running,
        last_system_proxy:  guard.last_system_proxy,
        last_app_version:   guard.last_app_version.clone(),
        ..new_config
    };
    config::save_app_config(&merged).map_err(|e| e.to_string())?;
    *guard = merged;
    Ok(())
}
```

- 风险极低，单点改动。
- 立即让「记住代理状态」在正常重启 / 升级 / 开机自启三场景同时生效。
- 同步消除 `last_app_version` 被清空导致的误判 `just_upgraded`。

**同一阶段必须一并修 `apply_config_bundle`（`commands.rs:379-387`）**，否则配置导入 / profile 切换仍会污染运行时字段。两处可共用一个 helper：

```rust
// 把外部传入的 AppConfig 与后端当前运行时字段合并，运行时字段以后端为准
fn merge_runtime_fields(incoming: AppConfig, current: &AppConfig) -> AppConfig {
    AppConfig {
        last_proxy_running: current.last_proxy_running,
        last_system_proxy:  current.last_system_proxy,
        last_app_version:   current.last_app_version.clone(),
        ..incoming
    }
}
```
`cmd_save_app_config` 与 `apply_config_bundle` 的 `app_config` 分支都改用它。

**验证（手动）：**
1. 开系统代理 → 改任意设置项保存 → 退出 → 重启 → 应自动恢复系统代理。
2. 开 TUN → 同上 → 重启应恢复 TUN。
3. 关代理 → 重启 → 应保持关闭（`last_proxy_running=false` 生效，不误恢复）。
4. 升级一次 → 重启后自动恢复，且日志中 `just_upgraded` 仅在真升级时为真。

### 阶段二（P2/P3，加固，建议同一 PR）

- **后端白名单化**：将「用户可编辑字段」与「后端运行时字段」在类型层面区分。可选两种做法，二选一：
  - 轻量：保留阶段一的合并写法，并在 `AppConfig` 上给 `last_proxy_running / last_system_proxy / last_app_version` 加注释「backend-owned, never written from frontend」，配单元测试锁定行为。
  - 彻底：新增 `EditableConfig` DTO，`cmd_save_app_config` 只接收可编辑字段，后端运行时字段不进入序列化边界。改动较大，可延后。
- **前端不再发送运行时字段**：`Settings.vue` 的保存载荷里剔除 `last_proxy_running / last_system_proxy`（接口层面也移除，避免误用）；或保存前 `localConfig` 先以最新 `store.config` 的运行时字段对齐。

### 阶段三（P1，开机自启 / 冷启动 TUN 恢复鲁棒性）

> 仅在 P0 修复、`restore` 能真正触发后才有意义；建议作为独立 PR，便于单独验证。

候选方案（按性价比排序，可组合）：

1. **恢复 TUN 时统一加重试**：把 `tun_attempts` 的条件从「仅升级」放宽为「凡恢复 TUN」（如非升级也给 2–3 次、间隔 1.5s + `cleanup_stale_tun_adapter`）。代价小，直接提升冷启动成功率。
2. **就绪等待替代固定延迟**：恢复前等待网络 / 路由就绪（如探测默认路由可用、或对 `wait_until_ready` 思路做网络层版本），而非死等 1s。
3. **失败后兜底重试**：恢复降级到 idle core 后，挂一个一次性延迟重试（如 5s 后再试一次），覆盖「启动瞬间网络未就绪」。

**验证：** 配置开机自启 + 记住 TUN，重启物理机 / 虚机，观察开机后是否自动进入 TUN 且有真实流量（非「TUN on, 0 B」黑洞）。

### 阶段四（P4，可观测与认知一致，可选）

- 启动恢复路径补充结构化日志：`restore` 判定值、解析出的 `mode`、每次 attempt 结果、是否走 heal —— 便于线上定位。
- 在 `shutdown_core` 注释中明确「退出清 OS 代理但保留 `last_*` 供恢复」的设计意图，防止后人误改。
- （可选）恢复前用 `get_system_proxy_status()` 对账，处理外部改动导致的陈旧状态。

---

## 4. 优先级与依赖

| 阶段 | 问题 | 必要性 | 依赖 | 状态 |
|------|------|--------|------|----------|
| 一 | P0 | 必须 | 无 | ✅ 已完成（`merge_runtime_fields` 应用于 `cmd_save_app_config` + `apply_config_bundle`） |
| 三 | P1 | 中 | 阶段一 | ✅ 已完成（TUN 恢复重试放宽到凡恢复 TUN：升级 5 次 / 冷启动 3 次） |
| 二 | P2/P3 | 高 | 阶段一 | ✅ 已完成（后端字段加 backend-owned 注释；前端 `app.ts` 剔除运行时字段，不再纳入保存载荷） |
| 四 | P4 | 低 | 无 | ✅ 已完成（升级恢复路径写 update.log 诊断；`shutdown_core` 加设计意图注释） |

> 已落地代码：
> - `src-tauri/src/commands.rs`：`merge_runtime_fields` + 两处调用、`shutdown_core` 注释
> - `src-tauri/src/lib.rs`：TUN 恢复重试条件、升级恢复路径 update.log 诊断日志
> - `src-tauri/src/types.rs`：运行时字段注释
> - `src-tauri/src/updater.rs`：`update_log` 改 `pub(crate)`
> - `src/stores/app.ts`：`AppConfig` 接口与初始化剔除 `last_proxy_running` / `last_system_proxy`
>
> 验证：前端 `vue-tsc --noEmit` 通过（真实类型检查）；后端 `rustfmt` 解析无语法错误。本机 Linux 系统库 `gio-2.0` 2.56 < 2.70 无法完整 `cargo check`（卡在 GTK 的 `gio-sys`，与改动无关），需在 Windows/CI 上做真实编译与手动回归。

### 手动回归清单（Windows）
1. 开系统代理 → 改任意设置项 → 退出 → 重启：应自动恢复系统代理。
2. 开 TUN → 同上 → 重启：应恢复 TUN 且有真实流量。
3. 关代理 → 重启：应保持关闭（不误恢复）。
4. 导入一份「代理开启时导出」的配置：当前运行时状态不应被 bundle 覆盖。
5. 应用内升级一次：重启后自动恢复；查看 `logs/update.log` 应有 `startup restore (post-upgrade): ...` 记录。
6. 开机自启 + 记住 TUN，重启物理机/虚机：开机后应自动进入 TUN（冷启动 3 次重试兜底）。

**最小可发布**：仅阶段一即可消除用户报告的「升级后 / 重启后 / 开机自启后代理不恢复、记住代理状态像没用」。阶段三决定开机自启 TUN 的恢复成功率，建议紧随其后单独验证。

---

## 5. 涉及文件索引

- `src-tauri/src/commands.rs`
  - `cmd_save_app_config` `:1216`（阶段一核心）
  - `apply_connection_mode` `:182`、`cmd_stop_singbox` `:156`、`shutdown_core` `:281`、`shutdown_core_forced` `:297`、`heal_tun_after_upgrade` `:256`
- `src-tauri/src/lib.rs`
  - 启动恢复块 `:209-303`、`just_upgraded`/`tun_attempts` `:221/266`、autostart 注册 `:129-132`、退出/关窗 `:148-171`、托盘事件 `:384-435`
- `src-tauri/src/types.rs`
  - `AppConfig` 字段定义 `:97-167`、`Default` `:202-236`
- `src-tauri/src/proxy.rs`
  - `set_system_proxy` / `get_system_proxy_status`（各平台）
- `src-tauri/src/singbox.rs`
  - `wait_until_ready` `:177`、`stop_singbox` 优雅/强杀 `:343`
- `src/stores/app.ts`
  - `saveConfig` `:699`、`setConnectionMode` `:284`、`fetchConfig` `:679`、`applyGlobalShortcuts` `:313`、`AppConfig` 接口 `:67-97`
- `src/views/Settings.vue`
  - `localConfig` 快照 `:113`、`scheduleSave` `:122`、「记住代理状态」绑定 `:775-782`

---

## 6. 补充修复（v0.3.11 实测中发现）

**现象**：升级到 v0.3.11 后，重启应用，TUN 被自动恢复（Stage 1 修复生效），但
仪表盘显示「TUN 模式 / 已连接」却 0 连接、0 流量；手动把 TUN 关掉再打开即恢复正常。

**根因**：新建/恢复的 TUN 隧道可能叠在残留路由状态上而黑洞掉全部流量（TUN 显示 on，
0 连接 0 B），唯一解药是 off→on settle。代码里 `heal_tun_after_upgrade` 就是自动化这个
off→on，但它**只在 `just_upgraded` 那一次启动才跑**。普通重启 / 开机自启恢复 TUN 时
`just_upgraded=false`，于是恢复成功却不 settle → 黑洞，需用户手动 off→on。
即：Stage 1 把「恢复 TUN」修好了，却没把「恢复后必须 off→on settle」一并带出升级专属路径。

> 这个黑洞**不是升级特有**的——它源于路由叠加，普通重启同样会触发。

**修复**（lib.rs）：把启动恢复路径里的 off→on 自愈条件从 `mode == "tun" && just_upgraded`
放宽为 `mode == "tun"`——**凡启动时恢复 TUN 都做一次 off→on settle**。这是一次性的启动
冷路径（约 1–2 秒 settle），不影响手动切换延迟。重试次数仍保留升级 5 次 / 冷启动 3 次。
`heal_tun_after_upgrade` 文档同步澄清其已用于所有恢复场景（函数名保留以减少改动面）。
升级恢复诊断日志的「post-upgrade」措辞改为通用「startup restore」并附 `just_upgraded` 标志。

**验证**：需在 Windows 重新构建（建议 bump 到 v0.3.12）后回归：开 TUN + 记住 → 重启应用
→ 应自动恢复 TUN **且直接有流量**，无需手动 off→on。

---

## 7. 回归修复(v0.3.12 实测:自动 heal 导致升级后闪退)

**现象**(update.log）：
```
startup restore: mode=tun, just_upgraded=true ...
startup restore: TUN restored (just_upgraded=true), running off→on heal
   ← 应用闪退；用户手动重启后可正常手动开启 TUN
```
升级到 0.3.12 重启后，TUN 被自动恢复并触发 §6 新加的 off→on heal，应用**当场闪退**。

**根因**：heal 的 graceful "off" 走 `stop_singbox(graceful)` → `send_ctrl_c`
（`singbox.rs:40`），它 `AttachConsole(core_pid)` 后向**整个控制台进程组**广播
`CTRL_C_EVENT`。这套进程级 console 操作从**启动后台 spawn 任务**里调用时会把 GUI 自身一起
杀掉——正是当年 updater 放弃 graceful、改用强杀 `shutdown_core_forced` 的同一个自杀坑
（updater.rs 注释有记录）。手动 off→on 不崩，是因为它跑在 webview 命令线程；heal 跑在启动
后台任务，上下文与 updater 崩溃点相同。

> 关键：§6 的 heal **在 `just_upgraded` 分支本来就会崩**。只是此前 P0 bug 让 restore 从不
> 触发、heal 从没真正跑过；P0 修好后才把这个潜伏崩溃暴露出来。

**修复**（lib.rs）：**移除启动路径的自动 off→on heal**，保留 restore + 重试。修复后：
- 正常重启：干净退出时 `shutdown_core` 已做 graceful 路由清理 → 恢复的 TUN 直接可用，本不需 heal；
- 升级重启（强杀退出留残留路由）：恢复的 TUN 可能黑洞 → 用户手动 off→on（命令线程，安全）即可。

即从「崩溃」回到「已知安全行为」，严格改善。`heal_tun_after_upgrade` 函数保留（加
`#[allow(dead_code)]` + 文档警告「只能从命令/UI 线程调用」），作为将来**前端触发的「重连/修复」
按钮**的构件——那是唯一能安全自动化 off→on 的上下文。

**后续（需 Windows 实测，未做）**：把 off→on 自愈做成前端触发的命令（与手动 off→on 同一安全
上下文），既能一键修复升级后的 TUN 黑洞，又不碰 GUI 自杀坑。
