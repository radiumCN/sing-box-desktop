<script setup lang="ts">
import { ref, nextTick, onMounted } from "vue";
import {
  Plus, RefreshCw, Trash2, QrCode, Clock, Server, AlertCircle, X, Copy, Check as CheckIcon, Zap
} from "@lucide/vue";
import QRCode from "qrcode";
import { useAppStore } from "../stores/app";

const store = useAppStore();

onMounted(() => {
  // Shared poller keeps the active auto group's current node fresh for the badge below.
  store.ensureActiveNowPoller();
});

const showAddDialog = ref(false);
const addMode = ref<"url" | "text">("url");
const newSubName = ref("");
const newSubUrl = ref("");
const newSubContent = ref("");
const addLoading = ref(false);
const addError = ref("");
const updatingId = ref<string | null>(null);

const formatDate = (iso?: string) => {
  if (!iso) return "从未";
  const d = new Date(iso);
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  if (diff < 60000) return "刚刚";
  if (diff < 3600000) return `${Math.floor(diff / 60000)} 分钟前`;
  if (diff < 86400000) return `${Math.floor(diff / 3600000)} 小时前`;
  return d.toLocaleDateString();
};

const subTypeLabel = (type: string) => {
  const map: Record<string, string> = { clash: "Clash", v2ray: "V2Ray", sip008: "SIP008", unknown: "未知" };
  return map[type] ?? type;
};

async function addSub() {
  const name = newSubName.value.trim();
  if (!name) {
    addError.value = "请填写名称";
    return;
  }
  if (addMode.value === "url" && !newSubUrl.value.trim()) {
    addError.value = "请填写订阅链接";
    return;
  }
  if (addMode.value === "text" && !newSubContent.value.trim()) {
    addError.value = "请粘贴节点内容";
    return;
  }
  addLoading.value = true;
  addError.value = "";
  try {
    if (addMode.value === "url") {
      await store.addSubscription(name, newSubUrl.value.trim());
    } else {
      await store.importSubscriptionFromText(name, newSubContent.value.trim());
    }
    cancelAdd();
  } catch (e) {
    addError.value = String(e);
  } finally {
    addLoading.value = false;
  }
}

async function pasteContent() {
  try {
    newSubContent.value = await navigator.clipboard.readText();
  } catch {
    // Clipboard read may be blocked — user can paste manually.
  }
}

async function updateSub(id: string) {
  updatingId.value = id;
  try {
    await store.updateSubscription(id);
  } finally {
    updatingId.value = null;
  }
}

async function deleteSub(id: string, name: string) {
  if (confirm(`确认删除订阅「${name}」？`)) {
    await store.deleteSubscription(id);
  }
}

const refreshingAll = ref(false);
async function refreshAll() {
  if (store.subscriptions.length === 0) return;
  refreshingAll.value = true;
  try {
    await Promise.allSettled(store.subscriptions.map((s) => store.updateSubscription(s.id)));
  } finally {
    refreshingAll.value = false;
  }
}

function cancelAdd() {
  showAddDialog.value = false;
  addMode.value = "url";
  newSubName.value = "";
  newSubUrl.value = "";
  newSubContent.value = "";
  addError.value = "";
}

// ─── QR Code share dialog ──────────────────────────────────────────
const qrVisible = ref(false);
const qrSubName = ref("");
const qrSubUrl = ref("");
const qrDataUrl = ref("");
const qrCopied = ref(false);

async function showQr(name: string, url: string) {
  qrSubName.value = name;
  qrSubUrl.value = url;
  qrVisible.value = true;
  await nextTick();
  try {
    qrDataUrl.value = await QRCode.toDataURL(url, {
      width: 240,
      margin: 2,
      color: { dark: "#1a1a1a", light: "#ffffff" },
      errorCorrectionLevel: "M",
    });
  } catch {
    qrDataUrl.value = "";
  }
}

function closeQr() {
  qrVisible.value = false;
  qrCopied.value = false;
  qrDataUrl.value = "";
}

