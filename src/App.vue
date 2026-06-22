<script setup lang="ts">
import { onMounted, watch } from "vue";
import { RouterView } from "vue-router";
import Sidebar from "./components/Sidebar.vue";
import Titlebar from "./components/Titlebar.vue";
import { useAppStore } from "./stores/app";

const store = useAppStore();

function applyTheme(theme: string) {
  const html = document.documentElement;
  if (theme === "dark") {
    html.setAttribute("data-theme", "dark");
  } else if (theme === "light") {
    html.setAttribute("data-theme", "light");
  } else {
    html.removeAttribute("data-theme");
  }
}

watch(() => store.config.theme, applyTheme, { immediate: true });

onMounted(async () => {
  await store.init();
  // Re-apply after config loads from backend
  applyTheme(store.config.theme);
});
</script>

<template>
  <div class="app-shell">
    <Titlebar />
    <div class="app-body">
      <Sidebar />
      <main class="app-content">
        <RouterView v-slot="{ Component }">
          <Transition name="page" mode="out-in">
            <component :is="Component" :key="$route.path" />
          </Transition>
        </RouterView>
      </main>
    </div>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
}
.app-body {
  display: flex;
  flex: 1;
  overflow: hidden;
}
.app-content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 24px;
  background: var(--color-bg);
}
</style>
