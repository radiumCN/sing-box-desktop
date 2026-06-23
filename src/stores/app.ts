import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";

export interface SingboxStatus {
  running: boolean;
  uptime?: number;
  pid?: number;
  version?: string;
}

export interface Subscription {
  id: string;
  name: string;
  url: string;
  sub_type: "clash" | "v2ray" | "sip008" | "unknown";
  node_count: number;
  last_update?: string;
  auto_update: boolean;
  update_interval: number;
}

export interface ProxyNode {
  id: string;
  name: string;
  group: string;
  protocol: string;
  server: string;
  port: number;
  latency?: number;        // ms
  download_speed?: number; // KB/s
  is_active: boolean;
  subscription_id?: string;
}

export interface SpeedResult {
  latency_ms?: number;
  download_kbps?: number;
}

export interface AppConfig {
  proxy_mode: "rule" | "global" | "direct" | "tun";
  startup_with_system: boolean;
  startup_minimized: boolean;
  allow_lan: boolean;
  http_port: number;
  socks_port: number;
  mixed_port: number;
  api_port: number;
  tun_enabled: boolean;
  log_level: string;
  theme: string;
  language: string;
  selected_subscription?: string;
  active_nodes: Record<string, string>;
  auto_update_interval: number;
  auto_update_notify: boolean;
  update_channel: string;
  close_to_tray: boolean;
  restore_proxy_on_startup: boolean;
  last_proxy_running: boolean;
  last_system_proxy: boolean;
}

export interface TrafficPoint {
  time: number;
  upload: number;
  download: number;
}

