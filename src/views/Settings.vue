<script setup lang="ts">
import { ref, watch, onMounted, computed } from "vue";
import {
  Shield, Globe, Cpu, Monitor, Download,
  RefreshCw, CheckCircle, AlertCircle, Package, ExternalLink,
  ShieldCheck, ShieldAlert, Check
} from "@lucide/vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { useAppStore, type AppConfig } from "../stores/app";

const store = useAppStore();
const saved = ref(false);
const appVersion = ref("");
const localConfig = ref<AppConfig>({ ...store.config });

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

onMounted(async () => {
  await Promise.all([
    refreshKernelStatus(),
    refreshTunStatus(),
    getVersion().then((v) => (appVersion.value = v)),
  ]);
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
          如需手动安装，也可将 <code>sing-box.exe</code> 放入应用数据目录的 <code>bin/</code> 子目录。
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
            <div class="setting-desc">Windows 登录时自动启动 sing-box-win</div>
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

          <div class="tun-divider" />

          <!-- WinTun driver -->
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
        <div class="about-name">sing-box-win</div>
        <div class="about-desc">基于 sing-box 的 Windows 图形化管理工具</div>
        <div class="about-version">v{{ appVersion }} · {{ installedVersion ?? "sing-box 未安装" }}</div>
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
</style>
