<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { Gauge, RefreshCw, CheckCircle, Signal, Zap, ArrowUpDown } from "@lucide/vue";
import { useAppStore } from "../stores/app";

const store = useAppStore();
const testingAll = ref(false);
const testingGroup = ref(false);
const testingIds = ref<string[]>([]);
const filterSubId = ref<string>(localStorage.getItem("nodes_filter_sub") ?? "all");
const sortBy = ref<"none" | "latency" | "speed">(
  (localStorage.getItem("nodes_sort") as "none" | "latency" | "speed") ?? "none"
);
const search = ref("");

watch(sortBy, (v) => localStorage.setItem("nodes_sort", v));

function validateSubFilter() {
  const savedSub = filterSubId.value;
  if (savedSub !== "all" && store.subscriptions.length > 0 && !store.subscriptions.find((s) => s.id === savedSub)) {
    filterSubId.value = "all";
    localStorage.setItem("nodes_filter_sub", "all");
  }
}

// Re-validate when subscriptions load (async after mount)
watch(() => store.subscriptions.length, validateSubFilter);

onMounted(() => {
  validateSubFilter();
  // Use the store's single shared poller for the active auto group's current node.
  store.ensureActiveNowPoller();
});

const nodesForSub = computed(() => {
  if (filterSubId.value === "all") return store.nodes;
  return store.nodes.filter((n) => n.subscription_id === filterSubId.value);
});

const filtered = computed(() => {
  let nodes = nodesForSub.value;
  if (search.value) {
    const q = search.value.toLowerCase();
    nodes = nodes.filter(
      (n) =>
        n.name.toLowerCase().includes(q) ||
        n.server.toLowerCase().includes(q) ||
        n.protocol.toLowerCase().includes(q)
    );
  }
  if (sortBy.value === "latency") {
    nodes = [...nodes].sort((a, b) => {
      if (a.latency == null && b.latency == null) return 0;
      if (a.latency == null) return 1;
      if (b.latency == null) return -1;
      return a.latency - b.latency;
    });
  } else if (sortBy.value === "speed") {
    nodes = [...nodes].sort((a, b) => {
      if (a.download_speed == null && b.download_speed == null) return 0;
      if (a.download_speed == null) return 1;
      if (b.download_speed == null) return -1;
      return b.download_speed - a.download_speed;
    });
  }
  return nodes;
});

function switchSub(id: string) {
  filterSubId.value = id;
  localStorage.setItem("nodes_filter_sub", id);
}

const latencyColor = (ms?: number) => {
  if (ms === undefined || ms === null) return "var(--color-text-muted)";
  if (ms < 100) return "#107c10";
  if (ms < 300) return "#ca5010";
  return "#d13438";
};

const latencyLabel = (ms?: number) => {
  if (ms === undefined || ms === null) return "--";
  return `${ms}ms`;
};

const speedColor = (kbps?: number) => {
  if (kbps === undefined || kbps === null) return "var(--color-text-muted)";
  if (kbps >= 5120) return "#107c10";   // ≥ 5 MB/s
  if (kbps >= 1024) return "#ca5010";   // ≥ 1 MB/s
  return "#d13438";                      // < 1 MB/s
};

const speedLabel = (kbps?: number) => {
  if (kbps === undefined || kbps === null) return "";
  if (kbps >= 1024) return `${(kbps / 1024).toFixed(1)} MB/s`;
  return `${kbps} KB/s`;
};

function isTesting(id: string) {
  return testingIds.value.includes(id);
}

async function testAll() {
  testingAll.value = true;
  await Promise.allSettled(
    store.nodes.map(async (n) => {
      testingIds.value = [...testingIds.value, n.id];
      await store.testNodeSpeed(n.id);
      testingIds.value = testingIds.value.filter((id) => id !== n.id);
    })
  );
  testingAll.value = false;
}

async function testOne(nodeId: string) {
  testingIds.value = [...testingIds.value, nodeId];
  await store.testNodeSpeed(nodeId);
  testingIds.value = testingIds.value.filter((id) => id !== nodeId);
}

async function selectNode(nodeId: string) {
  await store.setActiveNode(nodeId);
}

// Switch to a dynamic urltest group (core continuously picks the fastest node).
// No arg = global "auto"; pass a subscription id for that subscription's group.
async function selectAuto(subId?: string) {
  await store.setAutoNode(subId ? `auto-${subId}` : undefined);
}

// Force an immediate re-test of the current view's auto group.
async function retestGroup() {
  if (testingGroup.value) return;
  testingGroup.value = true;
  try {
    await store.testGroupDelay(currentAutoTag.value);
  } finally {
    testingGroup.value = false;
  }
}

