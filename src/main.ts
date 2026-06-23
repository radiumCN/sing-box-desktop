import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";
import "./styles/main.css";

// Disable right-click context menu
document.addEventListener("contextmenu", (e) => e.preventDefault());

// Block all browser F-key shortcuts (F1=help, F3=search, F5=refresh, F7=caret, F12=devtools …)
document.addEventListener("keydown", (e) => {
  if (e.key.match(/^F\d+$/)) {
    e.preventDefault();
    e.stopPropagation();
  }
}, true);

const app = createApp(App);
app.use(createPinia());
app.use(router);
app.mount("#app");
