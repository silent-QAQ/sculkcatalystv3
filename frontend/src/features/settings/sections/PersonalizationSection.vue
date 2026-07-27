<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check } from 'lucide-vue-next'
import { CHAT_STYLES } from '../types'
import { saveUi, uiSettings } from '../store'

const PRESET_KEYS = CHAT_STYLES.filter(style => style.key !== 'custom').map(style => style.key)

const selected = ref('default')
const customStyle = ref('')
const extraContext = ref('')

watch(uiSettings, value => {
  if (!value) return
  const style = value.personalization.chat_style || 'default'
  if (PRESET_KEYS.includes(style)) { selected.value = style; customStyle.value = '' }
  else { selected.value = 'custom'; customStyle.value = style }
  extraContext.value = value.personalization.extra_context
}, { immediate: true })

const effectiveStyle = computed(() => selected.value === 'custom' ? customStyle.value.trim() : selected.value)

async function commit(message: string) {
  await saveUi({ personalization: { chat_style: effectiveStyle.value, extra_context: extraContext.value.trim() } }, message)
}
function pickStyle(key: string) {
  selected.value = key
  if (key !== 'custom') commit('对话语言风格已更新')
}
</script>

<template>
  <div class="s-group">
    <h2>对话语言风格</h2>
    <p class="desc">影响 Sculk Agent 与 ACP 智能体的回复语气；自定义可以用自己的话描述期望的表达方式。</p>
    <div class="s-cards">
      <button v-for="style in CHAT_STYLES" :key="style.key" class="s-pick-card" :class="{active:selected===style.key}" @click="pickStyle(style.key)">
        <b>{{ style.label }}</b><small>{{ style.hint }}</small>
        <span v-if="selected===style.key" class="check"><Check/></span>
      </button>
    </div>
    <div v-if="selected==='custom'" class="s-card" style="margin-top:10px">
      <div class="s-row" style="align-items:stretch;flex-direction:column">
        <p style="margin-bottom:7px"><b>自定义风格描述</b><small>例如：像资深运维同事一样直接，先给结论，涉及风险时明确提醒</small></p>
        <textarea class="s-textarea" v-model="customStyle" placeholder="描述你希望 AI 用什么风格与你交流…" @change="commit('自定义风格已保存')"/>
      </div>
    </div>
  </div>

  <div class="s-group">
    <h2>额外上下文</h2>
    <p class="desc">每次对话都会附带这些背景信息，让 AI 更了解你的服务器与偏好（例如主服规则、玩家群体、常用插件栈）。</p>
    <div class="s-card">
      <div class="s-row" style="align-items:stretch;flex-direction:column">
        <textarea class="s-textarea" v-model="extraContext" style="min-height:110px" placeholder="例如：主服是 1.21.4 Paper 生存服，经济插件用 Vault + EconomyShopGUI，禁止在周末高峰期重启…" @change="commit('额外上下文已保存')"/>
        <small style="margin-top:6px;color:#5d6975;font-size:7px">内容保存在本地 state.json，会注入到系统提示，请避免填写密钥等敏感信息。</small>
      </div>
    </div>
  </div>
</template>