// The auto group tag matching the current view (global vs per-subscription).
const currentAutoTag = computed(() =>
  filterSubId.value === "all" ? "auto" : `auto-${filterSubId.value}`
);
// Show a per-subscription auto card only when that subscription has ≥2 nodes
// (matches the backend, which only builds a urltest group in that case).
const showAutoCard = computed(() =>
  filterSubId.value === "all" ? store.nodes.length > 0 : nodesForSub.value.length >= 2
);
// The node the active auto group is currently routing through (for display).
const autoNowName = computed(() => store.activeNodeNow);
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">节点列表</h1>
      <div class="header-actions">
        <span class="node-count">{{ store.nodes.length }} 个节点</span>
        <button class="btn btn-ghost" :disabled="testingAll" @click="testAll" title="测试所有节点的延迟和网速">
          <Gauge :size="14" :class="{ spin: testingAll }" />
          {{ testingAll ? "测试中..." : "全部测速" }}
        </button>
        <!-- Sort selector -->
        <div class="sort-group">
          <ArrowUpDown :size="13" />
          <button
            v-for="[k, label] in [['none','默认'],['latency','延迟'],['speed','速度']]"
            :key="k"
            class="sort-btn"
            :class="{ active: sortBy === k }"
            @click="sortBy = k as typeof sortBy"
          >{{ label }}</button>
        </div>

        <button class="btn btn-ghost" @click="store.fetchNodes">
          <RefreshCw :size="14" />
          刷新
        </button>
      </div>
    </div>

    <!-- Filters -->
    <div class="filters">
      <input class="input search-input" v-model="search" placeholder="搜索节点名称或服务器..." />

      <!-- Subscription tabs (show only if more than one sub) -->
      <div v-if="store.subscriptions.length > 0" class="sub-tabs">
        <button
          class="sub-tab"
          :class="{ active: filterSubId === 'all' }"
          @click="switchSub('all')"
        >
          全部 <span class="sub-count">{{ store.nodes.length }}</span>
        </button>
        <button
          v-for="sub in store.subscriptions"
          :key="sub.id"
          class="sub-tab"
          :class="{ active: filterSubId === sub.id }"
          @click="switchSub(sub.id)"
        >
          {{ sub.name }}
          <span class="sub-count">{{ store.nodes.filter(n => n.subscription_id === sub.id).length }}</span>
        </button>
      </div>
    </div>

    <!-- Speed-test notice when proxy is not running -->
    <div v-if="store.nodes.length > 0 && !store.status.running" class="speed-notice">
      <span>⚡ 代理未运行：测速只能测延迟，<strong>下载速度测试需要先启动代理</strong></span>
    </div>

    <!-- Empty -->
    <div v-if="store.nodes.length === 0" class="empty-state">
      <Signal :size="36" class="empty-icon" />
      <div class="empty-title">暂无节点</div>
      <div class="empty-desc">请先在「订阅」页面添加订阅</div>
    </div>

    <!-- Node List -->
    <div class="node-list">
      <!-- Dynamic auto-select (urltest) group — global or per-subscription -->
      <div
        v-if="showAutoCard"
        class="card node-item auto-item"
        :class="{ active: store.activeProxyTag === currentAutoTag }"
        @click="selectAuto(filterSubId === 'all' ? undefined : filterSubId)"
        title="动态自动选优：内核持续测速并自动切换到延迟最低的节点（Clash.Meta「Auto」）"
      >
        <div class="node-left">
          <div class="active-indicator">
            <Zap v-if="store.activeProxyTag === currentAutoTag" :size="16" class="auto-icon" />
            <div v-else class="check-placeholder" />
          </div>
          <div class="node-info">
            <div class="node-name">
              {{ filterSubId === 'all' ? '自动选优（全部节点）' : '自动选优（本订阅）' }}
            </div>
            <div class="node-meta">
              <span class="badge badge-gray protocol-badge">URLTest</span>
              <span
                v-if="store.activeProxyTag === currentAutoTag && autoNowName"
                class="node-server auto-now"
              >当前命中：{{ autoNowName }}</span>
              <span v-else class="node-server">内核持续测速，自动切换最快节点</span>
            </div>
          </div>
        </div>
        <div class="node-right">
          <button
            class="btn btn-ghost icon-btn"
            :disabled="testingGroup || !store.status.running"
            @click.stop="retestGroup"
            :title="store.status.running ? '立即重测本组所有节点并刷新最优' : '需先启动代理'"
          >
            <RefreshCw :size="13" :class="{ spin: testingGroup }" />
          </button>
        </div>
      </div>

      <div
        v-for="node in filtered"
        :key="node.id"
        class="card node-item"
        :class="{ active: node.is_active }"
        @click="selectNode(node.id)"
      >
        <div class="node-left">
          <div class="active-indicator">
            <CheckCircle v-if="node.is_active" :size="16" class="check-icon" />
            <div v-else class="check-placeholder" />
          </div>
          <div class="node-info">
            <div class="node-name">{{ node.name }}</div>
            <div class="node-meta">
              <span class="badge badge-gray protocol-badge">{{ node.protocol }}</span>
              <span class="node-server">{{ node.server }}:{{ node.port }}</span>
            </div>
          </div>
        </div>
        <div class="node-right">
          <div class="speed-info">
            <span class="latency" :style="{ color: latencyColor(node.latency) }">
              {{ latencyLabel(node.latency) }}
            </span>
            <!-- Show download speed if measured; show "↓ --" if tested but proxy was off -->
            <span
              v-if="node.latency !== undefined && node.latency !== null"
              class="download-speed"
              :style="{ color: node.download_speed != null ? speedColor(node.download_speed) : 'var(--color-text-muted)' }"
              :title="node.download_speed == null ? '下载测速需要先启动代理' : ''"
            >
              ↓ {{ node.download_speed != null ? speedLabel(node.download_speed) : '--' }}
            </span>
          </div>
          <button
            class="btn btn-ghost icon-btn"
            :disabled="isTesting(node.id)"
            @click.stop="testOne(node.id)"
            :title="store.status.running ? '测试延迟 + 下载速度' : '测试延迟（启动代理后可测速）'"
          >
            <Gauge :size="13" :class="{ spin: isTesting(node.id) }" />
          </button>
        </div>
      </div>
    </div>

    <div v-if="filtered.length === 0 && store.nodes.length > 0" class="no-result">
      没有匹配「{{ search }}」的节点
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 14px; max-width: 800px; }
.page-header { display: flex; align-items: center; justify-content: space-between; }
.page-title { font-size: 20px; font-weight: 600; }
.header-actions { display: flex; align-items: center; gap: 8px; }
.node-count { font-size: 12px; color: var(--color-text-secondary); }

