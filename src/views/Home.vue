<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from "vue";
import {
  Play, Square, Wifi, WifiOff, ArrowUp, ArrowDown,
  Filter, Zap, Server, Clock, Globe, Shield
} from "@lucide/vue";
import { Line } from "vue-chartjs";
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Filler,
  Tooltip,
} from "chart.js";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../stores/app";

ChartJS.register(CategoryScale, LinearScale, PointElement, LineElement, Filler, Tooltip);

const store = useAppStore();
const systemProxyEnabled = ref(false);
const systemProxyReady = ref(false);

async function fetchSystemProxy() {
  systemProxyEnabled.value = await invoke<boolean>("cmd_get_system_proxy_status");
}

async function toggleSystemProxy() {
  const next = !systemProxyEnabled.value;
  try {
    if (next && !store.status.running) {
      // Start sing-box first when user enables system proxy while it's stopped.
      // startProxy() also calls cmd_set_system_proxy internally so we're done after this.
      await store.startProxy();
      await fetchSystemProxy();
      return;
    }
    if (!next && store.status.running && !store.config.tun_enabled) {
      // When disabling system proxy and TUN is also off, stop sing-box entirely
      // so we don't leave an orphaned listener on the mixed port.
      await store.stopProxy();
      await fetchSystemProxy();
      return;
    }
    // sing-box already in the desired state — just flip the system proxy flag
    await invoke("cmd_set_system_proxy", { enabled: next });
    systemProxyEnabled.value = next;
    invoke("cmd_sync_tray_menu", { sysProxyEnabled: next, tunEnabled: store.config.tun_enabled ?? false }).catch(() => {});
  } catch (e) {
    store.error = String(e);
  }
}
const uploadSpeed = ref(0);
const downloadSpeed = ref(0);
const totalUpload = ref(0);
const totalDownload = ref(0);
const memoryUsage = ref<number | null>(null);
let pollTimer: ReturnType<typeof setInterval> | null = null;

const chartLabels = computed(() =>
  store.trafficHistory.map((_, i) => (i === store.trafficHistory.length - 1 ? "now" : ""))
);

const chartData = computed(() => ({
  labels: chartLabels.value,
  datasets: [
    {
      label: "上传",
      data: store.trafficHistory.map((p) => p.upload / 1024),
      borderColor: "#0078d4",
      backgroundColor: "rgba(0,120,212,0.08)",
      borderWidth: 1.5,
      fill: true,
      tension: 0.4,
      pointRadius: 0,
    },
    {
      label: "下载",
      data: store.trafficHistory.map((p) => p.download / 1024),
      borderColor: "#107c10",
      backgroundColor: "rgba(16,124,16,0.08)",
      borderWidth: 1.5,
      fill: true,
      tension: 0.4,
      pointRadius: 0,
    },
  ],
}));

const chartOptions = {
  responsive: true,
  maintainAspectRatio: false,
  animation: { duration: 0 },
  scales: {
    x: { display: false },
    y: {
      display: true,
      grid: { color: "rgba(128,128,128,0.08)" },
      ticks: {
        color: "var(--color-text-muted)",
        font: { size: 10 },
        callback: (v: string | number) => `${Number(v).toFixed(0)} KB/s`,
      },
    },
  },
  plugins: { legend: { display: false }, tooltip: { mode: "index" as const, intersect: false } },
};

const formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
};

const formatUptime = (sec?: number) => {
  if (!sec) return "--";
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
};

const proxyModeLabel = computed(() => {
  const map: Record<string, string> = {
    rule: "规则模式",
    global: "全局代理",
    direct: "直连模式",
    tun: "TUN 模式",
  };
  return map[store.config.proxy_mode] ?? store.config.proxy_mode;
});

async function toggleProxy() {
  if (store.status.running) {
    await store.stopProxy();
  } else {
    await store.startProxy();
  }
  await fetchSystemProxy();
}