async function copyUrl() {
  try {
    await navigator.clipboard.writeText(qrSubUrl.value);
    qrCopied.value = true;
    setTimeout(() => (qrCopied.value = false), 1800);
  } catch {
    // Fallback: show the URL selected
  }
}

const INTERVAL_OPTIONS = [
  { value: 1,  label: "每 1 小时" },
  { value: 3,  label: "每 3 小时" },
  { value: 6,  label: "每 6 小时" },
  { value: 12, label: "每 12 小时" },
  { value: 24, label: "每 24 小时" },
  { value: 72, label: "每 3 天" },
];

function formatNextUpdate(sub: { last_update?: string; update_interval: number }) {
  if (!sub.last_update) return "尚未更新";
  const next = new Date(sub.last_update).getTime() + sub.update_interval * 3600 * 1000;
  const diff = next - Date.now();
  if (diff <= 0) return "即将更新";
  const h = Math.floor(diff / 3600000);
  const m = Math.floor((diff % 3600000) / 60000);
  return h > 0 ? `${h}h ${m}m 后` : `${m}m 后`;
}

async function toggleAutoUpdate(id: string, currentAutoUpdate: boolean, interval: number) {
  await store.saveSubscriptionSettings(id, !currentAutoUpdate, interval);
}

async function changeInterval(id: string, autoUpdate: boolean, interval: number) {
  await store.saveSubscriptionSettings(id, autoUpdate, interval);
}
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">订阅管理</h1>
      <div style="display:flex;gap:8px;">
        <button
          v-if="store.subscriptions.length > 0"
          class="btn btn-ghost"
          :disabled="refreshingAll"
          @click="refreshAll"
        >
          <RefreshCw :size="13" :class="{ spin: refreshingAll }" />
          {{ refreshingAll ? "更新中..." : "全部更新" }}
        </button>
        <button class="btn btn-primary" @click="showAddDialog = true">
          <Plus :size="14" />
          添加订阅
        </button>
      </div>
    </div>

    <!-- Empty State -->
    <div v-if="store.subscriptions.length === 0 && !showAddDialog" class="empty-state">
      <div class="empty-icon">
        <Server :size="36" />
      </div>
      <div class="empty-title">暂无订阅</div>
      <div class="empty-desc">添加 Clash 或 V2Ray 订阅链接开始使用</div>
      <button class="btn btn-primary" @click="showAddDialog = true">
        <Plus :size="14" />
        添加第一个订阅
      </button>
    </div>

    <!-- Add Dialog (inline) -->
    <div v-if="showAddDialog" class="card add-dialog">
      <div class="dialog-title">添加订阅</div>
      <div class="mode-tabs">
        <button class="mode-tab" :class="{ active: addMode === 'url' }" @click="addMode = 'url'">
          链接订阅
        </button>
        <button class="mode-tab" :class="{ active: addMode === 'text' }" @click="addMode = 'text'">
          手动导入
        </button>
      </div>
      <div class="form-group">
        <label class="form-label">订阅名称</label>
        <input
          class="input"
          v-model="newSubName"
          placeholder="例如：我的机场"
          @keyup.enter="addSub"
        />
      </div>
      <div v-if="addMode === 'url'" class="form-group">
        <label class="form-label">订阅链接</label>
        <input
          class="input"
          v-model="newSubUrl"
          placeholder="https://... (支持 Clash / V2Ray / SIP008 格式)"
          @keyup.enter="addSub"
        />
      </div>
      <div v-else class="form-group">
        <div class="form-label-row">
          <label class="form-label">节点内容</label>
          <button class="btn btn-ghost paste-btn" @click="pasteContent">
            <Copy :size="12" />
            从剪贴板粘贴
          </button>
        </div>
        <textarea
          class="input content-area"
          v-model="newSubContent"
          rows="6"
          placeholder="粘贴节点链接 / Base64 / Clash YAML / SIP008 内容&#10;支持 vmess:// vless:// ss:// trojan:// hy2://（每行一个）"
        />
      </div>
      <div v-if="addError" class="form-error">
        <AlertCircle :size="13" />
        {{ addError }}
      </div>
      <div class="dialog-actions">
        <button class="btn btn-ghost" @click="cancelAdd">取消</button>
        <button class="btn btn-primary" :disabled="addLoading" @click="addSub">
          <RefreshCw v-if="addLoading" :size="13" class="spin" />
          {{ addLoading ? (addMode === "url" ? "获取中..." : "导入中...") : (addMode === "url" ? "添加" : "导入") }}
        </button>
      </div>
    </div>

    <!-- Subscription List -->
    <div class="sub-list">
      <div v-for="sub in store.subscriptions" :key="sub.id" class="card sub-item">
        <div class="sub-main">
          <div class="sub-left">
            <div class="sub-name">{{ sub.name }}</div>
            <div class="sub-meta">
              <span class="badge badge-blue">{{ subTypeLabel(sub.sub_type) }}</span>
              <span class="meta-item">
                <Server :size="11" />
                {{ sub.node_count }} 个节点
              </span>
              <span class="meta-item">
                <Clock :size="11" />
                {{ formatDate(sub.last_update) }}
              </span>
              <span
                v-if="store.activeProxyTag === `auto-${sub.id}`"
                class="meta-item auto-hit"
                title="本订阅自动选优组当前命中的节点"
              >
                <Zap :size="11" />
                自动 → {{ store.activeNodeNow ?? "选择中…" }}
              </span>
            </div>
            <div class="sub-url">{{ sub.url || "本地导入" }}</div>
          </div>
          <div class="sub-actions">
            <button
              v-if="sub.url"
              class="btn btn-ghost icon-btn"
              :disabled="updatingId === sub.id"
              title="更新订阅"
              @click="updateSub(sub.id)"
            >
              <RefreshCw :size="14" :class="{ spin: updatingId === sub.id }" />
            </button>
            <button
              v-if="sub.url"
              class="btn btn-ghost icon-btn"
              title="二维码分享"
              @click="showQr(sub.name, sub.url)"
            >
              <QrCode :size="14" />
            </button>
            <button
              class="btn btn-ghost icon-btn danger"
              title="删除"
              @click="deleteSub(sub.id, sub.name)"
            >
              <Trash2 :size="14" />
            </button>
          </div>
        </div>

        <!-- Auto-update row (URL subscriptions only) -->
        <div v-if="sub.url" class="sub-autoupdate">
          <div class="autoupdate-left">
            <Clock :size="12" class="autoupdate-icon" />
            <span class="autoupdate-label">自动更新</span>
            <span v-if="sub.auto_update" class="autoupdate-next">
              {{ formatNextUpdate(sub) }}
            </span>
          </div>
          <div class="autoupdate-right">
            <select
              v-if="sub.auto_update"
              class="interval-select"
              :value="sub.update_interval"
              @change="changeInterval(sub.id, sub.auto_update, Number(($event.target as HTMLSelectElement).value))"
            >
              <option v-for="opt in INTERVAL_OPTIONS" :key="opt.value" :value="opt.value">
                {{ opt.label }}
              </option>
            </select>
            <button
              class="mini-toggle"
              :class="{ on: sub.auto_update }"
              :title="sub.auto_update ? '关闭自动更新' : '开启自动更新'"
              @click="toggleAutoUpdate(sub.id, sub.auto_update, sub.update_interval)"
            >
              <span class="mini-knob" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- QR Code dialog -->
    <Teleport to="body">
      <Transition name="qr-fade">
        <div v-if="qrVisible" class="qr-overlay" @click.self="closeQr">
          <div class="qr-dialog">
            <div class="qr-header">
              <div class="qr-title">
                <QrCode :size="15" />
                {{ qrSubName }}
              </div>
              <button class="qr-close" @click="closeQr">
                <X :size="16" />
              </button>
            </div>

            <div class="qr-body">
              <div class="qr-image-wrap">
                <img v-if="qrDataUrl" :src="qrDataUrl" alt="QR Code" class="qr-image" />
                <div v-else class="qr-placeholder">生成中...</div>
              </div>

              <div class="qr-desc">用手机扫描二维码导入订阅</div>

              <div class="qr-url-row">
                <span class="qr-url-text">{{ qrSubUrl }}</span>
                <button class="btn btn-ghost qr-copy-btn" @click="copyUrl" :title="qrCopied ? '已复制' : '复制链接'">
                  <CheckIcon v-if="qrCopied" :size="13" class="copy-ok" />
                  <Copy v-else :size="13" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- Supported formats hint -->
    <div class="hint-card card">
      <div class="hint-title">支持的订阅格式</div>
      <div class="hint-list">
        <div class="hint-item">
          <span class="badge badge-blue">Clash</span>
          <span>YAML 格式，支持 ss/vmess/vless/trojan/hysteria2</span>
        </div>
        <div class="hint-item">
          <span class="badge badge-green">V2Ray</span>
          <span>Base64 编码节点链接，vmess:// vless:// ss:// trojan:// hy2:// tuic:// anytls://</span>
        </div>
        <div class="hint-item">
          <span class="badge badge-yellow">SIP008</span>
          <span>Shadowsocks 标准订阅格式 (JSON)</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 16px; max-width: 800px; }
