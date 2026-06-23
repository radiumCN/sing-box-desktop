<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, watch, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Trash2, ArrowDown, Copy } from "@lucide/vue";

const logs = ref<string[]>([]);
const autoScroll = ref(true);
const filterLevel = ref("all");
const logContainer = ref<HTMLElement | null>(null);
let pollTimer: ReturnType<typeof setInterval> | null = null;

const levelColors: Record<string, string> = {
  error: "#d13438",
  warn: "#ca5010",
  info: "#0078d4",
  debug: "#616161",
};

const parsedLogs = computed(() =>
  logs.value.map((line) => {
    const lower = line.toLowerCase();
    let level = "info";
    if (lower.includes(" error") || lower.includes("[error]")) level = "error";
    else if (lower.includes(" warn") || lower.includes("[warn]")) level = "warn";
    else if (lower.includes(" debug") || lower.includes("[debug]")) level = "debug";
    return { raw: line, level };
  })
);

const filtered = computed(() => {
  if (filterLevel.value === "all") return parsedLogs.value;
  return parsedLogs.value.filter((l) => l.level === filterLevel.value);
});

async function fetchLogs() {
  try {
    logs.value = await invoke<string[]>("cmd_get_logs");
  } catch {
    // ignore
  }
}

const copySuccess = ref(false);

async function scrollToBottom() {
  await nextTick();
  if (autoScroll.value && logContainer.value) {
    logContainer.value.scrollTop = logContainer.value.scrollHeight;
  }
}

async function copyAllLogs() {
  const text = logs.value.join("\n");
  try {
    await navigator.clipboard.writeText(text);
    copySuccess.value = true;
    setTimeout(() => (copySuccess.value = false), 1500);
  } catch {
    // fallback: create a temporary textarea
    const el = document.createElement("textarea");
    el.value = text;
    document.body.appendChild(el);
    el.select();
    document.execCommand("copy");
    document.body.removeChild(el);
    copySuccess.value = true;
    setTimeout(() => (copySuccess.value = false), 1500);
  }
}

watch(filtered, scrollToBottom);

onMounted(() => {
  fetchLogs();
  pollTimer = setInterval(fetchLogs, 1000);
});
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">运行日志</h1>
      <div class="header-actions">
        <div class="level-tabs">
          <button
            v-for="level in ['all', 'error', 'warn', 'info', 'debug']"
            :key="level"
            class="level-tab"
            :class="{ active: filterLevel === level }"
            @click="filterLevel = level"
          >
            {{ level === 'all' ? '全部' : level.toUpperCase() }}
          </button>
        </div>
        <button class="btn btn-ghost" title="自动滚动" @click="autoScroll = !autoScroll">
          <ArrowDown :size="14" :style="{ opacity: autoScroll ? 1 : 0.4 }" />
          自动滚动
        </button>
        <button class="btn btn-ghost" @click="copyAllLogs" :title="copySuccess ? '已复制!' : '复制全部日志'">
          <Copy :size="14" :style="{ color: copySuccess ? '#107c10' : undefined }" />
          {{ copySuccess ? '已复制' : '复制' }}
        </button>
        <button class="btn btn-ghost" @click="logs = []" title="清空日志">
          <Trash2 :size="14" />
        </button>
      </div>
    </div>

    <div class="log-container card" ref="logContainer">
      <div v-if="filtered.length === 0" class="log-empty">
        暂无日志，启动代理后将显示运行日志
      </div>
      <div
        v-for="(log, i) in filtered"
        :key="i"
        class="log-line"
        :style="{ color: levelColors[log.level] ?? 'inherit' }"
      >
        {{ log.raw }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 14px; height: calc(100vh - 40px - 48px); }
.page-header { display: flex; align-items: center; justify-content: space-between; flex-shrink: 0; }
.page-title { font-size: 20px; font-weight: 600; }
.header-actions { display: flex; align-items: center; gap: 8px; }

.level-tabs { display: flex; gap: 4px; }
.level-tab {
  padding: 3px 10px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 11px; font-weight: 500; cursor: pointer; transition: all 0.15s;
}
.level-tab:hover { background: rgba(128,128,128,0.1); }
.level-tab.active { background: var(--color-primary); color: white; border-color: transparent; }

.log-container {
  flex: 1;
  overflow-y: auto;
  padding: 12px 16px;
  background: #1a1a1a;
  border-radius: var(--radius-lg);
  font-family: 'Cascadia Code', 'Consolas', 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.6;
}
.log-line {
  padding: 1px 0;
  white-space: pre-wrap;
  word-break: break-all;
}
.log-line:hover { background: rgba(255,255,255,0.04); }
.log-empty {
  color: #616161;
  text-align: center;
  padding: 48px;
  font-family: 'Segoe UI', sans-serif;
}
</style>
