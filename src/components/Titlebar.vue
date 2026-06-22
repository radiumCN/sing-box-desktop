<script setup lang="ts">
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X } from "@lucide/vue";
import { useAppStore } from "../stores/app";

const appWindow = getCurrentWindow();
const store = useAppStore();

function onDragStart(e: MouseEvent) {
  if (e.button !== 0) return;
  appWindow.startDragging();
}

function onClose() {
  if (store.config.close_to_tray) {
    appWindow.hide();
  } else {
    appWindow.close();
  }
}
</script>

<template>
  <header class="titlebar" data-tauri-drag-region @mousedown="onDragStart">
    <div class="titlebar-left" data-tauri-drag-region>
      <div class="app-icon">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
          <circle cx="12" cy="12" r="10" stroke="var(--color-primary)" stroke-width="2"/>
          <path d="M8 12h8M12 8l4 4-4 4" stroke="var(--color-primary)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </div>
      <span class="app-title" data-tauri-drag-region>sing-box</span>
    </div>
    <div class="titlebar-controls" @mousedown.stop>
      <button class="ctrl-btn" @click="appWindow.minimize()" title="最小化">
        <Minus :size="12" />
      </button>
      <button
        class="ctrl-btn ctrl-close"
        @click="onClose"
        :title="store.config.close_to_tray ? '最小化到托盘' : '退出应用'"
      >
        <X :size="12" />
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  height: var(--titlebar-height);
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 12px 0 0;
  background: var(--color-surface);
  border-bottom: 1px solid var(--color-border);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  flex-shrink: 0;
  user-select: none;
}
.titlebar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 16px;
  height: 100%;
}
.app-icon {
  display: flex;
  align-items: center;
}
.app-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  letter-spacing: 0.2px;
}
.titlebar-controls {
  display: flex;
  align-items: center;
}
.ctrl-btn {
  width: 46px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: background 0.1s;
}
.ctrl-btn:hover { background: rgba(128,128,128,0.15); }
.ctrl-btn:active { background: rgba(128,128,128,0.25); }
.ctrl-close:hover { background: #c42b1c; color: white; }
.ctrl-close:active { background: #b72518; color: white; }
</style>
