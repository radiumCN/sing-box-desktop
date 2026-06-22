<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { Gauge, RefreshCw, CheckCircle, Signal, Zap } from "@lucide/vue";
import { useAppStore } from "../stores/app";

const store = useAppStore();
const testingAll = ref(false);
const autoSelecting = ref(false);
const testingIds = ref<string[]>([]);
const filterSubId = ref<string>(localStorage.getItem("nodes_filter_sub") ?? "all");
const filterGroup = ref(localStorage.getItem("nodes_filter_group") ?? "全部");
const search = ref("");
const autoSelectedId = ref<string | null>(localStorage.getItem("auto_selected_node_id"));

// Persist group filter in localStorage
watch(filterGroup, (val) => localStorage.setItem("nodes_filter_group", val));

// On mount, validate saved IDs still exist
onMounted(() => {
  // Validate subscription filter
  const savedSub = localStorage.getItem("nodes_filter_sub");
  if (savedSub && savedSub !== "all" && !store.subscriptions.find((s) => s.id === savedSub)) {
    filterSubId.value = "all";
    localStorage.setItem("nodes_filter_sub", "all");
  }

  // Validate auto-selected node — clear if the node no longer exists or is no longer active
  const savedAutoId = localStorage.getItem("auto_selected_node_id");
  if (savedAutoId) {
    const node = store.nodes.find((n) => n.id === savedAutoId);
    if (!node || !node.is_active) {
      autoSelectedId.value = null;
      localStorage.removeItem("auto_selected_node_id");
    }
  }
});

const nodesForSub = computed(() => {
  if (filterSubId.value === "all") return store.nodes;
  return store.nodes.filter((n) => n.subscription_id === filterSubId.value);
});

const allGroups = computed(() => {
  const groups = new Set(nodesForSub.value.map((n) => n.group));
  return ["全部", ...groups];
});

const filtered = computed(() => {
  let nodes = nodesForSub.value;
  if (filterGroup.value !== "全部") {
    nodes = nodes.filter((n) => n.group === filterGroup.value);
  }
  if (search.value) {
    const q = search.value.toLowerCase();
    nodes = nodes.filter(
      (n) =>
        n.name.toLowerCase().includes(q) ||
        n.server.toLowerCase().includes(q) ||
        n.protocol.toLowerCase().includes(q)
    );
  }
  return nodes;
});

function switchSub(id: string) {
  filterSubId.value = id;
  filterGroup.value = "全部";
  localStorage.setItem("nodes_filter_sub", id);
  localStorage.removeItem("nodes_filter_group");
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
  // Manual selection clears the auto-select indicator
  autoSelectedId.value = null;
  localStorage.removeItem("auto_selected_node_id");
}

async function autoSelect() {
  autoSelecting.value = true;
  autoSelectedId.value = null;
  localStorage.removeItem("auto_selected_node_id");
  try {
    const bestId = await store.autoSelectNode();
    if (bestId) {
      autoSelectedId.value = bestId;
      localStorage.setItem("auto_selected_node_id", bestId);
    }
  } finally {
    autoSelecting.value = false;
  }
}
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">节点列表</h1>
      <div class="header-actions">
        <span class="node-count">{{ store.nodes.length }} 个节点</span>
        <button
          class="btn btn-ghost auto-btn"
          :disabled="autoSelecting"
          @click="autoSelect"
          title="测试所有节点并自动选择延迟最低的"
        >
          <Zap :size="14" :class="{ spin: autoSelecting }" />
          {{ autoSelecting ? "选择中..." : "Auto 选择" }}
        </button>
        <button class="btn btn-ghost" :disabled="testingAll" @click="testAll" title="测试所有节点的延迟和网速">
          <Gauge :size="14" :class="{ spin: testingAll }" />
          {{ testingAll ? "测试中..." : "全部测速" }}
        </button>
        <button class="btn btn-ghost" @click="store.fetchNodes">
          <RefreshCw :size="14" />
          刷新
        </button>
      </div>
    </div>

    <!-- Auto-select result banner -->
    <div v-if="autoSelectedId" class="auto-banner">
      <Zap :size="13" />
      已自动选择延迟最低节点：<strong>{{ store.nodes.find(n => n.id === autoSelectedId)?.name }}</strong>
      <button class="banner-close" @click="autoSelectedId = null; localStorage.removeItem('auto_selected_node_id')">×</button>
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

      <div class="group-tabs">
        <button
          v-for="g in allGroups"
          :key="g"
          class="group-tab"
          :class="{ active: filterGroup === g }"
          @click="filterGroup = g"
        >
          {{ g }}
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
      <div
        v-for="node in filtered"
        :key="node.id"
        class="card node-item"
        :class="{
          active: node.is_active,
          'auto-selected': node.id === autoSelectedId,
        }"
        @click="selectNode(node.id)"
      >
        <div class="node-left">
          <div class="active-indicator">
            <Zap v-if="node.id === autoSelectedId && node.is_active" :size="16" class="auto-icon" />
            <CheckCircle v-else-if="node.is_active" :size="16" class="check-icon" />
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

.auto-btn { color: #f0c040 !important; }
.auto-btn:hover:not(:disabled) { background: rgba(240,192,64,0.12) !important; }

.auto-banner {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 14px; border-radius: 8px;
  background: rgba(240,192,64,0.12); border: 1px solid rgba(240,192,64,0.3);
  font-size: 13px; color: #f0c040;
}
.auto-banner strong { color: var(--color-text-primary); }
.banner-close {
  margin-left: auto; background: transparent; border: none;
  color: var(--color-text-muted); cursor: pointer; font-size: 16px; padding: 0 4px;
}

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

.group-tabs { display: flex; gap: 6px; flex-wrap: wrap; }
.group-tab {
  padding: 4px 12px; border-radius: 100px;
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 12px; cursor: pointer; transition: all 0.15s;
}
.group-tab:hover { background: rgba(128,128,128,0.1); }
.group-tab.active { background: var(--color-primary); color: white; border-color: transparent; }

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
.node-item.auto-selected {
  border-color: rgba(240,192,64,0.5);
  background: rgba(240,192,64,0.05);
}
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
</style>