onMounted(async () => {
  // Fetch the real state first, then enable transitions to avoid the initial "flash" animation.
  await fetchSystemProxy();
  await nextTick();
  systemProxyReady.value = true;

  let lastRunning = store.status.running;

  pollTimer = setInterval(async () => {
    await store.fetchStatus();
    // Always sync system proxy — avoids the timing race where the watcher fired
    // before the backend finished setting the proxy on auto-restore startup.
    fetchSystemProxy();

    // Keep tray tooltip in sync whenever the running state changes (e.g. auto-restore
    // on startup, or sing-box crash) — these paths bypass startProxy/stopProxy so
    // updateTrayTooltip would otherwise never be called.
    if (store.status.running !== lastRunning) {
      lastRunning = store.status.running;
      store.updateTrayTooltip();
    }

    if (store.status.running) {
      // TODO: replace with real traffic data from sing-box API
      const up = Math.random() * 200 * 1024;
      const down = Math.random() * 800 * 1024;
      uploadSpeed.value = up;
      downloadSpeed.value = down;
      totalUpload.value += up;
      totalDownload.value += down;
      store.addTrafficPoint(up, down);
      memoryUsage.value = await invoke<number | null>("cmd_get_memory_usage").catch(() => null);
    } else {
      uploadSpeed.value = 0;
      downloadSpeed.value = 0;
      memoryUsage.value = null;
    }
  }, 1000);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">仪表盘</h1>
      <div class="header-actions">
        <span class="badge" :class="store.status.running ? 'badge-green' : 'badge-gray'">
          <span class="dot" :class="store.status.running ? 'dot-green' : ''" />
          {{ store.status.running ? "运行中" : "已停止" }}
        </span>
      </div>
    </div>

    <!-- Error Banner (top, prominent) -->
    <div v-if="store.error" class="error-banner">
      <span>⚠️ {{ store.error }}</span>
      <button class="btn btn-ghost" @click="store.error = null">关闭</button>
    </div>

    <!-- Quick Controls -->
    <div class="quick-grid">
      <!-- Start/Stop Card -->
      <div class="card control-card" :class="{ active: store.status.running }">
        <div class="control-info">
          <div class="control-icon" :class="store.status.running ? 'icon-green' : 'icon-gray'">
            <component :is="store.status.running ? Wifi : WifiOff" :size="22" />
          </div>
          <div>
            <div class="control-label">代理状态</div>
            <div class="control-value">{{ store.status.running ? "运行中" : "未运行" }}</div>
          </div>
        </div>
        <button
          class="btn"
          :class="store.status.running ? 'btn-danger' : 'btn-primary'"
          :disabled="store.loading"
          @click="toggleProxy"
        >
          <component
            :is="store.status.running ? Square : Play"
            :size="14"
            :class="{ spin: store.loading }"
          />
          {{ store.loading ? (store.status.running ? "停止中..." : "启动中...") : (store.status.running ? "停止" : "启动") }}
        </button>
      </div>

      <!-- Active Node -->
      <div class="card stat-card">
        <div class="stat-icon icon-blue">
          <Server :size="18" />
        </div>
        <div class="stat-content">
          <div class="stat-label">当前节点</div>
          <div class="stat-value text-sm">
            {{ store.activeNode?.name ?? "未选择" }}
          </div>
          <div class="stat-sub">{{ store.activeNode?.server ?? "--" }}</div>
        </div>
      </div>

      <!-- Proxy Mode -->
      <div class="card stat-card">
        <div class="stat-icon icon-purple">
          <Filter :size="18" />
        </div>
        <div class="stat-content">
          <div class="stat-label">代理模式</div>
          <div class="stat-value">{{ proxyModeLabel }}</div>
          <div class="mode-btns">
            <button
              v-for="mode in ['rule', 'global', 'direct']"
              :key="mode"
              class="mode-btn"
              :class="{ active: store.config.proxy_mode === mode }"
              @click="store.setProxyMode(mode)"
            >
              {{ mode === 'rule' ? '规则' : mode === 'global' ? '全局' : '直连' }}
            </button>
          </div>
        </div>
      </div>

      <!-- Uptime -->
      <div class="card stat-card">
        <div class="stat-icon icon-orange">
          <Clock :size="18" />
        </div>
        <div class="stat-content">
          <div class="stat-label">运行时长</div>
          <div class="stat-value">{{ formatUptime(store.status.uptime) }}</div>
          <div class="stat-sub">
            <template v-if="memoryUsage !== null">
              内存 {{ formatBytes(memoryUsage) }} · {{ store.status.version ?? "sing-box" }}
            </template>
            <template v-else>
              {{ store.status.version ?? "sing-box" }}
            </template>
          </div>
        </div>
      </div>
    </div>

    <!-- Network Settings -->
    <div class="card net-settings-card">
      <div class="net-settings-title">
        <Globe :size="14" />
        网络设置
      </div>
      <div class="net-settings-body">
        <!-- System Proxy toggle -->
        <div class="net-row">
          <div class="net-row-left">
            <div class="net-row-icon icon-blue"><Globe :size="15" /></div>
            <div>
              <div class="net-row-label">系统代理</div>
              <div class="net-row-sub">{{ systemProxyEnabled ? `127.0.0.1:${store.config.mixed_port}` : '未开启' }}</div>
            </div>
          </div>
          <button
            class="toggle-btn"
            :class="{ on: systemProxyEnabled, 'no-anim': !systemProxyReady }"
            @click="toggleSystemProxy"
            :title="systemProxyEnabled ? '关闭系统代理' : '开启系统代理'"
          >
            <span class="toggle-knob" />
          </button>
        </div>

        <div class="net-divider" />

        <!-- Proxy mode selector -->
        <div class="net-row">
          <div class="net-row-left">
            <div class="net-row-icon icon-purple"><Filter :size="15" /></div>
            <div>
              <div class="net-row-label">代理模式</div>
              <div class="net-row-sub">{{ proxyModeLabel }}</div>
            </div>
          </div>
          <div class="mode-pills">
            <button
              v-for="[k, label] in [['rule','规则'],['global','全局'],['direct','直连']]"
              :key="k"
              class="mode-pill"
              :class="{ active: store.config.proxy_mode === k }"
              @click="store.setProxyMode(k)"
            >{{ label }}</button>
          </div>
        </div>

        <div class="net-divider" />

        <!-- TUN Mode toggle -->
        <div class="net-row">
          <div class="net-row-left">
            <div class="net-row-icon icon-orange"><Shield :size="15" /></div>
            <div>
              <div class="net-row-label">TUN 模式</div>
              <div class="net-row-sub">{{ store.config.tun_enabled ? '虚拟网卡已启用' : '未启用，需管理员权限' }}</div>
            </div>
          </div>
          <button
            class="toggle-btn"
            :class="{ on: store.config.tun_enabled }"
            @click="async () => { const c = { ...store.config, tun_enabled: !store.config.tun_enabled }; await store.saveConfig(c); }"
          >
            <span class="toggle-knob" />
          </button>
        </div>
      </div>
    </div>

    <!-- Traffic Stats -->
    <div class="traffic-row">
      <div class="card traffic-stat upload">
        <ArrowUp :size="16" />
        <span class="traffic-label">上传速率</span>
        <span class="traffic-value">{{ formatBytes(uploadSpeed) }}/s</span>
        <span class="traffic-total">总计: {{ formatBytes(totalUpload) }}</span>
      </div>
      <div class="card traffic-stat download">
        <ArrowDown :size="16" />
        <span class="traffic-label">下载速率</span>
        <span class="traffic-value">{{ formatBytes(downloadSpeed) }}/s</span>
        <span class="traffic-total">总计: {{ formatBytes(totalDownload) }}</span>
      </div>
    </div>

    <!-- Traffic Chart -->
    <div class="card chart-card">
      <div class="chart-header">
        <Zap :size="15" />
        <span>实时流量</span>
        <div class="chart-legend">
          <span class="legend-item upload-color">▲ 上传</span>
          <span class="legend-item download-color">▼ 下载</span>
        </div>
      </div>
      <div class="chart-body">
        <Line v-if="store.trafficHistory.length > 1" :data="chartData" :options="chartOptions" />
        <div v-else class="chart-empty">启动代理后将显示实时流量数据</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 16px; max-width: 900px; }
.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.page-title {
  font-size: 20px;
  font-weight: 600;
  color: var(--color-text);
}
.dot {
  width: 6px; height: 6px; border-radius: 50%; background: currentColor; display: inline-block;
}
.dot-green { animation: pulse 2s infinite; }
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.quick-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 12px;
}
.control-card {
  padding: 16px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.control-card.active { border-color: rgba(16, 124, 16, 0.3); }
.control-info { display: flex; align-items: center; gap: 12px; }
.control-icon {
  width: 44px; height: 44px;
  border-radius: var(--radius-lg);
  display: flex; align-items: center; justify-content: center;
}
.control-label { font-size: 12px; color: var(--color-text-secondary); margin-bottom: 2px; }
.control-value { font-size: 14px; font-weight: 600; }

.stat-card { padding: 16px; display: flex; align-items: flex-start; gap: 12px; }
.stat-icon {
  width: 40px; height: 40px; border-radius: var(--radius-lg);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
}
.stat-content { flex: 1; min-width: 0; }
.stat-label { font-size: 11px; color: var(--color-text-secondary); text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 4px; }
.stat-value { font-size: 15px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.text-sm { font-size: 13px !important; }
.stat-sub { font-size: 11px; color: var(--color-text-muted); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.icon-green { background: rgba(16,124,16,0.12); color: #107c10; }
.icon-gray { background: rgba(128,128,128,0.1); color: var(--color-text-muted); }
.icon-blue { background: rgba(0,120,212,0.12); color: #0078d4; }
.icon-purple { background: rgba(136,23,152,0.1); color: #881798; }
.icon-orange { background: rgba(202,80,16,0.1); color: #ca5010; }

.mode-btns { display: flex; gap: 4px; margin-top: 6px; }
.mode-btn {
  padding: 2px 8px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 11px; cursor: pointer; transition: all 0.15s;
}
.mode-btn:hover { background: rgba(128,128,128,0.1); }
.mode-btn.active { background: var(--color-primary); color: white; border-color: transparent; }

.traffic-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.traffic-stat {
  padding: 14px 16px;
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.traffic-stat.upload { color: #0078d4; }
.traffic-stat.download { color: #107c10; }
.traffic-label { font-size: 12px; color: var(--color-text-secondary); }
.traffic-value { font-size: 16px; font-weight: 700; margin-left: auto; }
.traffic-total { font-size: 11px; color: var(--color-text-muted); width: 100%; text-align: right; }

.chart-card { padding: 16px; }
.chart-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  margin-bottom: 12px;
}
.chart-legend { margin-left: auto; display: flex; gap: 12px; }
.legend-item { font-size: 11px; font-weight: 500; }
.upload-color { color: #0078d4; }
.download-color { color: #107c10; }
.chart-body { height: 160px; }
.chart-empty {
  height: 100%; display: flex; align-items: center; justify-content: center;
  color: var(--color-text-muted); font-size: 13px;
}

/* Network Settings Card */
.net-settings-card { padding: 14px 16px; }
.net-settings-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.5px;
  margin-bottom: 10px;
}
.net-settings-body { display: flex; flex-direction: column; }
.net-row {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 0; gap: 12px;
}
.net-row-left { display: flex; align-items: center; gap: 10px; }
.net-row-icon {
  width: 32px; height: 32px; border-radius: var(--radius-md);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
}
.net-row-label { font-size: 13px; font-weight: 500; }
.net-row-sub { font-size: 11px; color: var(--color-text-muted); margin-top: 1px; }
.net-divider { height: 1px; background: var(--color-border); margin: 2px 0; }

/* Toggle switch */
.toggle-btn {
  width: 42px; height: 24px; border-radius: 100px;
  background: var(--color-border); border: none; cursor: pointer;
  position: relative; transition: background 0.2s; flex-shrink: 0;
  padding: 0;
}
.toggle-btn.on { background: var(--color-primary); }
.toggle-knob {
  position: absolute; top: 3px; left: 3px;
  width: 18px; height: 18px; border-radius: 50%;
  background: white; transition: transform 0.2s;
  display: block;
}
.toggle-btn.on .toggle-knob { transform: translateX(18px); }
.toggle-btn.no-anim,
.toggle-btn.no-anim .toggle-knob { transition: none; }

/* Mode pills */
.mode-pills { display: flex; gap: 4px; }
.mode-pill {
  padding: 3px 10px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 11px; cursor: pointer; transition: all 0.15s;
}
.mode-pill:hover { background: rgba(128,128,128,0.1); }
.mode-pill.active { background: var(--color-primary); color: white; border-color: transparent; }

.error-banner {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 16px;
  background: rgba(209,52,56,0.08);
  border: 1px solid rgba(209,52,56,0.2);
  border-radius: var(--radius-md);
  color: var(--color-error);
  font-size: 13px;
}

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }
</style>
