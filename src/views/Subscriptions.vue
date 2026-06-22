<script setup lang="ts">
import { ref } from "vue";
import {
  Plus, RefreshCw, Trash2, ExternalLink, Clock, Server, AlertCircle
} from "@lucide/vue";
import { useAppStore } from "../stores/app";

const store = useAppStore();

const showAddDialog = ref(false);
const newSubName = ref("");
const newSubUrl = ref("");
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
  if (!newSubName.value.trim() || !newSubUrl.value.trim()) {
    addError.value = "请填写名称和订阅链接";
    return;
  }
  addLoading.value = true;
  addError.value = "";
  try {
    await store.addSubscription(newSubName.value.trim(), newSubUrl.value.trim());
    showAddDialog.value = false;
    newSubName.value = "";
    newSubUrl.value = "";
  } catch (e) {
    addError.value = String(e);
  } finally {
    addLoading.value = false;
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
  newSubName.value = "";
  newSubUrl.value = "";
  addError.value = "";
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
      <div class="form-group">
        <label class="form-label">订阅名称</label>
        <input
          class="input"
          v-model="newSubName"
          placeholder="例如：我的机场"
          @keyup.enter="addSub"
        />
      </div>
      <div class="form-group">
        <label class="form-label">订阅链接</label>
        <input
          class="input"
          v-model="newSubUrl"
          placeholder="https://... (支持 Clash / V2Ray / SIP008 格式)"
          @keyup.enter="addSub"
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
          {{ addLoading ? "获取中..." : "添加" }}
        </button>
      </div>
    </div>

    <!-- Subscription List -->
    <div class="sub-list">
      <div v-for="sub in store.subscriptions" :key="sub.id" class="card sub-item">
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
          </div>
          <div class="sub-url">{{ sub.url }}</div>
        </div>
        <div class="sub-actions">
          <button
            class="btn btn-ghost icon-btn"
            :disabled="updatingId === sub.id"
            title="更新订阅"
            @click="updateSub(sub.id)"
          >
            <RefreshCw :size="14" :class="{ spin: updatingId === sub.id }" />
          </button>
          <button
            class="btn btn-ghost icon-btn"
            title="打开链接"
            @click="() => {}"
          >
            <ExternalLink :size="14" />
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
    </div>

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
          <span>Base64 编码节点链接，vmess:// vless:// ss:// trojan:// hy2://</span>
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
.form-error {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; color: var(--color-error);
}
.dialog-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 4px; }

.sub-list { display: flex; flex-direction: column; gap: 10px; }
.sub-item {
  padding: 16px 18px;
  display: flex; align-items: flex-start; justify-content: space-between; gap: 12px;
  transition: box-shadow 0.15s;
}
.sub-item:hover { box-shadow: var(--shadow-md); }
.sub-left { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
.sub-name { font-size: 14px; font-weight: 600; }
.sub-meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.meta-item {
  display: flex; align-items: center; gap: 3px;
  font-size: 11px; color: var(--color-text-secondary);
}
.sub-url {
  font-size: 11px; color: var(--color-text-muted);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 500px;
}
.sub-actions { display: flex; gap: 4px; flex-shrink: 0; }
.icon-btn { padding: 6px !important; }
.icon-btn.danger:hover { color: var(--color-error); }

.hint-card { padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; }
.hint-title { font-size: 12px; font-weight: 600; color: var(--color-text-secondary); text-transform: uppercase; letter-spacing: 0.5px; }
.hint-list { display: flex; flex-direction: column; gap: 8px; }
.hint-item { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--color-text-secondary); }

@keyframes spin { to { transform: rotate(360deg); } }
.spin { animation: spin 0.8s linear infinite; }
</style>
