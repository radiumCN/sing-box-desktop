<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  Home,
  Rss,
  Server,
  Activity,
  ScrollText,
  Filter,
  Settings,
  Power,
} from "@lucide/vue";
import { listen } from "@tauri-apps/api/event";
import { useAppStore } from "../stores/app";

const route = useRoute();
const router = useRouter();
const store = useAppStore();

const hasUpdate = ref(false);
const updateVersion = ref("");

const navItems = [
  { path: "/home", icon: Home, label: "仪表盘" },
  { path: "/subscriptions", icon: Rss, label: "订阅" },
  { path: "/nodes", icon: Server, label: "节点" },
  { path: "/connections", icon: Activity, label: "连接" },
  { path: "/logs", icon: ScrollText, label: "日志" },
  { path: "/rules", icon: Filter, label: "分流规则" },
];

const isActive = (path: string) => route.path === path;

const statusColor = computed(() =>
  store.status.running ? "#107c10" : "#9e9e9e"
);

onMounted(() => {
  listen<{ version: string }>("singbox-update-available", (e) => {
    hasUpdate.value = true;
    updateVersion.value = e.payload.version;
  });
});
</script>

<template>
  <nav class="sidebar">
    <div class="sidebar-nav">
      <button
        v-for="item in navItems"
        :key="item.path"
        class="nav-item"
        :class="{ active: isActive(item.path) }"
        @click="router.push(item.path)"
      >
        <component :is="item.icon" :size="18" />
        <span>{{ item.label }}</span>
      </button>
    </div>

    <div class="sidebar-footer">
      <!-- Quick proxy toggle -->
      <button
        class="proxy-toggle"
        :class="store.status.running ? 'running' : 'stopped'"
        :disabled="store.loading"
        @click="store.status.running ? store.stopProxy() : store.startProxy()"
        :title="store.status.running ? '停止代理' : '启动代理'"
      >
        <div class="toggle-indicator" :style="{ background: statusColor }" />
        <span>{{ store.status.running ? "运行中" : "已停止" }}</span>
        <Power :size="14" class="toggle-icon" />
      </button>

      <button
        class="nav-item"
        :class="{ active: isActive('/settings') }"
        :title="hasUpdate ? `sing-box ${updateVersion} 可更新` : ''"
        @click="router.push('/settings')"
      >
        <div class="icon-wrap">
          <Settings :size="18" />
          <span v-if="hasUpdate" class="update-dot" />
        </div>
        <span>设置</span>
        <span v-if="hasUpdate" class="update-badge">更新</span>
      </button>
    </div>
  </nav>
</template>

<style scoped>
.sidebar {
  width: var(--sidebar-width);
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  backdrop-filter: blur(20px) saturate(180%);
  -webkit-backdrop-filter: blur(20px) saturate(180%);
  flex-shrink: 0;
  padding: 8px 8px 12px;
}
.sidebar-nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border-radius: var(--radius-md);
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  text-align: left;
  width: 100%;
  transition: all 0.15s ease;
}
.nav-item:hover {
  background: rgba(128, 128, 128, 0.1);
  color: var(--color-text);
}
.nav-item.active {
  background: rgba(0, 120, 212, 0.12);
  color: var(--color-primary);
}
.sidebar-footer {
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-top: 1px solid var(--color-border);
  padding-top: 8px;
}
.proxy-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: var(--radius-md);
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  width: 100%;
  transition: background 0.15s;
}
.proxy-toggle:hover { background: rgba(128,128,128,0.1); }
.proxy-toggle:disabled { opacity: 0.6; cursor: wait; }
.proxy-toggle.running { color: #107c10; }
.toggle-indicator {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
  transition: background 0.3s;
}
.toggle-icon {
  margin-left: auto;
  opacity: 0.6;
}
.icon-wrap { position: relative; display: flex; align-items: center; }
.update-dot {
  position: absolute; top: -3px; right: -3px;
  width: 7px; height: 7px; border-radius: 50%;
  background: #d13438;
  border: 1.5px solid var(--color-surface);
  animation: pulse-dot 2s infinite;
}
.update-badge {
  margin-left: auto;
  font-size: 10px; font-weight: 700;
  padding: 1px 6px; border-radius: 100px;
  background: rgba(209,52,56,0.12); color: #d13438;
}
@keyframes pulse-dot {
  0%, 100% { transform: scale(1); }
  50% { transform: scale(1.3); }
}
</style>
