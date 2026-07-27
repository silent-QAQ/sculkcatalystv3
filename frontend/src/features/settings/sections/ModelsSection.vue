<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronDown, ChevronRight, FlaskConical, LoaderCircle, Pencil, Plus, RefreshCw, Trash2, X } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import { SCENARIOS } from '../types'
import type { AiProviderView, AiSettingsView, ModelBinding, TestResult } from '../types'
import { aiSettings, flash, friendly, loadAi } from '../store'

const expanded = ref<string[]>([])
const syncing = ref('')
const togglingModel = ref('')
const testing = ref('')
const testResults = ref<Record<string, TestResult>>({})
const addingModel = ref<Record<string, string>>({})

const providerModal = ref(false)
const modalMode = ref<'create' | 'edit'>('create')
const editingId = ref('')
const saving = ref(false)
const formError = ref('')
const form = ref({ name: '', base_url: '', api_key: '', enabled: true })
const editingMaskedKey = ref('')
const confirmOpen = ref(false)
const confirmTarget = ref<AiProviderView | null>(null)
const deleting = ref(false)

const providers = computed(() => aiSettings.value?.providers ?? [])
const enabledProviders = computed(() => providers.value.filter(provider => provider.enabled))

function modelKey(providerId: string, modelId: string) { return providerId + '::' + modelId }
function syncedLabel(provider: AiProviderView) {
  if (!provider.models_synced_at) return '未同步'
  const date = new Date(provider.models_synced_at)
  return isNaN(date.getTime()) ? '已同步' : date.toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}
async function reload() { try { await loadAi() } catch (error) { flash('刷新失败：' + friendly(error)) } }

function openCreate() {
  modalMode.value = 'create'; editingId.value = ''; editingMaskedKey.value = ''
  form.value = { name: '', base_url: '', api_key: '', enabled: true }
  formError.value = ''; providerModal.value = true
}
function openEdit(provider: AiProviderView) {
  modalMode.value = 'edit'; editingId.value = provider.id; editingMaskedKey.value = provider.api_key_masked
  form.value = { name: provider.name, base_url: provider.base_url, api_key: '', enabled: provider.enabled }
  formError.value = ''; providerModal.value = true
}
async function saveProvider() {
  const name = form.value.name.trim(), baseUrl = form.value.base_url.trim()
  if (!name || !baseUrl) { formError.value = '请填写提供商名称和 API 地址'; return }
  saving.value = true; formError.value = ''
  try {
    const path = modalMode.value === 'create' ? '/api/ai/providers' : '/api/ai/providers/' + editingId.value
    await apiRequest(path, {
      method: modalMode.value === 'create' ? 'POST' : 'PUT',
      body: JSON.stringify({ name, base_url: baseUrl, api_key: form.value.api_key.trim() || null, enabled: form.value.enabled }),
    })
    providerModal.value = false
    await reload()
    flash(modalMode.value === 'create' ? '提供商已添加，可点击「同步模型」拉取模型列表' : '提供商已更新')
  } catch (error) { formError.value = friendly(error) }
  finally { saving.value = false }
}
function askDelete(provider: AiProviderView) { confirmTarget.value = provider; confirmOpen.value = true }
async function deleteProvider() {
  if (!confirmTarget.value) return
  deleting.value = true
  try {
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/providers/' + confirmTarget.value.id, { method: 'DELETE' })
    confirmOpen.value = false
    flash('提供商已删除，相关情景绑定已清除')
  } catch (error) { flash('删除失败：' + friendly(error)) }
  finally { deleting.value = false }
}
async function toggleProvider(provider: AiProviderView) {
  try {
    await apiRequest('/api/ai/providers/' + provider.id, {
      method: 'PUT',
      body: JSON.stringify({ name: provider.name, base_url: provider.base_url, enabled: !provider.enabled }),
    })
    await reload()
  } catch (error) { flash('操作失败：' + friendly(error)) }
}
async function syncModels(provider: AiProviderView) {
  syncing.value = provider.id
  try {
    await apiRequest('/api/ai/providers/' + provider.id + '/models/sync', { method: 'POST' })
    await reload()
    if (!expanded.value.includes(provider.id)) expanded.value.push(provider.id)
    flash('模型列表已同步，勾选需要启用的模型')
  } catch (error) { flash('同步失败：' + friendly(error)) }
  finally { syncing.value = '' }
}
function toggleExpand(id: string) {
  expanded.value = expanded.value.includes(id) ? expanded.value.filter(item => item !== id) : [...expanded.value, id]
}
async function toggleModel(provider: AiProviderView, modelId: string) {
  togglingModel.value = modelKey(provider.id, modelId)
  try {
    await apiRequest('/api/ai/providers/' + provider.id + '/models/toggle', { method: 'POST', body: JSON.stringify({ model_id: modelId }) })
    await reload()
  } catch (error) { flash('操作失败：' + friendly(error)) }
  finally { togglingModel.value = '' }
}
async function addModel(provider: AiProviderView) {
  const modelId = (addingModel.value[provider.id] ?? '').trim()
  if (!modelId) return
  try {
    await apiRequest('/api/ai/providers/' + provider.id + '/models/add', { method: 'POST', body: JSON.stringify({ model_id: modelId }) })
    addingModel.value = { ...addingModel.value, [provider.id]: '' }
    await reload()
    flash('模型已添加并启用')
  } catch (error) { flash('添加失败：' + friendly(error)) }
}
async function removeModel(provider: AiProviderView, modelId: string) {
  try {
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/providers/' + provider.id + '/models/remove', { method: 'POST', body: JSON.stringify({ model_id: modelId }) })
    flash('模型已移除，引用它的绑定已清除')
  } catch (error) { flash('移除失败：' + friendly(error)) }
}
async function testModel(provider: AiProviderView, modelId: string) {
  const key = modelKey(provider.id, modelId)
  testing.value = key
  try {
    testResults.value = { ...testResults.value, [key]: await apiRequest<TestResult>('/api/ai/test', { method: 'POST', body: JSON.stringify({ provider_id: provider.id, model_id: modelId }) }) }
  } catch (error) {
    testResults.value = { ...testResults.value, [key]: { ok: false, latency_ms: 0, error: friendly(error) } }
  } finally { testing.value = '' }
}