.filters { display: flex; flex-direction: column; gap: 10px; }
.search-input { max-width: 340px; }

.sub-tabs {
  display: flex; gap: 4px; flex-wrap: wrap;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--color-border);
}
.sub-tab {
  display: flex; align-items: center; gap: 5px;
  padding: 5px 14px; border-radius: var(--radius-md);
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 12px; font-weight: 500; cursor: pointer; transition: all 0.15s;
}
.sub-tab:hover { background: rgba(128,128,128,0.1); color: var(--color-text); }
.sub-tab.active {
  background: rgba(0,120,212,0.1);
  border-color: rgba(0,120,212,0.35);
  color: var(--color-primary);
}
.sub-count {
  font-size: 10px; font-weight: 700;
  background: rgba(128,128,128,0.15);
  border-radius: 100px; padding: 0 5px; min-width: 18px; text-align: center;
}
.sub-tab.active .sub-count {
  background: rgba(0,120,212,0.15);
}

.empty-state {
  display: flex; flex-direction: column; align-items: center; gap: 10px;
  padding: 48px 24px; color: var(--color-text-muted);
}
.empty-icon { opacity: 0.35; }
.empty-title { font-size: 15px; font-weight: 600; color: var(--color-text-secondary); }
.empty-desc { font-size: 13px; }

.node-list { display: flex; flex-direction: column; gap: 6px; }
.node-item {
  padding: 12px 16px;
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
  cursor: pointer; transition: all 0.15s;
  border: 1px solid var(--color-border);
}
.node-item:hover { box-shadow: var(--shadow-md); background: var(--color-surface-strong); }
.node-item.active {
  border-color: rgba(0,120,212,0.4);
  background: rgba(0,120,212,0.04);
}
.auto-item.active {
  border-color: rgba(240,192,64,0.6);
  background: rgba(240,192,64,0.08);
}
.auto-item .auto-icon { color: #f0c040; }
.auto-item .auto-now { color: #f0c040; font-weight: 500; }
.node-left { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
.active-indicator { flex-shrink: 0; }
.check-icon { color: var(--color-primary); }
.auto-icon { color: #f0c040; }
.check-placeholder { width: 16px; height: 16px; }
.node-info { flex: 1; min-width: 0; }
.node-name { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.node-meta { display: flex; align-items: center; gap: 6px; margin-top: 3px; }
.protocol-badge { font-size: 10px; padding: 1px 6px; }
.node-server { font-size: 11px; color: var(--color-text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.node-right { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.speed-info { display: flex; flex-direction: column; align-items: flex-end; gap: 2px; min-width: 72px; }
.latency { font-size: 12px; font-weight: 600; }
.download-speed { font-size: 11px; font-weight: 500; }
.icon-btn { padding: 5px !important; }

.no-result { text-align: center; color: var(--color-text-muted); font-size: 13px; padding: 24px; }

.speed-notice {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 14px; border-radius: 8px; font-size: 12px;
  background: rgba(202,80,16,0.08); border: 1px solid rgba(202,80,16,0.2);
  color: #ca5010;
}
.speed-notice strong { color: var(--color-text); }

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }

.sort-group {
  display: flex; align-items: center; gap: 3px;
  padding: 3px 6px 3px 8px; border-radius: var(--radius-md);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted); font-size: 12px;
}
.sort-btn {
  padding: 2px 7px; border-radius: var(--radius-sm);
  border: none; background: transparent;
  color: var(--color-text-secondary);
  font-size: 11px; cursor: pointer; transition: all 0.15s;
}
.sort-btn:hover { background: rgba(128,128,128,0.1); }
.sort-btn.active { background: var(--color-primary); color: white; border-radius: var(--radius-sm); }
</style>
