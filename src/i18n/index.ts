import { createI18n } from "vue-i18n";
import zhCN from "./locales/zh-CN.ts";
import en from "./locales/en.ts";

export type AppLocale = "zh-CN" | "en";

// Initial locale: persisted choice (so the very first paint is correct, before the
// backend AppConfig has loaded), falling back to Chinese. The store re-syncs this to
// AppConfig.language once config is fetched (see App.vue).
function initialLocale(): AppLocale {
  const saved = localStorage.getItem("skylark-locale");
  return saved === "en" || saved === "zh-CN" ? saved : "zh-CN";
}

export const i18n = createI18n({
  legacy: false,
  locale: initialLocale(),
  fallbackLocale: "zh-CN",
  messages: { "zh-CN": zhCN, en },
});

/** Switch the active UI locale and persist it for the next launch. Unknown values are
 * ignored so a stray config string can never blank the UI. */
export function setLocale(locale: string) {
  if (locale !== "en" && locale !== "zh-CN") return;
  i18n.global.locale.value = locale;
  localStorage.setItem("skylark-locale", locale);
}
