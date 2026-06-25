<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, computed } from "vue";
import {
  Shield, Globe, Cpu, Monitor, Download,
  RefreshCw, CheckCircle, AlertCircle, Package, ExternalLink,
  ShieldCheck, ShieldAlert, Check, Rocket, Zap, Save, Upload, Archive
} from "@lucide/vue";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { useAppStore, type AppConfig } from "../stores/app";

const store = useAppStore();
const saved = ref(false);
const appVersion = ref("");
const localConfig = ref<AppConfig>({ ...store.config });

// Platform detection (WinTun is a Windows-only requirement).
const isWindows = /win/i.test(navigator.userAgent);
const isMacOS = /mac/i.test(navigator.userAgent);
const kernelBinaryName = isWindows ? "sing-box.exe" : "sing-box";

let saveTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    await store.saveConfig({ ...localConfig.value });
    saved.value = true;
    setTimeout(() => (saved.value = false), 1500);
  }, 600);
}

watch(localConfig, scheduleSave, { deep: true });

// ─── Config backup / restore ──────────────────────────────────────────

const exportingConfig = ref(false);
const importingConfig = ref(false);
const showImportModal = ref(false);
const importText = ref("");

/** Export the full setup to a JSON file and reveal it in the file manager. */
async function exportConfig() {
  exportingConfig.value = true;
  try {
    const path = await invoke<string>("cmd_export_config");
    try {
      await revealItemInDir(path);
    } catch {
      /** Reveal may be unavailable; the file is still written. */
    }
    alert(`配置已导出到：\n${path}`);
  } catch (e) {
    alert(`导出失败：${e}`);
  } finally {
    exportingConfig.value = false;
  }
}

/** Restore from pasted backup JSON, then refresh all in-memory store state. */
async function importConfig() {
  const content = importText.value.trim();
  if (!content) {
    alert("请粘贴备份文件的内容");
    return;
  }
  if (!confirm("导入将覆盖当前的订阅、节点、分组、规则与设置，确定继续？")) return;
  importingConfig.value = true;
  try {
    await invoke("cmd_import_config", { content });
    await store.fetchConfig();
    localConfig.value = { ...store.config };
    await Promise.all([
      store.fetchSubscriptions(),
      store.fetchNodes(),
      store.fetchProxyGroups(),
    ]);
    showImportModal.value = false;
    importText.value = "";
    alert("配置已导入。新的路由 / DNS 设置将在下次启动代理时生效。");
  } catch (e) {
    alert(`导入失败：${e}`);
  } finally {
    importingConfig.value = false;
  }
}

// ─── Kernel Updater ───────────────────────────────────────────────────

interface ReleaseInfo {
  version: string;
  published_at: string;
  release_notes: string;
  download_url: string;
}

// ─── TUN / Admin ─────────────────────────────────────────────────────

const isAdmin = ref(false);
const wintunAvailable = ref(false);
const downloadingWintun = ref(false);
const wintunError = ref("");

async function refreshTunStatus() {
  [isAdmin.value, wintunAvailable.value] = await Promise.all([
    invoke<boolean>("cmd_is_elevated"),
    invoke<boolean>("cmd_wintun_available"),
  ]);
}

async function requestAdmin() {
  try {
    await invoke("cmd_relaunch_as_admin");
    // Process exits and re-opens elevated; if we reach here, user cancelled
  } catch (e) {
    // UAC cancelled - ignore
  }
}

async function downloadWintun() {
  downloadingWintun.value = true;
  wintunError.value = "";
  try {
    await invoke("cmd_download_wintun");
    wintunAvailable.value = true;
  } catch (e) {
    wintunError.value = String(e);
  } finally {
    downloadingWintun.value = false;
  }
}

// ─── Kernel Updater ───────────────────────────────────────────────────

const kernelExists = ref(false);
const installedVersion = ref<string | null>(null);
const latestRelease = ref<ReleaseInfo | null>(null);
const checkingUpdate = ref(false);
const checkError = ref("");
const downloading = ref(false);
const downloadProgress = ref(0);
const downloadedBytes = ref(0);
const totalBytes = ref(0);
const downloadError = ref("");
const downloadDone = ref(false);

const hasUpdate = computed(() => {
  if (!latestRelease.value || !installedVersion.value) return false;
  const installed = installedVersion.value.match(/(\d+\.\d+\.\d+)/)?.[1] ?? "";
  const latest = latestRelease.value.version.replace(/^v/, "");
  return installed !== latest && installed !== "";
});

