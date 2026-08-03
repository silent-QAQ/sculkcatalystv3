<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { AudioLines, Check, Cloud, Mic, Save } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import type { AiSettingsView, SpeechRecognitionMode, SpeechRecognitionSettings } from '../types'
import { aiSettings, flash, friendly } from '../store'

const mode = ref<SpeechRecognitionMode>('browser')
const language = ref('zh-CN')
const providerId = ref('')
const modelId = ref('whisper-1')
const saving = ref(false)
const error = ref('')

const providers = computed(() => aiSettings.value?.providers ?? [])
const selectedProvider = computed(() => providers.value.find(provider => provider.id === providerId.value))
const browserSupported = typeof window !== 'undefined' && ('SpeechRecognition' in window || 'webkitSpeechRecognition' in window)
const LANGUAGES = [
  { key: 'auto', label: '自动检测' },
  { key: 'zh-CN', label: '简体中文' },
  { key: 'zh-TW', label: '繁體中文' },
  { key: 'en-US', label: 'English (US)' },
  { key: 'ja-JP', label: '日本語' },
  { key: 'ko-KR', label: '한국어' },
]

watch(() => aiSettings.value?.speech_recognition, value => {
  if (!value) return
  mode.value = value.mode
  language.value = value.language || 'auto'
  providerId.value = value.provider_id ?? ''
  modelId.value = value.model_id || 'whisper-1'
}, { immediate: true })

async function save() {
  error.value = ''
  if (mode.value === 'model' && !providerId.value) { error.value = '请选择用于语音转写的模型提供商'; return }
  if (mode.value === 'model' && !modelId.value.trim()) { error.value = '请填写 ASR 模型 ID'; return }
  saving.value = true
  try {
    const payload: SpeechRecognitionSettings = {
      mode: mode.value,
      language: language.value,
      provider_id: providerId.value || null,
      model_id: modelId.value.trim() || 'whisper-1',
    }
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/speech-recognition', {
      method: 'PUT',
      body: JSON.stringify(payload),
    })
    flash('语音识别设置已保存')
  } catch (cause) { error.value = friendly(cause) }
  finally { saving.value = false }
}
</script>

<template>
  <div class="s-group">
    <h2>识别方式</h2>
    <p class="desc">录音只会转成对话草稿，不会自动发送。你可以使用浏览器语音识别，或把录音交给自己配置的 OpenAI 兼容 ASR 模型。</p>
    <div class="speech-mode-grid">
      <button class="s-pick-card" :class="{active:mode==='browser'}" @click="mode='browser'">
        <span class="speech-mode-icon"><Mic/></span>
        <b>浏览器语音识别</b>
        <small>直接调用浏览器提供的 Speech Recognition API，不经过 Sculk 后端。</small>
        <span v-if="mode==='browser'" class="check"><Check/></span>
      </button>
      <button class="s-pick-card" :class="{active:mode==='model'}" @click="mode='model'">
        <span class="speech-mode-icon"><Cloud/></span>
        <b>ASR 模型转写</b>
        <small>录音结束后上传到本机后端，再由后端调用所选提供商的音频转写接口。</small>
        <span v-if="mode==='model'" class="check"><Check/></span>
      </button>
    </div>
    <p v-if="mode==='browser'&&!browserSupported" class="s-error">当前浏览器未提供语音识别 API，请改用 ASR 模型模式，或使用支持该 API 的浏览器。</p>
  </div>

  <div class="s-group">
    <h2>识别配置</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>识别语言</b><small>自动检测适合多语言输入；指定语言通常能减少短句误识别。</small></p>
        <select v-model="language" class="s-select">
          <option v-for="item in LANGUAGES" :key="item.key" :value="item.key">{{ item.label }}</option>
        </select>
      </div>
      <template v-if="mode==='model'">
        <div class="s-row">
          <p><b>模型提供商</b><small>复用“模型”页面保存的 API 地址与密钥，密钥不会发送到前端。</small></p>
          <select v-model="providerId" class="s-select">
            <option value="">选择提供商</option>
            <option v-for="provider in providers" :key="provider.id" :value="provider.id">{{ provider.name }}{{ provider.enabled ? '' : '（已停用）' }}</option>
          </select>
        </div>
        <div class="s-row">
          <p><b>ASR 模型 ID</b><small>例如 whisper-1、gpt-4o-mini-transcribe，亦可填写兼容服务提供的模型名。</small></p>
          <div class="speech-model-input">
            <AudioLines/>
            <input v-model="modelId" class="s-input" list="speech-model-options" placeholder="whisper-1"/>
            <datalist id="speech-model-options">
              <option v-for="model in selectedProvider?.models ?? []" :key="model.id" :value="model.id"/>
            </datalist>
          </div>
        </div>
      </template>
    </div>
  </div>

  <div class="speech-save-row">
    <p><span v-if="mode==='browser'">浏览器可能使用其厂商的在线识别服务，具体由浏览器实现决定。</span><span v-else>单次录音上限 25 MiB，录音内容会发送给你选择的模型提供商。</span></p>
    <button class="s-btn primary" :disabled="saving" @click="save"><Save/>{{ saving ? '保存中…' : '保存设置' }}</button>
  </div>
  <p v-if="error" class="s-error">{{ error }}</p>
</template>

<style scoped>
.speech-mode-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:9px}.speech-mode-grid .s-pick-card{min-height:126px}.speech-mode-icon{width:34px;height:34px;display:grid;place-items:center;border-radius:9px;color:#8e9aa6;background:rgba(255,255,255,.045)}.active .speech-mode-icon{color:var(--accent);background:color-mix(in srgb,var(--accent) 11%,transparent)}.speech-mode-icon svg{width:17px}.speech-model-input{width:min(360px,52%);display:flex;align-items:center;gap:7px}.speech-model-input>svg{width:14px;flex:none;color:#7b8793}.speech-model-input .s-input{min-width:0;flex:1}.speech-save-row{display:flex;align-items:center;justify-content:space-between;gap:14px}.speech-save-row p{margin:0;color:#687581;font-size:8px;line-height:1.6}.speech-save-row .s-btn{flex:none}@media(max-width:760px){.speech-mode-grid{grid-template-columns:1fr}.speech-model-input{width:100%}.speech-save-row{align-items:flex-start;flex-direction:column}}
</style>
