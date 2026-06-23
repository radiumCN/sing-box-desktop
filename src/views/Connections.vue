<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Activity } from "@lucide/vue";

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

// Build a compact rule label from rule + rule_payload
function ruleLabel(conn: ConnectionInfo): string {
  if (conn.rule_payload) return conn.rule_payload;
  const r = conn.rule || "final";
  // Shorten long rule names like "rule_set=geosite-cn => route(direct)"
  const match = r.match(/rule_set=([^=\s]+)/);
  if (match) return match[1];
  const matchFn = r.match(/^([^=\s]+)/);
  if (matchFn) return matchFn[1];
  return r;
}

// Full chain: show "A → proxy", just last proxy name
function chainLabel(chains: string[]): string {
  if (!chains || chains.length === 0) return "direct";
  return chains[chains.length - 1];
}

function isProxy(chains: string[]): boolean {
  const label = chainLabel(chains).toLowerCase();
  return label !== "direct" && label !== "block";
}

async function fetchConnections() {
  loading.value = true;
  try {
    connections.value = await invoke<ConnectionInfo[]>("cmd_get_connections");
  } catch {
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
            <th class="col-host">主机</th>
            <th class="col-port">端口</th>
            <th class="col-rule">规则</th>
            <th class="col-chain">代理链</th>
            <th class="col-traffic">上传</th>
            <th class="col-traffic">下载</th>
            <th class="col-proto">协议</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="conn in filtered"
            :key="conn.id"
            :class="{ 'row-proxy': isProxy(conn.chains), 'row-direct': !isProxy(conn.chains) }"
          >
            <td class="col-host">
              <span class="host" :title="conn.host || conn.destination">
                {{ conn.host || conn.destination }}
              </span>
            </td>
            <td class="col-port text-muted">
              {{ conn.destination.split(':').pop() }}
            </td>
            <td class="col-rule">
              <span class="rule-tag" :class="isProxy(conn.chains) ? 'tag-proxy' : 'tag-direct'">
                {{ ruleLabel(conn) }}
              </span>
            </td>
            <td class="col-chain">
              <span class="chain-label" :title="conn.chains.join(' → ')">
                {{ chainLabel(conn.chains) }}
              </span>
            </td>
            <td class="col-traffic upload-val">↑ {{ formatBytes(conn.upload) }}</td>
            <td class="col-traffic download-val">↓ {{ formatBytes(conn.download) }}</td>
            <td class="col-proto">
              <span class="proto-tag">{{ conn.network.toUpperCase() }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 12px; height: 100%; }
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

.conn-table-wrapper { overflow: auto; flex: 1; }
.conn-table {
  width: 100%; border-collapse: collapse; font-size: 12px; white-space: nowrap;
}
.conn-table th {
  text-align: left; padding: 7px 10px;
  border-bottom: 2px solid var(--color-border);
  color: var(--color-text-secondary); font-weight: 600;
  font-size: 11px; letter-spacing: 0.3px;
  position: sticky; top: 0; background: var(--color-surface); z-index: 1;
}
.conn-table td {
  padding: 7px 10px;
  border-bottom: 1px solid var(--color-border);
  vertical-align: middle;
}
.conn-table tr:hover td { background: rgba(128,128,128,0.05); }

/* Row tinting */
.row-proxy td:first-child { border-left: 2px solid rgba(0,120,212,0.35); }
.row-direct td:first-child { border-left: 2px solid rgba(16,124,16,0.25); }

/* Columns */
.col-host { max-width: 220px; }
.host {
  display: block; overflow: hidden; text-overflow: ellipsis;
  font-weight: 500; max-width: 210px;
}
.col-port { color: var(--color-text-muted); width: 50px; }
.col-rule { max-width: 160px; }
.col-chain { max-width: 120px; color: var(--color-text-secondary); }
.col-traffic { width: 80px; }
.col-proto { width: 52px; }

.text-muted { color: var(--color-text-muted); }

.rule-tag {
  display: inline-block; padding: 2px 7px; border-radius: 100px;
  font-size: 10px; font-weight: 600; max-width: 150px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.tag-proxy { background: rgba(0,120,212,0.1); color: #0078d4; }
.tag-direct { background: rgba(16,124,16,0.1); color: #107c10; }

.chain-label {
  display: block; overflow: hidden; text-overflow: ellipsis;
  max-width: 115px; font-size: 11px;
}

.upload-val { color: #0078d4; font-weight: 500; }
.download-val { color: #107c10; font-weight: 500; }

.proto-tag {
  display: inline-block; padding: 1px 5px; border-radius: 3px;
  background: rgba(128,128,128,0.1); color: var(--color-text-secondary);
  font-size: 10px; font-weight: 600;
}

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }
</style>