.page-header { display: flex; align-items: center; justify-content: space-between; }
.page-title { font-size: 20px; font-weight: 600; }

.empty-state {
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 12px; padding: 60px 24px; text-align: center;
  color: var(--color-text-muted);
}
.empty-icon { opacity: 0.4; }
.empty-title { font-size: 16px; font-weight: 600; color: var(--color-text-secondary); }
.empty-desc { font-size: 13px; }

.add-dialog { padding: 20px; display: flex; flex-direction: column; gap: 14px; }
.dialog-title { font-size: 15px; font-weight: 600; }
.form-group { display: flex; flex-direction: column; gap: 6px; }
.form-label { font-size: 12px; font-weight: 500; color: var(--color-text-secondary); }
.form-label-row { display: flex; align-items: center; justify-content: space-between; }
.paste-btn { padding: 2px 8px !important; font-size: 11px; }
.content-area {
  resize: vertical; min-height: 110px;
  font-family: 'Cascadia Code', monospace; font-size: 12px; line-height: 1.5;
}

.mode-tabs {
  display: flex; gap: 4px; padding: 3px;
  background: rgba(128,128,128,0.08); border-radius: var(--radius-md);
}
.mode-tab {
  flex: 1; padding: 6px 12px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--color-text-secondary);
  font-size: 12px; font-weight: 500; cursor: pointer; transition: all 0.15s;
}
.mode-tab.active {
  background: var(--color-surface); color: var(--color-text);
  box-shadow: 0 1px 2px rgba(0,0,0,0.08);
}
.form-error {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; color: var(--color-error);
}
.dialog-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 4px; }

