<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Activity, ArrowUp, ArrowDown } from "@lucide/vue";

interface ConnectionInfo {
  id: string;
  network: string;
  conn_type: string;
  source: string;
  destination: string;
  host: string;
  rule: string;
  rule_payload: string;
  chains: string[];
  upload: number;
  download: number;
  start: string;
}

const connections = ref<ConnectionInfo[]>([]);
const loading = ref(false);
const search = ref("");
let pollTimer: ReturnType<typeof setInterval> | null = null;

const filtered = computed(() => {
  if (!search.value) return connections.value;
  const q = search.value.toLowerCase();
  return connections.value.filter(
    (c) =>
      c.host.toLowerCase().includes(q) ||
      c.destination.toLowerCase().includes(q) ||
      c.rule.toLowerCase().includes(q) ||
      c.chains.join("").toLowerCase().includes(q)
  );
});

const formatBytes = (b: number) => {
  if (b < 1024) return `${b}B`;
  if (b < 1048576) return `${(b / 1024).toFixed(1)}KB`;
  return `${(b / 1048576).toFixed(1)}MB`;
};

async function fetchConnections() {
  loading.value = true;
  try {
    connections.value = await invoke<ConnectionInfo[]>("cmd_get_connections");
  } catch {
    // sing-box may not be running
    connections.value = [];
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  fetchConnections();
  pollTimer = setInterval(fetchConnections, 2000);
});
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">活动连接</h1>
      <div class="header-actions">
        <span class="conn-count">{{ connections.length }} 个连接</span>
        <button class="btn btn-ghost" @click="fetchConnections" :disabled="loading">
          <RefreshCw :size="14" :class="{ spin: loading }" />
          刷新
        </button>
      </div>
    </div>

    <input class="input" v-model="search" placeholder="搜索主机、目标、规则..." style="max-width: 400px" />

    <div v-if="connections.length === 0 && !loading" class="empty-state">
      <Activity :size="36" class="empty-icon" />
      <div class="empty-title">暂无活动连接</div>
      <div class="empty-desc">代理运行后将在此显示实时连接信息</div>
    </div>

    <div class="conn-table-wrapper">
      <table v-if="filtered.length > 0" class="conn-table">
        <thead>
          <tr>
            <th>主机</th>
            <th>目标</th>
            <th>规则</th>
            <th>代理链</th>
            <th>上传</th>
            <th>下载</th>
            <th>协议</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="conn in filtered" :key="conn.id">
            <td class="host-cell">
              <span class="host" :title="conn.host">{{ conn.host || conn.destination }}</span>
            </td>
            <td class="addr-cell">{{ conn.destination }}</td>
            <td>
              <span class="badge badge-blue rule-badge">{{ conn.rule }}</span>
            </td>
            <td class="chain-cell">{{ conn.chains.join(" → ") }}</td>
            <td class="traffic-cell upload">
              <ArrowUp :size="10" />
              {{ formatBytes(conn.upload) }}
            </td>
            <td class="traffic-cell download">
              <ArrowDown :size="10" />
              {{ formatBytes(conn.download) }}
            </td>
            <td>
              <span class="badge badge-gray">{{ conn.network }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 14px; max-width: 1100px; }
.page-header { display: flex; align-items: center; justify-content: space-between; }
.page-title { font-size: 20px; font-weight: 600; }
.header-actions { display: flex; align-items: center; gap: 8px; }
.conn-count { font-size: 12px; color: var(--color-text-secondary); }

.empty-state {
  display: flex; flex-direction: column; align-items: center; gap: 10px;
  padding: 48px 24px; color: var(--color-text-muted); text-align: center;
}
.empty-icon { opacity: 0.35; }
.empty-title { font-size: 15px; font-weight: 600; color: var(--color-text-secondary); }
.empty-desc { font-size: 13px; }

.conn-table-wrapper { overflow-x: auto; }
.conn-table {
  width: 100%; border-collapse: collapse; font-size: 12px;
}
.conn-table th {
  text-align: left; padding: 8px 12px;
  border-bottom: 1px solid var(--color-border);
  color: var(--color-text-secondary); font-weight: 600;
  font-size: 11px; text-transform: uppercase; letter-spacing: 0.4px;
}
.conn-table td {
  padding: 8px 12px;
  border-bottom: 1px solid var(--color-border);
  vertical-align: middle;
}
.conn-table tr:hover td { background: rgba(128,128,128,0.04); }
.host-cell { max-width: 200px; }
.host { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: block; font-weight: 500; }
.addr-cell { color: var(--color-text-secondary); font-family: 'Cascadia Code', monospace; }
.rule-badge { font-size: 10px !important; }
.chain-cell { color: var(--color-text-secondary); max-width: 150px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.traffic-cell { display: flex; align-items: center; gap: 3px; white-space: nowrap; }
.upload { color: #0078d4; }
.download { color: #107c10; }

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }
</style>