function bindingOf(scenario: string): ModelBinding | null {
  if (!aiSettings.value) return null
  return scenario === 'default' ? aiSettings.value.default_binding ?? null : aiSettings.value.scenarios[scenario] ?? null
}
function bindingValue(scenario: string) {
  const binding = bindingOf(scenario)
  return binding ? modelKey(binding.provider_id, binding.model_id) : ''
}
function bindingInvalid(scenario: string) {
  const binding = bindingOf(scenario)
  if (!binding) return false
  const provider = providers.value.find(item => item.id === binding.provider_id)
  return !provider || !provider.enabled
}
async function setBinding(scenario: string, value: string) {
  let binding: ModelBinding | null = null
  if (value) {
    const [providerId, ...rest] = value.split('::')
    binding = { provider_id: providerId, model_id: rest.join('::') }
  }
  try {
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/scenarios', { method: 'PUT', body: JSON.stringify({ scenario, binding }) })
    flash(binding ? '情景模型已更新' : '已恢复为默认模型')
  } catch (error) { flash('绑定失败：' + friendly(error)) }
}
</script>

<template>
  <div class="s-group">
    <h2 style="display:flex;align-items:center;justify-content:space-between">模型提供商<button class="s-btn primary" @click="openCreate"><Plus/>添加提供商</button></h2>
    <p class="desc">接入 OpenAI 格式 API 或中转站，支持多个提供商同时接入。同步模型列表后，也可手动添加上游未列出的模型 ID。</p>
    <div v-if="!providers.length" class="s-empty">还没有配置提供商。点击「添加提供商」接入 OpenAI 格式 API 或中转站。</div>
    <div v-for="provider in providers" :key="provider.id" class="s-card" :style="{marginTop:'8px',opacity:provider.enabled?1:.55}">
      <div class="s-row">
        <button class="s-btn small" style="border:0;background:transparent;padding:0 2px" @click="toggleExpand(provider.id)"><ChevronDown v-if="expanded.includes(provider.id)"/><ChevronRight v-else/></button>
        <p><b>{{ provider.name }}</b><small>{{ provider.base_url }}<template v-if="provider.has_key"> · 密钥 {{ provider.api_key_masked }}</template><template v-else> · 无密钥</template></small></p>
        <small style="color:#5d6975;font-size:7px;white-space:nowrap">{{ provider.models.length }} 个模型 · {{ syncedLabel(provider) }}</small>
        <button class="s-btn small" :disabled="syncing===provider.id" @click="syncModels(provider)"><LoaderCircle v-if="syncing===provider.id" class="s-spin"/><RefreshCw v-else/>同步模型</button>
        <button class="s-btn small" @click="openEdit(provider)"><Pencil/>编辑</button>
        <button class="s-btn small danger" @click="askDelete(provider)"><Trash2/></button>
        <button class="s-switch" :class="{on:provider.enabled}" @click="toggleProvider(provider)"><i/></button>
      </div>
      <template v-if="expanded.includes(provider.id)">
        <div v-if="!provider.models.length" class="s-row"><p><small>尚未同步模型。点击「同步模型」从上游 <code>/v1/models</code> 读取列表，或在下方手动添加模型 ID。</small></p></div>
        <div v-for="model in provider.models" :key="model.id" class="s-row" style="padding-left:34px">
          <label style="display:flex;align-items:center;gap:7px;min-width:0;cursor:pointer;flex:1">
            <input type="checkbox" :checked="model.enabled" :disabled="togglingModel===modelKey(provider.id,model.id)" :style="{accentColor:'var(--accent)'}" @change="toggleModel(provider,model.id)"/>
            <code style="overflow:hidden;color:#aeb9c4;font:8px 'Cascadia Code',monospace;text-overflow:ellipsis;white-space:nowrap">{{ model.id }}</code>
          </label>
          <span v-if="testResults[modelKey(provider.id,model.id)]" class="s-test" :class="{ok:testResults[modelKey(provider.id,model.id)].ok}">
            <template v-if="testResults[modelKey(provider.id,model.id)].ok">✓ {{ testResults[modelKey(provider.id,model.id)].latency_ms }} ms<em v-if="testResults[modelKey(provider.id,model.id)].reply"> · {{ testResults[modelKey(provider.id,model.id)].reply }}</em></template>
            <template v-else>✗ {{ testResults[modelKey(provider.id,model.id)].error }}</template>
          </span>
          <button class="s-btn small" :disabled="testing===modelKey(provider.id,model.id)" @click="testModel(provider,model.id)"><LoaderCircle v-if="testing===modelKey(provider.id,model.id)" class="s-spin"/><FlaskConical v-else/>测试</button>
          <button class="s-btn small danger" @click="removeModel(provider,model.id)"><Trash2/></button>
        </div>
        <div class="s-row" style="padding-left:34px">
          <input class="s-input" style="flex:1;max-width:300px" :value="addingModel[provider.id] ?? ''" placeholder="手动添加模型 ID，例如 gpt-4o-mini" @input="addingModel={...addingModel,[provider.id]:($event.target as HTMLInputElement).value}" @keydown.enter="addModel(provider)"/>
          <button class="s-btn small" @click="addModel(provider)"><Plus/>添加模型</button>
        </div>
      </template>
    </div>
  </div>

  <div class="s-group">
    <h2>情景模型绑定</h2>
    <p class="desc">为不同工作情景选择合适的模型；未单独绑定的情景会回退到默认模型。</p>
    <div class="s-card">
      <div class="s-row">
        <p><b>对话默认模型</b><small>所有未单独绑定的情景使用此模型</small></p>
        <span v-if="bindingInvalid('default')" class="stale">绑定已失效</span>
        <select class="s-select" :value="bindingValue('default')" @change="setBinding('default',($event.target as HTMLSelectElement).value)">
          <option value="">未设置（回退本地规则回复）</option>
          <optgroup v-for="provider in enabledProviders" :key="provider.id" :label="provider.name">
            <option v-for="model in provider.models.filter(m=>m.enabled)" :key="model.id" :value="modelKey(provider.id,model.id)">{{ model.id }}</option>
          </optgroup>
        </select>
      </div>
      <div v-for="scenario in SCENARIOS" :key="scenario.key" class="s-row">
        <p><b>{{ scenario.label }}</b><small>{{ scenario.hint }}</small></p>
        <span v-if="bindingInvalid(scenario.key)" class="stale">绑定已失效</span>
        <select class="s-select" :value="bindingValue(scenario.key)" @change="setBinding(scenario.key,($event.target as HTMLSelectElement).value)">
          <option value="">跟随默认模型</option>
          <optgroup v-for="provider in enabledProviders" :key="provider.id" :label="provider.name">
            <option v-for="model in provider.models.filter(m=>m.enabled)" :key="model.id" :value="modelKey(provider.id,model.id)">{{ model.id }}</option>
          </optgroup>
        </select>
      </div>
    </div>
  </div>

  <div v-if="providerModal" class="s-modal-backdrop" @click.self="providerModal=false">
    <section class="s-modal">
      <header><b>{{ modalMode==='create' ? '添加模型提供商' : '编辑模型提供商' }}</b><button @click="providerModal=false"><X/></button></header>
      <div class="field"><label>名称</label><input class="s-input" v-model="form.name" placeholder="例如：OpenAI 官方 / 某中转站"/></div>
      <div class="field"><label>API 地址（OpenAI 格式）</label><input class="s-input" v-model="form.base_url" placeholder="https://api.openai.com/v1 或中转站地址"/><small>末尾带不带 /v1 均可，将自动归一</small></div>
      <div class="field"><label>API Key</label><input class="s-input" v-model="form.api_key" type="password" :placeholder="modalMode==='edit'&&editingMaskedKey ? '留空保持不变（当前 '+editingMaskedKey+'）' : 'sk-…（本地网关可留空）'"/></div>
      <label class="check"><input v-model="form.enabled" type="checkbox"/><span>启用该提供商</span></label>
      <p v-if="formError" class="s-error">{{ formError }}</p>
      <footer><button class="s-btn" @click="providerModal=false">取消</button><button class="s-btn primary" :disabled="saving" @click="saveProvider"><LoaderCircle v-if="saving" class="s-spin"/>{{ modalMode==='create' ? '添加' : '保存' }}</button></footer>
    </section>
  </div>

  <div v-if="confirmOpen" class="s-modal-backdrop" @click.self="confirmOpen=false">
    <section class="s-modal">
      <header><b>删除 {{ confirmTarget?.name }}？</b><button @click="confirmOpen=false"><X/></button></header>
      <p class="confirm-body">提供商及其模型缓存会被移除，引用它的情景绑定与默认模型会一并清除。此操作不可撤销。</p>
      <footer><button class="s-btn" @click="confirmOpen=false">取消</button><button class="s-btn danger-solid" :disabled="deleting" @click="deleteProvider"><LoaderCircle v-if="deleting" class="s-spin"/>确认删除</button></footer>
    </section>
  </div>
</template>