.sub-list { display: flex; flex-direction: column; gap: 10px; }
.sub-item {
  padding: 0;
  display: flex; flex-direction: column;
  transition: box-shadow 0.15s; overflow: hidden;
}
.sub-item:hover { box-shadow: var(--shadow-md); }
.sub-main {
  display: flex; align-items: flex-start; justify-content: space-between;
  gap: 12px; padding: 16px 18px;
}
.sub-left { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
.sub-name { font-size: 14px; font-weight: 600; }
.sub-meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.meta-item {
  display: flex; align-items: center; gap: 3px;
  font-size: 11px; color: var(--color-text-secondary);
}
.meta-item.auto-hit {
  color: #f0c040; font-weight: 500;
  background: rgba(240,192,64,0.12); padding: 1px 7px; border-radius: 10px;
}
.sub-url {
  font-size: 11px; color: var(--color-text-muted);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 500px;
}
.sub-actions { display: flex; gap: 4px; flex-shrink: 0; }
.icon-btn { padding: 6px !important; }
.icon-btn.danger:hover { color: var(--color-error); }

/* Auto-update row */
.sub-autoupdate {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 18px;
  background: rgba(128,128,128,0.04);
  border-top: 1px solid var(--color-border);
}
.autoupdate-left {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; color: var(--color-text-secondary);
}
.autoupdate-icon { color: var(--color-text-muted); flex-shrink: 0; }
.autoupdate-label { font-weight: 500; }
.autoupdate-next {
  font-size: 11px; color: var(--color-text-muted);
  background: rgba(128,128,128,0.1); padding: 1px 7px; border-radius: 10px;
}
.autoupdate-right { display: flex; align-items: center; gap: 8px; }

.interval-select {
  font-size: 12px; padding: 2px 6px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border); background: var(--color-surface);
  color: var(--color-text); cursor: pointer; outline: none;
}
.interval-select:focus { border-color: var(--color-primary); }