const formatBytes = (b: number) => {
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / 1024 / 1024).toFixed(1)} MB`;
};

const formatDate = (iso: string) => {
  if (!iso) return "";
  return new Date(iso).toLocaleDateString("zh-CN", {
    year: "numeric", month: "long", day: "numeric",
  });
};

async function refreshKernelStatus() {
  [kernelExists.value, installedVersion.value] = await Promise.all([
    invoke<boolean>("cmd_singbox_exists"),
    invoke<string | null>("cmd_get_installed_version"),
  ]);
}

async function checkUpdate(forceRefresh = false) {
  checkingUpdate.value = true;
  checkError.value = "";
  try {
    latestRelease.value = await invoke<ReleaseInfo>("cmd_check_singbox_update", { forceRefresh });
  } catch (e) {
    const msg = String(e);
    if (msg.includes("频率超限") || msg.includes("rate limit") || msg.includes("Rate limit")) {
      checkError.value = msg;
    } else {
      checkError.value = msg;
    }
  } finally {
    checkingUpdate.value = false;
  }
}

async function startDownload() {
  if (!latestRelease.value) return;
  downloading.value = true;
  downloadProgress.value = 0;
  downloadError.value = "";
  downloadDone.value = false;

  // Listen for progress events
  const unlistenProgress = await listen<{
    percent: number; downloaded: number; total: number;
  }>("singbox-download-progress", (event) => {
    downloadProgress.value = event.payload.percent;
    downloadedBytes.value = event.payload.downloaded;
    totalBytes.value = event.payload.total;
  });

  const unlistenDone = await listen<{ success: boolean; message: string }>(
    "singbox-download-done",
    async (event) => {
      if (event.payload.success) {
        downloadDone.value = true;
        downloadProgress.value = 100;
        await refreshKernelStatus();
      }
      unlistenProgress();
      unlistenDone();
    }
  );

  try {
    await invoke("cmd_download_singbox", {
      downloadUrl: latestRelease.value.download_url,
    });
  } catch (e) {
    downloadError.value = String(e);
    unlistenProgress();
    unlistenDone();
  } finally {
    downloading.value = false;
  }
}

// ─── App Self-Updater ─────────────────────────────────────────────────

interface AppReleaseInfo {
  version: string;
  published_at: string;
  release_notes: string;
  download_url: string;
  is_prerelease: boolean;
}

const appLatestRelease = ref<AppReleaseInfo | null>(null);
const checkingAppUpdate = ref(false);
const appCheckError = ref("");
const downloadingApp = ref(false);
const appDownloadProgress = ref(0);
const appDownloadedBytes = ref(0);
const appTotalBytes = ref(0);
const appDownloadDone = ref(false);
const appDownloadError = ref("");

const hasAppUpdate = computed(() => {
  if (!appLatestRelease.value || !appVersion.value) return false;
  const installed = appVersion.value.replace(/^v/, "");
  const latest = appLatestRelease.value.version.replace(/^v/, "");
  return installed !== latest && installed !== "";
});

async function checkAppUpdate(forceRefresh = false) {
  checkingAppUpdate.value = true;
  appCheckError.value = "";
  appLatestRelease.value = null;
  try {
    appLatestRelease.value = await invoke<AppReleaseInfo>("cmd_check_app_update", {
      channel: localConfig.value.update_channel ?? "stable",
      forceRefresh,
    });
  } catch (e) {
    appCheckError.value = String(e);
  } finally {
    checkingAppUpdate.value = false;
  }
}

async function startAppDownload() {
  if (!appLatestRelease.value) return;
  downloadingApp.value = true;
  appDownloadProgress.value = 0;
  appDownloadError.value = "";
  appDownloadDone.value = false;

  const unlistenProgress = await listen<{ percent: number; downloaded: number; total: number }>(
    "app-download-progress",
    (event) => {
      appDownloadProgress.value = event.payload.percent;
      appDownloadedBytes.value = event.payload.downloaded;
      appTotalBytes.value = event.payload.total;
    }
  );

  const unlistenDone = await listen<{ success: boolean; message: string }>(
    "app-download-done",
    (event) => {
      if (event.payload.success) {
        appDownloadDone.value = true;
        appDownloadProgress.value = 100;
      }
      unlistenProgress();
      unlistenDone();
    }
  );

  try {
    await invoke("cmd_download_app_update", {
      downloadUrl: appLatestRelease.value.download_url,
    });
  } catch (e) {
    appDownloadError.value = String(e);
    unlistenProgress();
    unlistenDone();
  } finally {
    downloadingApp.value = false;
  }
}

let unlistenAppUpdate: (() => void) | null = null;

onMounted(async () => {
  await Promise.all([
    refreshKernelStatus(),
    refreshTunStatus(),
    getVersion().then((v) => (appVersion.value = v)),
  ]);

  // Silently check for app update in background (uses 1-hour cache, won't spam API).
  checkAppUpdate(false);

  // Also listen for the background checker's event (fired ~45s after launch).
  unlistenAppUpdate = await listen<{
    version: string;
    download_url: string;
    release_notes: string;
    published_at: string;
    is_prerelease: boolean;
    current_version: string;
  }>("app-update-available", (event) => {
    appLatestRelease.value = {
      version: event.payload.version,
      published_at: event.payload.published_at,
      release_notes: event.payload.release_notes,
      download_url: event.payload.download_url,
      is_prerelease: event.payload.is_prerelease,
    };
  });
});

onUnmounted(() => {
  unlistenAppUpdate?.();
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">设置</h1>
      <Transition name="autosave">
        <span v-if="saved" class="autosave-badge">
          <Check :size="12" />已保存
        </span>
      </Transition>
    </div>

    <!-- ─── 应用更新 ─── -->
    <section class="settings-section">
      <div class="section-header">
        <Rocket :size="15" />
        <span>应用更新</span>
      </div>
      <div class="card settings-card kernel-card">

        <!-- Current app version + channel -->
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">当前版本</div>
            <div class="setting-desc">v{{ appVersion }}</div>
          </div>
          <div class="channel-select-wrap">
            <span class="channel-label">更新通道</span>
            <select class="input select-input" v-model="localConfig.update_channel" style="width:110px">
              <option value="stable">稳定版</option>
              <option value="beta">测试版</option>
            </select>
          </div>
        </div>

        <!-- Latest release info -->
        <div v-if="appLatestRelease" class="release-info">
          <div class="release-header">
            <span class="release-tag">{{ appLatestRelease.version }}</span>
            <span class="release-date">{{ formatDate(appLatestRelease.published_at) }}</span>
            <span v-if="appLatestRelease.is_prerelease" class="badge badge-purple">预览版</span>
            <span v-if="hasAppUpdate" class="badge badge-yellow">有新版本</span>
            <span v-else class="badge badge-green">已是最新</span>
          </div>
          <div v-if="appLatestRelease.release_notes" class="release-notes">
            {{ appLatestRelease.release_notes }}
          </div>
        </div>

        <div class="setting-divider" />

        <!-- Action buttons -->
        <div class="kernel-actions">
          <button
            class="btn btn-ghost"
            :disabled="checkingAppUpdate"
            @click="() => checkAppUpdate(true)"
          >
            <RefreshCw :size="13" :class="{ spin: checkingAppUpdate }" />
            {{ checkingAppUpdate ? "检查中..." : "检查更新" }}
          </button>

          <button
            v-if="appLatestRelease && hasAppUpdate"
            class="btn btn-primary"
            :disabled="downloadingApp"
            @click="startAppDownload"
          >
            <Download :size="13" :class="{ spin: downloadingApp }" />
            {{ downloadingApp ? "下载中..." : "下载并安装" }}
          </button>

          <a
            class="btn btn-ghost"
            href="https://github.com/radiumCN/skylark/releases"
            target="_blank"
          >
            <ExternalLink :size="13" />
            查看所有版本
          </a>
        </div>

        <!-- Download progress -->
        <div v-if="downloadingApp || appDownloadDone" class="download-progress-area">
          <div class="progress-info">
            <span>{{ appDownloadDone ? "下载完成 ✓ 安装程序即将启动" : `下载中 ${appDownloadProgress.toFixed(1)}%` }}</span>
            <span v-if="!appDownloadDone && appTotalBytes > 0" class="progress-bytes">
              {{ formatBytes(appDownloadedBytes) }} / {{ formatBytes(appTotalBytes) }}
            </span>
          </div>
          <div class="progress-bar-track">
            <div
              class="progress-bar-fill"
              :style="{ width: `${appDownloadProgress}%` }"
              :class="{ done: appDownloadDone }"
            />
          </div>
        </div>

        <!-- Errors -->
        <div v-if="appCheckError || appDownloadError" class="kernel-error">
          <AlertCircle :size="13" />
          {{ appCheckError || appDownloadError }}
        </div>

        <div class="kernel-hint">
          更新通道：<strong>稳定版</strong> 发布经过测试的正式版本，<strong>测试版</strong> 包含最新功能但可能存在已知问题。
          下载完成后将自动启动安装程序完成升级。
        </div>
      </div>
    </section>

    <!-- ─── sing-box 内核管理 ─── -->
    <section class="settings-section">
      <div class="section-header">
        <Package :size="15" />
        <span>sing-box 内核管理</span>
      </div>
      <div class="card settings-card kernel-card">

        <!-- Current status -->
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">当前内核版本</div>
            <div class="setting-desc">
              <template v-if="installedVersion">{{ installedVersion }}</template>
              <span v-else class="not-installed">未安装 — 需要下载才能使用代理功能</span>
            </div>
          </div>
          <div class="kernel-status">
            <span v-if="kernelExists" class="badge badge-green">
              <CheckCircle :size="11" /> 已安装
            </span>
            <span v-else class="badge badge-red">
              <AlertCircle :size="11" /> 未安装
            </span>
          </div>
        </div>

        <!-- Latest release info -->
        <div v-if="latestRelease" class="release-info">
          <div class="release-header">
            <span class="release-tag">{{ latestRelease.version }}</span>
            <span class="release-date">{{ formatDate(latestRelease.published_at) }}</span>
            <span v-if="hasUpdate" class="badge badge-yellow">有更新</span>
            <span v-else-if="kernelExists" class="badge badge-green">已是最新</span>
          </div>
          <div v-if="latestRelease.release_notes" class="release-notes">
            {{ latestRelease.release_notes }}
          </div>
        </div>

        <div class="setting-divider" />

        <!-- Action buttons -->
        <div class="kernel-actions">
          <button
            class="btn btn-ghost"
            :disabled="checkingUpdate"
            @click="() => checkUpdate()"
          >
            <RefreshCw :size="13" :class="{ spin: checkingUpdate }" />
            {{ checkingUpdate ? "检查中..." : "检查更新" }}
          </button>

          <button
            v-if="latestRelease && (!kernelExists || hasUpdate)"
            class="btn btn-primary"
            :disabled="downloading"
            @click="startDownload"
          >
            <Download :size="13" :class="{ spin: downloading }" />
            {{ downloading ? "下载中..." : kernelExists ? "更新内核" : "下载安装" }}
          </button>

          <a
            class="btn btn-ghost"
            href="https://github.com/SagerNet/sing-box/releases"
            target="_blank"
            title="在 GitHub 查看所有版本"
          >
            <ExternalLink :size="13" />
            GitHub Releases
          </a>
        </div>

        <!-- Download progress -->
        <div v-if="downloading || downloadDone" class="download-progress-area">
          <div class="progress-info">
            <span>{{ downloadDone ? "下载完成 ✓" : `下载中 ${downloadProgress.toFixed(1)}%` }}</span>
            <span v-if="!downloadDone && totalBytes > 0" class="progress-bytes">
              {{ formatBytes(downloadedBytes) }} / {{ formatBytes(totalBytes) }}
            </span>
          </div>
          <div class="progress-bar-track">
            <div
              class="progress-bar-fill"
              :style="{ width: `${downloadProgress}%` }"
              :class="{ done: downloadDone }"
            />
          </div>
        </div>

        <!-- Errors -->
        <div v-if="checkError || downloadError" class="kernel-error">
          <AlertCircle :size="13" />
          {{ checkError || downloadError }}
        </div>

        <div class="kernel-hint">
          sing-box 内核保存在应用数据目录中，下载完成后无需重启即可使用。
          如需手动安装，也可将 <code>{{ kernelBinaryName }}</code> 放入应用数据目录的 <code>bin/</code> 子目录。
        </div>
      </div>
    </section>

    <!-- System Behavior -->
    <section class="settings-section">
      <div class="section-header">
        <Monitor :size="15" />
        <span>系统行为</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">开机自启动</div>
            <div class="setting-desc">Windows 登录时自动启动 Skylark</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.startup_with_system" />
            <span class="toggle-track" />
          </label>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">启动时最小化</div>
            <div class="setting-desc">启动后自动最小化到系统托盘</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.startup_minimized" />
            <span class="toggle-track" />
          </label>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">允许局域网访问</div>
            <div class="setting-desc">允许局域网内其他设备使用此代理</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.allow_lan" />
            <span class="toggle-track" />
          </label>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">关闭按钮行为</div>
            <div class="setting-desc">
              {{ localConfig.close_to_tray ? "点击关闭按钮最小化到系统托盘" : "点击关闭按钮退出应用" }}
            </div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.close_to_tray" />
            <span class="toggle-track" />
          </label>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">记住代理状态</div>
            <div class="setting-desc">开机自启动时自动恢复上次的代理状态（系统代理 / TUN 模式）</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.restore_proxy_on_startup" />
            <span class="toggle-track" />
          </label>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">日志写入文件</div>
            <div class="setting-desc">将内核日志持续写入 logs/skylark-日期.log，崩溃后仍可查看；下次启动代理时生效</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.log_to_file" />
            <span class="toggle-track" />
          </label>
        </div>
      </div>
    </section>

    <!-- Ports -->
    <section class="settings-section">
      <div class="section-header">
        <Globe :size="15" />
        <span>端口配置</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">HTTP 代理端口</div>
            <div class="setting-desc">HTTP/HTTPS 代理监听端口</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.http_port" min="1024" max="65535" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">SOCKS5 端口</div>
            <div class="setting-desc">SOCKS5 代理监听端口</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.socks_port" min="1024" max="65535" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">混合端口</div>
            <div class="setting-desc">HTTP+SOCKS5 混合代理端口（推荐）</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.mixed_port" min="1024" max="65535" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">API 端口</div>
            <div class="setting-desc">Clash 兼容 API 监听端口</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.api_port" min="1024" max="65535" />
        </div>
      </div>
    </section>

    <!-- DNS / 网络 -->
    <!-- Subscription -->
    <section class="settings-section">
      <div class="section-header">
        <RefreshCw :size="15" />
        <span>订阅</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">订阅 User-Agent</div>
            <div class="setting-desc">
              拉取订阅时发送的客户端标识。部分机场按 UA 返回不同内容——旧版 Clash 标识可能只返回「请更换客户端」占位节点。
              常用：<code>v2rayN/6.45</code>、<code>clash-verge/v2.0.0</code>、<code>ClashMetaForAndroid/2.10</code>、<code>sing-box/1.12</code>。留空恢复默认。
            </div>
          </div>
          <input
            class="input"
            type="text"
            v-model.trim="localConfig.subscription_user_agent"
            placeholder="v2rayN/6.45"
            style="width:220px"
          />
        </div>
      </div>
    </section>

    <section class="settings-section">
      <div class="section-header">
        <Globe :size="15" />
        <span>DNS 与网络</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">国内 DNS 解析器</div>
            <div class="setting-desc">直连域名使用；支持 IP（如 223.5.5.5）、DoH（https://…）、DoT（tls://…）</div>
          </div>
          <input class="input" type="text" v-model.trim="localConfig.dns_local" placeholder="223.5.5.5" style="width:220px" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">启用 IPv6</div>
            <div class="setting-desc">开启后双栈解析（优先 IPv4）并接管 IPv6 流量；默认关闭为纯 IPv4</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.enable_ipv6" />
            <span class="toggle-track" />
          </label>
        </div>
      </div>
    </section>

    <!-- Config backup / restore -->
    <section class="settings-section">
      <div class="section-header">
        <Archive :size="15" />
        <span>配置备份</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">导出配置</div>
            <div class="setting-desc">将订阅、节点、分组、路由规则与全部设置打包为一个 JSON 文件（不含 API 密钥）</div>
          </div>
          <button class="btn btn-ghost btn-sm" :disabled="exportingConfig" @click="exportConfig">
            <Save :size="12" />
            {{ exportingConfig ? "导出中..." : "导出" }}
          </button>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">导入配置</div>
            <div class="setting-desc">从备份文件恢复，将覆盖当前的订阅、节点、分组、规则与设置</div>
          </div>
          <button class="btn btn-ghost btn-sm" :disabled="importingConfig" @click="showImportModal = true">
            <Upload :size="12" />
            导入
          </button>
        </div>
      </div>
    </section>

    <!-- TUN Mode -->
    <section class="settings-section">
      <div class="section-header">
        <Shield :size="15" />
        <span>TUN 模式</span>
      </div>
      <div class="card settings-card">
        <!-- Enable toggle -->
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">启用 TUN 模式</div>
            <div class="setting-desc">接管全局流量，包括不支持代理的应用。需要管理员权限。</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.tun_enabled" />
            <span class="toggle-track" />
          </label>
        </div>

        <!-- TUN requirements (show when enabled) -->
        <div v-if="localConfig.tun_enabled" class="tun-checklist">
          <!-- Admin status -->
          <div class="tun-check-row">
            <div class="check-status">
              <ShieldCheck v-if="isAdmin" :size="15" class="check-ok" />
              <ShieldAlert v-else :size="15" class="check-bad" />
            </div>
            <div class="check-info">
              <div class="check-label">管理员权限</div>
              <div class="check-desc">
                <span v-if="isAdmin" class="text-ok">当前以管理员身份运行</span>
                <span v-else class="text-bad">当前无管理员权限，TUN 模式无法启动</span>
              </div>
            </div>
            <button v-if="!isAdmin" class="btn btn-primary btn-sm" @click="requestAdmin">
              <ShieldCheck :size="12" />
              以管理员重启
            </button>
          </div>

          <!-- WinTun driver (Windows only; macOS/Linux use the kernel TUN device) -->
          <template v-if="isWindows">
            <div class="tun-divider" />
            <div class="tun-check-row">
              <div class="check-status">
                <CheckCircle v-if="wintunAvailable" :size="15" class="check-ok" />
                <AlertCircle v-else :size="15" class="check-bad" />
              </div>
              <div class="check-info">
                <div class="check-label">WinTun 驱动</div>
                <div class="check-desc">
                  <span v-if="wintunAvailable" class="text-ok">wintun.dll 已就绪</span>
                  <span v-else class="text-bad">未找到 wintun.dll，需要下载</span>
                </div>
              </div>
              <button
                v-if="!wintunAvailable"
                class="btn btn-ghost btn-sm"
                :disabled="downloadingWintun"
                @click="downloadWintun"
              >
                <Download :size="12" :class="{ spin: downloadingWintun }" />
                {{ downloadingWintun ? "下载中..." : "下载 WinTun" }}
              </button>
            </div>
            <div v-if="wintunError" class="tun-error">{{ wintunError }}</div>
          </template>

          <!-- macOS TUN note -->
          <template v-else-if="isMacOS">
            <div class="tun-divider" />
            <div class="tun-check-row">
              <div class="check-info">
                <div class="check-label">macOS TUN</div>
                <div class="check-desc">
                  <span class="text-ok">无需额外驱动；启用 TUN 时将请求一次 root 授权。</span>
                </div>
              </div>
            </div>
          </template>
        </div>
      </div>
    </section>

    <!-- Auto-select (URLTest) -->
    <section class="settings-section">
      <div class="section-header">
        <Zap :size="15" />
        <span>自动选优（URLTest）</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">测速 URL</div>
            <div class="setting-desc">用于探测各节点延迟的地址，建议使用返回 204 的轻量端点</div>
          </div>
          <input
            class="input"
            type="text"
            v-model="localConfig.auto_test_url"
            placeholder="https://www.gstatic.com/generate_204"
            style="width: 260px"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">检测间隔（分钟）</div>
            <div class="setting-desc">内核每隔多久重新测速并切换到更快的节点</div>
          </div>
          <input
            class="input port-input"
            type="number"
            v-model.number="localConfig.auto_test_interval"
            min="1" max="1440"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">容差（毫秒）</div>
            <div class="setting-desc">仅当新节点比当前节点快出此差值才切换，避免在相近节点间频繁抖动</div>
          </div>
          <input
            class="input port-input"
            type="number"
            v-model.number="localConfig.auto_tolerance"
            min="0" max="2000"
          />
        </div>
        <div class="kernel-hint">
          修改后需重新启动代理才会生效。该设置同时作用于「全部节点」与各订阅的自动选优组。
        </div>
      </div>
    </section>

    <!-- Advanced -->
    <section class="settings-section">
      <div class="section-header">
        <Cpu :size="15" />
        <span>高级设置</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">日志级别</div>
            <div class="setting-desc">sing-box 核心日志详细程度</div>
          </div>
          <select class="input select-input" v-model="localConfig.log_level">
            <option value="trace">Trace（最详细）</option>
            <option value="debug">Debug</option>
            <option value="info">Info（推荐）</option>
            <option value="warn">Warn</option>
            <option value="error">Error（最少）</option>
          </select>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">主题</div>
            <div class="setting-desc">界面颜色主题</div>
          </div>
          <select class="input select-input" v-model="localConfig.theme">
            <option value="system">跟随系统</option>
            <option value="light">浅色</option>
            <option value="dark">深色</option>
          </select>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">自动检查内核更新</div>
            <div class="setting-desc">启动后定期检查 sing-box 新版本</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.auto_update_notify" />
            <span class="toggle-track" />
          </label>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">检查间隔（小时）</div>
            <div class="setting-desc">0 表示仅启动时检查一次</div>
          </div>
          <input
            class="input port-input"
            type="number"
            v-model.number="localConfig.auto_update_interval"
            min="0" max="168"
            :disabled="!localConfig.auto_update_notify"
          />
        </div>
      </div>
    </section>

    <!-- About -->
    <div class="card about-card">
      <div class="about-logo">
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none">
          <circle cx="12" cy="12" r="10" stroke="var(--color-primary)" stroke-width="2"/>
          <path d="M8 12h8M12 8l4 4-4 4" stroke="var(--color-primary)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </div>
      <div>
        <div class="about-name">Skylark</div>
        <div class="about-desc">基于 sing-box 内核的跨平台图形化代理客户端</div>
        <div class="about-version">v{{ appVersion }} · {{ installedVersion ?? "sing-box 未安装" }}</div>
      </div>
    </div>

    <!-- Import config modal -->
    <div v-if="showImportModal" class="dialog-overlay" @click.self="showImportModal = false">
      <div class="dialog-card">
        <div class="dialog-title">导入配置</div>
        <div class="dialog-hint">粘贴此前导出的备份文件（skylark-config-*.json）的完整内容。导入会覆盖现有数据。</div>
        <textarea
          class="input import-area"
          v-model="importText"
          rows="8"
          placeholder='粘贴备份 JSON，例如 {"format":"skylark-config",...}'
        />
        <div class="dialog-actions">
          <button class="btn btn-ghost" @click="showImportModal = false">取消</button>
          <button class="btn btn-primary" :disabled="importingConfig" @click="importConfig">
            <RefreshCw v-if="importingConfig" :size="13" class="spin" />
            {{ importingConfig ? "导入中..." : "导入并覆盖" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 20px; max-width: 700px; }
.page-header { display: flex; align-items: center; justify-content: space-between; }
.page-title { font-size: 20px; font-weight: 600; }

.autosave-badge {
  display: inline-flex; align-items: center; gap: 5px;
  font-size: 12px; font-weight: 500;
  color: #107c10; padding: 4px 10px;
  background: rgba(16,124,16,0.08);
  border-radius: 100px;
}
.autosave-enter-active, .autosave-leave-active { transition: opacity 0.3s, transform 0.3s; }
.autosave-enter-from, .autosave-leave-to { opacity: 0; transform: translateY(-4px); }

.settings-section { display: flex; flex-direction: column; gap: 10px; }
.section-header {
  display: flex; align-items: center; gap: 7px;
  font-size: 12px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.5px; padding: 0 4px;
}
.settings-card { padding: 0; overflow: hidden; }
.setting-row {
  display: flex; align-items: center; justify-content: space-between;
  gap: 16px; padding: 14px 18px;
}
.setting-info { flex: 1; }
.setting-label { font-size: 13px; font-weight: 500; margin-bottom: 2px; }
.setting-desc { font-size: 11px; color: var(--color-text-muted); }
.not-installed { color: var(--color-error); }
.setting-divider { height: 1px; background: var(--color-border); margin: 0 18px; }

/* Toggle Switch */
.toggle { position: relative; display: inline-block; width: 42px; height: 24px; flex-shrink: 0; }
.toggle input { opacity: 0; width: 0; height: 0; }
.toggle-track {
  position: absolute; inset: 0;
  background: rgba(128,128,128,0.3); border-radius: 12px;
  cursor: pointer; transition: background 0.2s;
}
.toggle-track::before {
  content: '';
  position: absolute; left: 3px; top: 3px;
  width: 18px; height: 18px; border-radius: 50%;
  background: white; transition: transform 0.2s;
  box-shadow: 0 1px 3px rgba(0,0,0,0.3);
}
.toggle input:checked + .toggle-track { background: var(--color-primary); }
.toggle input:checked + .toggle-track::before { transform: translateX(18px); }

.port-input { width: 100px; text-align: right; }
.select-input { width: 160px; cursor: pointer; }

/* ─── Kernel Card ─── */
.kernel-card {}
.kernel-status { flex-shrink: 0; }

.release-info {
  padding: 10px 18px 12px;
  background: rgba(0, 120, 212, 0.04);
  border-top: 1px solid var(--color-border);
  border-bottom: 1px solid var(--color-border);
}
.release-header {
  display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 6px;
}
.release-tag {
  font-size: 13px; font-weight: 700;
  color: var(--color-primary);
  font-family: 'Cascadia Code', monospace;
}
.release-date { font-size: 11px; color: var(--color-text-muted); }
.release-notes {
  font-size: 11px; color: var(--color-text-secondary);
  white-space: pre-line; line-height: 1.5;
  max-height: 80px; overflow-y: auto;
}

.kernel-actions {
  display: flex; align-items: center; gap: 8px; padding: 12px 18px; flex-wrap: wrap;
}

.download-progress-area {
  padding: 0 18px 14px;
  display: flex; flex-direction: column; gap: 6px;
}
.progress-info {
  display: flex; justify-content: space-between; align-items: center;
  font-size: 12px; color: var(--color-text-secondary);
}
.progress-bytes { font-family: 'Cascadia Code', monospace; font-size: 11px; }
.progress-bar-track {
  height: 6px; background: rgba(128,128,128,0.15);
  border-radius: 3px; overflow: hidden;
}
.progress-bar-fill {
  height: 100%; background: var(--color-primary);
  border-radius: 3px;
  transition: width 0.3s ease;
}
.progress-bar-fill.done { background: var(--color-success); }

.kernel-error {
  display: flex; align-items: flex-start; gap: 6px;
  margin: 0 18px 12px;
  padding: 10px 12px;
  background: rgba(209,52,56,0.06);
  border: 1px solid rgba(209,52,56,0.2);
  border-radius: var(--radius-md);
  font-size: 12px; color: var(--color-error); line-height: 1.4;
}
.kernel-hint {
  padding: 0 18px 14px;
  font-size: 11px; color: var(--color-text-muted); line-height: 1.5;
}
.kernel-hint code {
  font-family: 'Cascadia Code', monospace;
  background: rgba(128,128,128,0.12);
  padding: 1px 4px; border-radius: 3px;
}

.badge-purple {
  display: inline-flex; align-items: center; gap: 4px;
  font-size: 11px; font-weight: 500; padding: 2px 8px;
  border-radius: 100px;
  background: rgba(100, 65, 165, 0.12); color: #6441a5;
}

.channel-select-wrap {
  display: flex; align-items: center; gap: 8px; flex-shrink: 0;
}
.channel-label { font-size: 11px; color: var(--color-text-muted); white-space: nowrap; }

/* TUN checklist */
.tun-checklist {
  padding: 12px 18px 14px;
  border-top: 1px solid var(--color-border);
  display: flex; flex-direction: column; gap: 0;
  background: rgba(0,0,0,0.02);
}
.tun-check-row {
  display: flex; align-items: center; gap: 10px; padding: 8px 0;
}
.tun-divider { height: 1px; background: var(--color-border); margin: 2px 0; }
.check-status { flex-shrink: 0; }
.check-ok { color: #107c10; }
.check-bad { color: #d13438; }
.check-info { flex: 1; }
.check-label { font-size: 13px; font-weight: 500; margin-bottom: 1px; }
.check-desc { font-size: 11px; }
.text-ok { color: #107c10; }
.text-bad { color: #d13438; }
.btn-sm { padding: 4px 10px !important; font-size: 12px !important; flex-shrink: 0; }
.tun-error {
  font-size: 11px; color: var(--color-error);
  padding: 6px 8px; background: rgba(209,52,56,0.06);
  border-radius: var(--radius-sm); margin-top: 4px;
}

.about-card { padding: 20px; display: flex; align-items: center; gap: 16px; }
.about-name { font-size: 15px; font-weight: 600; margin-bottom: 3px; }
.about-desc { font-size: 12px; color: var(--color-text-secondary); margin-bottom: 3px; }
.about-version { font-size: 11px; color: var(--color-text-muted); }

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }

/* Import config dialog */
.dialog-overlay {
  position: fixed; inset: 0; z-index: 50;
  display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,0.4); backdrop-filter: blur(4px);
}
.dialog-card {
  width: min(560px, 92vw);
  background: var(--color-bg); border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 10px); padding: 20px;
  display: flex; flex-direction: column; gap: 12px;
  box-shadow: 0 16px 48px rgba(0,0,0,0.28);
}
.dialog-title { font-size: 15px; font-weight: 600; }
.dialog-hint { font-size: 12px; color: var(--color-text-secondary); line-height: 1.5; }
.import-area { width: 100%; resize: vertical; font-family: var(--font-mono, monospace); font-size: 12px; }
.dialog-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
</style>
