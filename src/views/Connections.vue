<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

const { t } = useI18n();
import { RefreshCw, Activity, X, Ban } from "@lucide/vue";

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
const refreshing = ref(false);
const search = ref("");
const grouped = ref(false);
type SortKey = "default" | "host" | "upload" | "download" | "proto";
const sortKey = ref<SortKey>("default");
const sortDir = ref<"asc" | "desc">("desc");
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

// Click a sortable header: toggle direction if same key, else switch key (desc first
// for traffic — users want the heaviest connections on top).
function toggleSort(key: Exclude<SortKey, "default">) {
  if (sortKey.value === key) {
    sortDir.value = sortDir.value === "asc" ? "desc" : "asc";
  } else {
    sortKey.value = key;
    sortDir.value = key === "host" || key === "proto" ? "asc" : "desc";
  }
}

const sorted = computed(() => {
  if (sortKey.value === "default") return filtered.value;
  const dir = sortDir.value === "asc" ? 1 : -1;
  const key = sortKey.value;
  return [...filtered.value].sort((a, b) => {
    let r = 0;
    if (key === "upload") r = a.upload - b.upload;
    else if (key === "download") r = a.download - b.download;
    else if (key === "host") r = (a.host || a.destination).localeCompare(b.host || b.destination);
    else if (key === "proto") r = a.network.localeCompare(b.network);
    return r * dir;
  });
});

// Live totals across the currently-filtered connections.
const totals = computed(() =>
  filtered.value.reduce(
    (acc, c) => ({ upload: acc.upload + c.upload, download: acc.download + c.download }),
    { upload: 0, download: 0 }
  )
);

interface HostGroup {
  host: string;
  count: number;
  upload: number;
  download: number;
  proxied: boolean;
  ids: string[];
}

// Group filtered connections by host (or destination when host is empty), summing
// traffic — useful when one site opens dozens of connections.
const hostGroups = computed<HostGroup[]>(() => {
  const map = new Map<string, HostGroup>();
  for (const c of filtered.value) {
    const host = c.host || c.destination;
    const g = map.get(host) ?? {
      host,
      count: 0,
      upload: 0,
      download: 0,
      proxied: isProxy(c.chains),
      ids: [],
    };
    g.count += 1;
    g.upload += c.upload;
    g.download += c.download;
    g.ids.push(c.id);
    map.set(host, g);
  }
  const arr = [...map.values()];
  // Default heaviest-first; honour the active sort when it applies to a group field.
  const dir = sortDir.value === "asc" ? 1 : -1;
  arr.sort((a, b) => {
    if (sortKey.value === "host") return a.host.localeCompare(b.host) * dir;
    if (sortKey.value === "upload") return (a.upload - b.upload) * dir;
    return (b.download + b.upload) - (a.download + a.upload);
  });
  return arr;
});

async function closeHostGroup(g: HostGroup) {
  await Promise.allSettled(g.ids.map((id) => invoke("cmd_close_connection", { id })));
  connections.value = connections.value.filter((c) => !g.ids.includes(c.id));
}

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

// Manual refresh keeps the spin visible for at least 600ms — the 2s poll
// otherwise toggles `loading` too briefly for the animation to register.
async function manualRefresh() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    await Promise.all([fetchConnections(), new Promise((r) => setTimeout(r, 600))]);
  } finally {
    refreshing.value = false;
  }
}

async function closeConnection(id: string) {
  try {
    await invoke("cmd_close_connection", { id });
    connections.value = connections.value.filter((c) => c.id !== id);
  } catch {
    // Ignore — the connection may have already closed; next poll reconciles.
  }
}

