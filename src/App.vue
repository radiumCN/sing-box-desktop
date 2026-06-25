<script setup lang="ts">
import { onMounted, watch } from "vue";
import { RouterView } from "vue-router";
import Sidebar from "./components/Sidebar.vue";
import { useAppStore } from "./stores/app";
import { setLocale } from "./i18n";

const store = useAppStore();

// Keep the active UI locale in sync with the persisted AppConfig.language.
watch(() => store.config.language, (lang) => setLocale(lang), { immediate: true });

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
    <Sidebar />
    <main class="app-content">
      <RouterView v-slot="{ Component }">
        <Transition name="page" mode="out-in">
          <component :is="Component" :key="$route.path" />
        </Transition>
      </RouterView>
    </main>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  height: 100vh;
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