.mini-toggle {
  width: 34px; height: 20px; border-radius: 10px;
  background: rgba(128,128,128,0.3); border: none; cursor: pointer;
  position: relative; transition: background 0.2s; padding: 0; flex-shrink: 0;
}
.mini-toggle.on { background: var(--color-primary); }
.mini-knob {
  position: absolute; top: 2px; left: 2px;
  width: 16px; height: 16px; border-radius: 50%;
  background: white; transition: transform 0.2s; display: block;
  box-shadow: 0 1px 2px rgba(0,0,0,0.2);
}
.mini-toggle.on .mini-knob { transform: translateX(14px); }

.hint-card { padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; }
.hint-title { font-size: 12px; font-weight: 600; color: var(--color-text-secondary); text-transform: uppercase; letter-spacing: 0.5px; }
.hint-list { display: flex; flex-direction: column; gap: 8px; }
.hint-item { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--color-text-secondary); }

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }

/* ─── QR Code dialog ─── */
.qr-overlay {
  position: fixed; inset: 0; z-index: 1000;
  background: rgba(0,0,0,0.45);
  display: flex; align-items: center; justify-content: center;
}
.qr-dialog {
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: 0 20px 60px rgba(0,0,0,0.25);
  width: 320px;
  overflow: hidden;
}
.qr-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 16px 12px;
  border-bottom: 1px solid var(--color-border);
}
.qr-title {
  display: flex; align-items: center; gap: 7px;
  font-size: 14px; font-weight: 600;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  max-width: 240px;
}
.qr-close {
  background: none; border: none; cursor: pointer;
  color: var(--color-text-muted); padding: 4px; border-radius: var(--radius-sm);
  display: flex; align-items: center;
  transition: color 0.15s, background 0.15s;
}
.qr-close:hover { color: var(--color-text); background: rgba(128,128,128,0.12); }

.qr-body {
  padding: 20px 24px 20px;
  display: flex; flex-direction: column; align-items: center; gap: 14px;
}
.qr-image-wrap {
  background: #fff;
  border-radius: var(--radius-md);
  padding: 8px;
  box-shadow: 0 2px 8px rgba(0,0,0,0.1);
}
.qr-image { display: block; width: 200px; height: 200px; }
.qr-placeholder {
  width: 200px; height: 200px;
  display: flex; align-items: center; justify-content: center;
  font-size: 12px; color: var(--color-text-muted);
}
.qr-desc {
  font-size: 12px; color: var(--color-text-muted); text-align: center;
}
.qr-url-row {
  display: flex; align-items: center; gap: 6px; width: 100%;
  background: rgba(128,128,128,0.06); border: 1px solid var(--color-border);
  border-radius: var(--radius-md); padding: 6px 8px 6px 10px;
}
.qr-url-text {
  flex: 1; min-width: 0;
  font-size: 11px; font-family: 'Cascadia Code', monospace;
  color: var(--color-text-secondary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.qr-copy-btn { padding: 4px 6px !important; flex-shrink: 0; }
.copy-ok { color: #107c10; }

/* QR dialog enter/leave transition */
.qr-fade-enter-active, .qr-fade-leave-active { transition: opacity 0.18s, transform 0.18s; }
.qr-fade-enter-from, .qr-fade-leave-to { opacity: 0; transform: scale(0.95); }
</style>