async function closeAll() {
  try {
    await invoke("cmd_close_all_connections");
    connections.value = [];
  } catch {
    // Ignore — next poll reconciles state.
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
      <h1 class="page-title">{{ t('connections.title') }}</h1>
      <div class="header-actions">
        <span class="conn-count">{{ t('connections.connCount', { count: connections.length }) }}</span>
        <span class="conn-total">
          {{ t('connections.total') }}
          <span class="upload-val">↑ {{ formatBytes(totals.upload) }}</span>
          <span class="download-val">↓ {{ formatBytes(totals.download) }}</span>
        </span>
        <div class="view-tabs">
          <button class="view-tab" :class="{ active: !grouped }" @click="grouped = false">
            {{ t('connections.viewList') }}
          </button>
          <button class="view-tab" :class="{ active: grouped }" @click="grouped = true">
            {{ t('connections.viewGrouped') }}
          </button>
        </div>
        <button class="btn btn-ghost" @click="closeAll" :disabled="connections.length === 0">
          <Ban :size="14" />
          {{ t('connections.closeAll') }}
        </button>
        <button class="btn btn-ghost" @click="manualRefresh" :disabled="refreshing">
          <RefreshCw :size="14" :class="{ spin: refreshing }" />
          {{ t('connections.refresh') }}
        </button>
      </div>
    </div>

    <input class="input" v-model="search" :placeholder="t('connections.searchPlaceholder')" style="max-width: 400px" />

    <div v-if="connections.length === 0 && !loading" class="empty-state">
      <Activity :size="36" class="empty-icon" />
      <div class="empty-title">{{ t('connections.emptyTitle') }}</div>
      <div class="empty-desc">{{ t('connections.emptyDesc') }}</div>
    </div>
    <div v-else-if="filtered.length === 0 && search" class="empty-state">
      <Activity :size="36" class="empty-icon" />
      <div class="empty-title">{{ t('connections.noResult', { q: search }) }}</div>
    </div>

    <!-- Grouped-by-host view -->
    <div v-else-if="grouped" class="conn-table-wrapper">
      <table class="conn-table">
        <thead>
          <tr>
            <th class="col-host sortable" @click="toggleSort('host')">
              {{ t('connections.colHost') }}<span v-if="sortKey === 'host'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-port">{{ t('connections.colCount') }}</th>
            <th class="col-traffic sortable" @click="toggleSort('upload')">
              {{ t('connections.colUpload') }}<span v-if="sortKey === 'upload'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-traffic sortable" @click="toggleSort('download')">
              {{ t('connections.colDownload') }}<span v-if="sortKey === 'download'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-close"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="g in hostGroups" :key="g.host" :class="{ 'row-proxy': g.proxied, 'row-direct': !g.proxied }">
            <td class="col-host"><span class="host" :title="g.host">{{ g.host }}</span></td>
            <td class="col-port text-muted">{{ g.count }}</td>
            <td class="col-traffic upload-val">↑ {{ formatBytes(g.upload) }}</td>
            <td class="col-traffic download-val">↓ {{ formatBytes(g.download) }}</td>
            <td class="col-close">
              <button class="close-btn" :title="t('connections.closeHost')" @click="closeHostGroup(g)">
                <X :size="13" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Flat list view -->
    <div v-else class="conn-table-wrapper">
      <table v-if="sorted.length > 0" class="conn-table">
        <thead>
          <tr>
            <th class="col-host sortable" @click="toggleSort('host')">
              {{ t('connections.colHost') }}<span v-if="sortKey === 'host'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-port">{{ t('connections.colPort') }}</th>
            <th class="col-rule">{{ t('connections.colRule') }}</th>
            <th class="col-chain">{{ t('connections.colChain') }}</th>
            <th class="col-traffic sortable" @click="toggleSort('upload')">
              {{ t('connections.colUpload') }}<span v-if="sortKey === 'upload'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-traffic sortable" @click="toggleSort('download')">
              {{ t('connections.colDownload') }}<span v-if="sortKey === 'download'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-proto sortable" @click="toggleSort('proto')">
              {{ t('connections.colProto') }}<span v-if="sortKey === 'proto'" class="sort-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
            <th class="col-close"></th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="conn in sorted"
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
            <td class="col-close">
              <button class="close-btn" :title="t('connections.closeConn')" @click="closeConnection(conn.id)">
                <X :size="13" />
              </button>
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
.conn-total { font-size: 12px; color: var(--color-text-secondary); display: inline-flex; gap: 6px; align-items: center; }

.view-tabs { display: flex; gap: 2px; background: var(--color-bg-secondary, rgba(128,128,128,0.1)); border-radius: 8px; padding: 2px; }
.view-tab {
  border: none; background: transparent; cursor: pointer; font-size: 12px;
  padding: 4px 10px; border-radius: 6px; color: var(--color-text-secondary);
}
.view-tab.active { background: var(--color-surface); color: var(--color-text); box-shadow: 0 1px 2px rgba(0,0,0,0.08); }

.sortable { cursor: pointer; user-select: none; }
.sortable:hover { color: var(--color-text); }
.sort-arrow { margin-left: 3px; font-size: 9px; }

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
.col-close { width: 32px; text-align: center; }

.close-btn {
  display: inline-flex; align-items: center; justify-content: center;
  width: 22px; height: 22px; border: none; border-radius: 4px;
  background: transparent; color: var(--color-text-muted); cursor: pointer;
  opacity: 0; transition: opacity 0.12s, background 0.12s, color 0.12s;
}
.conn-table tr:hover .close-btn { opacity: 1; }
.close-btn:hover { background: rgba(232,17,35,0.12); color: #e81123; }

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