export const useAppStore = defineStore("app", () => {
  const status = ref<SingboxStatus>({ running: false });
  const subscriptions = ref<Subscription[]>([]);
  const nodes = ref<ProxyNode[]>([]);
  const config = ref<AppConfig>({
    proxy_mode: "rule",
    startup_with_system: false,
    startup_minimized: false,
    allow_lan: false,
    http_port: 7890,
    socks_port: 7891,
    mixed_port: 7890,
    api_port: 9090,
    tun_enabled: false,
    log_level: "info",
    theme: "system",
    language: "zh-CN",
    active_nodes: {},
    auto_update_interval: 24,
    auto_update_notify: true,
    update_channel: "stable",
    close_to_tray: true,
    restore_proxy_on_startup: false,
    last_proxy_running: false,
    last_system_proxy: false,
  });
  const trafficHistory = ref<TrafficPoint[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  const activeNode = computed(() => {
    // Prefer the node flagged as active; fall back to matching by saved tag name.
    const byFlag = nodes.value.find((n) => n.is_active);
    if (byFlag) return byFlag;
    const activeTag = config.value.active_nodes["proxy"];
    if (!activeTag) return undefined;
    return nodes.value.find((n) => n.name === activeTag);
  });

  const nodesByGroup = computed(() => {
    const groups: Record<string, ProxyNode[]> = {};
    for (const node of nodes.value) {
      if (!groups[node.group]) groups[node.group] = [];
      groups[node.group].push(node);
    }
    return groups;
  });

  async function fetchStatus() {
    try {
      status.value = await invoke<SingboxStatus>("cmd_get_singbox_status");
    } catch (e) {
      console.error("fetchStatus error:", e);
    }
  }

  function updateTrayTooltip() {
    const modeMap: Record<string, string> = { rule: "规则", global: "全局", direct: "直连", tun: "TUN" };
    const mode = modeMap[config.value.proxy_mode] ?? config.value.proxy_mode;
    const node = activeNode.value?.name ?? "未选择";
    const state = status.value.running ? "● 运行中" : "○ 已停止";
    const tooltip = `sing-box-win\n${state}\n节点: ${node}\n模式: ${mode}`;
    invoke("cmd_update_tray_tooltip", { tooltip }).catch(() => {});
  }

  function syncTrayMenu(sysProxyEnabled: boolean, tunEnabled?: boolean) {
    const tun = tunEnabled ?? config.value.tun_enabled ?? false;
    invoke("cmd_sync_tray_menu", { sysProxyEnabled, tunEnabled: tun }).catch(() => {});
  }

  async function startProxy() {
    loading.value = true;
    error.value = null;
    try {
      await invoke("cmd_start_singbox");
      await fetchStatus();
      updateTrayTooltip();
      // Only enable Windows system proxy when TUN is NOT active.
      // TUN mode captures traffic at the network layer — system proxy is redundant and
      // would cause double-proxying confusion.
      const sysProxyOn = !config.value.tun_enabled;
      if (sysProxyOn) {
        await invoke("cmd_set_system_proxy", { enabled: true }).catch(() => {});
      } else {
        // TUN is on: ensure system proxy is cleared to avoid mixing two proxy methods
        await invoke("cmd_set_system_proxy", { enabled: false }).catch(() => {});
      }
      syncTrayMenu(sysProxyOn);
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function stopProxy() {
    loading.value = true;
    error.value = null;
    try {
      await invoke("cmd_stop_singbox");
      await fetchStatus();
      updateTrayTooltip();
      // Always clear system proxy when stopping sing-box
      await invoke("cmd_set_system_proxy", { enabled: false }).catch(() => {});
      syncTrayMenu(false);
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function fetchSubscriptions() {
    subscriptions.value = await invoke<Subscription[]>("cmd_get_subscriptions");
  }

  async function addSubscription(name: string, url: string) {
    const sub = await invoke<Subscription>("cmd_add_subscription", { name, url });
    subscriptions.value.push(sub);
    await fetchNodes();
    return sub;
  }

  async function updateSubscription(id: string) {
    const sub = await invoke<Subscription>("cmd_update_subscription", { id });
    const idx = subscriptions.value.findIndex((s) => s.id === id);
    if (idx !== -1) subscriptions.value[idx] = sub;
    await fetchNodes();
    return sub;
  }

  async function deleteSubscription(id: string) {
    await invoke("cmd_delete_subscription", { id });
    subscriptions.value = subscriptions.value.filter((s) => s.id !== id);
    nodes.value = nodes.value.filter((n) => n.subscription_id !== id);
  }

  async function saveSubscriptionSettings(id: string, autoUpdate: boolean, updateInterval: number) {
    await invoke("cmd_save_subscription_settings", { id, autoUpdate, updateInterval });
    const idx = subscriptions.value.findIndex((s) => s.id === id);
    if (idx !== -1) {
      subscriptions.value[idx] = {
        ...subscriptions.value[idx],
        auto_update: autoUpdate,
        update_interval: updateInterval,
      };
    }
  }

  async function fetchNodes() {
    nodes.value = await invoke<ProxyNode[]>("cmd_get_nodes");
  }

  async function setActiveNode(nodeId: string) {
    await invoke("cmd_set_active_node", { nodeId });
    await fetchNodes();
    updateTrayTooltip();
  }

  async function testNodeLatency(nodeId: string): Promise<number | null> {
    try {
      const ms = await invoke<number>("cmd_test_node_latency", { nodeId });
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) nodes.value[idx] = { ...nodes.value[idx], latency: ms };
      return ms;
    } catch {
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) nodes.value[idx] = { ...nodes.value[idx], latency: undefined };
      return null;
    }
  }

  async function testNodeSpeed(nodeId: string): Promise<SpeedResult | null> {
    try {
      const result = await invoke<SpeedResult>("cmd_test_node_speed", { nodeId });
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) {
        nodes.value[idx] = {
          ...nodes.value[idx],
          latency: result.latency_ms,
          download_speed: result.download_kbps,
        };
      }
      return result;
    } catch {
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) {
        nodes.value[idx] = { ...nodes.value[idx], latency: undefined, download_speed: undefined };
      }
      return null;
    }
  }

  async function autoSelectNode(): Promise<string | null> {
    try {
      const bestId = await invoke<string>("cmd_auto_select_node");
      // Refresh nodes from backend to get updated latencies + active flag
      await fetchNodes();
      return bestId;
    } catch (e) {
      error.value = String(e);
      return null;
    }
  }

  async function fetchConfig() {
    config.value = await invoke<AppConfig>("cmd_get_app_config");
  }

  // Apply the "launch on system startup" setting to the OS (registry Run key on Windows).
  // Only toggles when the desired state differs from the actual state to avoid redundant work.
  async function syncAutostart(want: boolean) {
    try {
      const current = await isEnabled();
      if (want && !current) {
        await enable();
      } else if (!want && current) {
        await disable();
      }
    } catch (e) {
      console.error("autostart sync failed:", e);
      error.value = `开机自启动设置失败: ${String(e)}`;
    }
  }

  async function saveConfig(newConfig: AppConfig) {
    await invoke("cmd_save_app_config", { newConfig });
    config.value = newConfig;
    // Apply the autostart setting to the OS — the toggle alone only persists a flag.
    await syncAutostart(newConfig.startup_with_system);
    // Keep tray TUN checkbox in sync whenever config changes
    const sysProxy = await invoke<boolean>("cmd_get_system_proxy_status").catch(() => false);
    syncTrayMenu(sysProxy, newConfig.tun_enabled);
  }

  async function setProxyMode(mode: string) {
    await invoke("cmd_set_proxy_mode", { mode });
    config.value.proxy_mode = mode as AppConfig["proxy_mode"];
    // Restart sing-box so the new mode config takes effect immediately
    if (status.value.running) {
      loading.value = true;
      try {
        await invoke("cmd_stop_singbox");
        await invoke("cmd_start_singbox");
        await fetchStatus();
        updateTrayTooltip();
      } catch (e) {
        error.value = String(e);
      } finally {
        loading.value = false;
      }
    }
  }

  function addTrafficPoint(upload: number, download: number) {
    trafficHistory.value.push({ time: Date.now(), upload, download });
    if (trafficHistory.value.length > 60) {
      trafficHistory.value.shift();
    }
  }

  function clearTrafficHistory() {
    trafficHistory.value = [];
  }

  async function init() {
    await Promise.all([
      fetchStatus(),
      fetchSubscriptions(),
      fetchNodes(),
      fetchConfig(),
    ]);
    // Reconcile the autostart toggle with the real OS state so the UI never lies,
    // and re-apply the configured value in case a previous session failed to persist it.
    try {
      const actual = await isEnabled();
      if (actual !== config.value.startup_with_system) {
        await syncAutostart(config.value.startup_with_system);
      }
    } catch (e) {
      console.error("autostart reconcile failed:", e);
    }
    updateTrayTooltip();
  }

  return {
    status,
    subscriptions,
    nodes,
    config,
    trafficHistory,
    loading,
    error,
    activeNode,
    nodesByGroup,
    fetchStatus,
    startProxy,
    stopProxy,
    fetchSubscriptions,
    addSubscription,
    updateSubscription,
    deleteSubscription,
    saveSubscriptionSettings,
    fetchNodes,
    setActiveNode,
    testNodeLatency,
    testNodeSpeed,
    autoSelectNode,
    fetchConfig,
    saveConfig,
    setProxyMode,
    addTrafficPoint,
    clearTrafficHistory,
    updateTrayTooltip,
    init,
  };
});
