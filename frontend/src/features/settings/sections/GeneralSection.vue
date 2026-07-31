<script setup lang="ts">
import { computed } from 'vue'
import { Check, ShieldCheck } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import { LANGUAGES, REVIEW_MODES } from '../types'
import type { AiSettingsView, ReviewMode } from '../types'
import { aiSettings, flash, friendly, saveUi, uiSettings } from '../store'

const reviewMode = computed<ReviewMode>(() => aiSettings.value?.review_mode ?? 'approval')

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
    <p class="desc">决定 Sculk Agent 执行任务时的审批策略。默认情况下，AI 可以读取和编辑其工作区中的文件；涉及停服、玩家数据与正式部署的操作按下列模式审批。</p>
    <div class="s-card">
      <div v-for="mode in REVIEW_MODES" :key="mode.key" class="s-row">
        <p><b>{{ mode.label }}</b><small>{{ mode.hint }}</small></p>
        <span v-if="reviewMode===mode.key" class="s-test ok" style="display:flex;align-items:center;gap:4px"><Check style="width:11px"/>当前模式</span>
        <button class="s-switch" :class="{on:reviewMode===mode.key}" @click="setReviewMode(mode.key)"><i/></button>
      </div>
    </div>
    <p v-if="reviewMode==='full'" class="s-error" style="display:flex;align-items:center;gap:5px;margin-top:8px">
      <ShieldCheck style="width:12px"/>完全访问权限已开启：所有任务自动执行，会显著增加数据丢失或意外行为的风险。
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
