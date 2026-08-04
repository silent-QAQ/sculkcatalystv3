<script setup lang="ts">
import { computed } from 'vue'
import { Check, ShieldCheck } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import { LANGUAGES, REVIEW_MODES } from '../types'
import type { AiSettingsView, ReviewMode } from '../types'
import { aiSettings, flash, friendly, saveUi, uiSettings } from '../store'

const reviewMode = computed<ReviewMode>(() => aiSettings.value?.review_mode ?? 'approval')
const codexFullAccessAvailable = computed(() => aiSettings.value?.codex_full_access_available === true)
const codexFullAccessReadyCount = computed(() => aiSettings.value?.codex_full_access_ready_agent_ids.length ?? 0)

async function setLanguage(language: string) {
  await saveUi({ language }, 'UI 语言已更新，将同步影响 AI 回复语言')
}
async function setReviewMode(mode: ReviewMode) {
  if (mode === reviewMode.value) return
  try {
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/review-mode', { method: 'PUT', body: JSON.stringify({ mode }) })
    flash('审核模式已切换为「' + (REVIEW_MODES.find(item => item.key === mode)?.label ?? mode) + '」')
  } catch (error) { flash('切换失败：' + friendly(error)) }
}
</script>

<template>
  <div class="s-group">
    <h2>权限</h2>
    <p class="desc">决定 Sculk 受管任务的审批策略。原生 Codex 在“请求批准”和“替我审核”模式下保持只读；选择“完全访问权限”并完成本机授权后，才会以不受沙盒限制的方式运行。</p>
    <div class="s-card">
      <div v-for="mode in REVIEW_MODES" :key="mode.key" class="s-row">
        <p><b>{{ mode.label }}</b><small>{{ mode.hint }}</small></p>
        <span v-if="reviewMode===mode.key" class="s-test ok" style="display:flex;align-items:center;gap:4px"><Check style="width:11px"/>当前模式</span>
        <button class="s-switch" :class="{on:reviewMode===mode.key}" @click="setReviewMode(mode.key)"><i/></button>
      </div>
    </div>
    <p v-if="reviewMode==='full'" class="s-error" style="display:flex;align-items:center;gap:5px;margin-top:8px">
      <ShieldCheck style="width:12px"/>完全访问权限已开启：Sculk 受管任务不再等待审批，会显著增加数据丢失或意外行为的风险。
    </p>
    <p v-if="reviewMode==='full'&&codexFullAccessReadyCount>0" class="s-test ok" style="display:flex;align-items:center;gap:5px;margin-top:8px">
      <ShieldCheck style="width:12px"/>Codex 完整权限已就绪：已授权的 Codex CLI 将以运行后端的本机账户获得不受沙盒限制的文件和命令访问；当前工作区仅为初始目录。
    </p>
    <p v-else-if="reviewMode==='full'&&codexFullAccessAvailable" class="s-error" style="display:flex;align-items:center;gap:5px;margin-top:8px">
      <ShieldCheck style="width:12px"/>Codex 完整权限总闸已就绪，但当前没有受信任的 Codex CLI。请重新接入检测到的 Codex，或将启动命令改为已授权的绝对路径。
    </p>
    <p v-else-if="reviewMode==='full'" class="s-error" style="display:flex;align-items:center;gap:5px;margin-top:8px">
      <ShieldCheck style="width:12px"/>Codex 完整权限尚未配置。请以本机回环地址重启服务，并设置 SCULK_ALLOW_CODEX_FULL=true 与 SCULK_CODEX_TRUSTED_COMMAND。
    </p>
  </div>

  <div class="s-group">
    <h2>常规</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>语言</b><small>应用 UI 语言，同时作为 AI 对话的默认回复语言</small></p>
        <select class="s-select" :value="uiSettings?.language ?? 'auto'" @change="setLanguage(($event.target as HTMLSelectElement).value)">
          <option v-for="language in LANGUAGES" :key="language.key" :value="language.key">{{ language.label }}</option>
        </select>
      </div>
    </div>
  </div>
</template>
