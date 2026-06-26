import { createI18n } from "vue-i18n";
import zhCN from "./locales/zh-CN.ts";
import en from "./locales/en.ts";

export type AppLocale = "zh-CN" | "en";

// Best-guess locale from the OS, mirroring the backend's fresh-install detection in
// `config.rs::detect_system_language` (Chinese system → zh-CN, otherwise English). Used
// when there is no persisted choice yet so the first paint matches what the backend will
// report, avoiding a zh→en flicker on the first launch on a non-Chinese system.
export function detectLocale(): AppLocale {
  return navigator.language?.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

// Initial locale: persisted choice (so the very first paint is correct, before the
// backend AppConfig has loaded), falling back to the detected system locale. The store
// re-syncs this to AppConfig.language once config is fetched (see App.vue).
function initialLocale(): AppLocale {
  const saved = localStorage.getItem("skylark-locale");
  return saved === "en" || saved === "zh-CN" ? saved : detectLocale();
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
