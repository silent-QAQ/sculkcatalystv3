<script setup lang="ts">
import { computed, ref } from 'vue'
import { Bot, Check, FlaskConical, LoaderCircle, Pencil, Plus, RefreshCw, SquareTerminal, Trash2, X } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import { AGENT_KINDS, REASONING_EFFORTS } from '../types'
import type { AgentTransport, AiAgent, AiSettingsView, DetectedAgent, ReasoningEffort, TestResult } from '../types'
import { aiSettings, flash, friendly, loadAi } from '../store'

const agentModal = ref(false)
const agentModalMode = ref<'create' | 'edit'>('create')
const editingAgentId = ref('')
const agentSaving = ref(false)
const agentFormError = ref('')
const agentForm = ref<{ name: string; kind: string; command: string; args: string; enabled: boolean; transport: AgentTransport; reasoning_effort: ReasoningEffort | null }>({ name: '', kind: 'custom', command: '', args: '', enabled: true, transport: 'acp', reasoning_effort: null })
const agentTesting = ref('')
const agentTestResults = ref<Record<string, TestResult>>({})
const agentConfirmOpen = ref(false)
const agentConfirmTarget = ref<AiAgent | null>(null)
const agentDeleting = ref(false)
const detecting = ref(false)
const addingDetected = ref('')

const agents = computed(() => aiSettings.value?.agents ?? [])
const detectedAgents = computed(() => aiSettings.value?.detected_agents ?? [])
const activeAgent = computed(() => aiSettings.value?.active_agent ?? null)
const agentKindLabel = (kind: string) => AGENT_KINDS.find(item => item.key === kind)?.label ?? kind
const agentCommandHint = computed(() => AGENT_KINDS.find(item => item.key === agentForm.value.kind)?.commandHint ?? '')
const agentTransportLabel = (agent: AiAgent) => (agent.transport ?? 'acp') === 'cli' ? '原生 CLI' : 'ACP'
const detectedConfigured = (detected: DetectedAgent) => agents.value.some(agent => agent.kind === detected.kind && (agent.transport ?? 'acp') === 'cli')
const detectedStatus = (detected: DetectedAgent) => detected.available ? (detectedConfigured(detected) ? '已接入' : '可用') : detected.installed ? '已检测，但不可用' : '未安装'
const formReasoningEfforts = computed(() => {
  const detected = detectedAgents.value.find(item => item.kind === agentForm.value.kind)
  if (agentForm.value.transport === 'cli' && detected?.capabilities.reasoning_effort.supported) {
    return REASONING_EFFORTS.filter(item => detected.capabilities.reasoning_effort.values.includes(item.key))
  }
  return []
})

function openAgentCreate() {
  agentModalMode.value = 'create'; editingAgentId.value = ''
  agentForm.value = { name: '', kind: 'custom', command: '', args: '', enabled: true, transport: 'acp', reasoning_effort: null }
  agentFormError.value = ''; agentModal.value = true
}
function openAgentEdit(agent: AiAgent) {
  agentModalMode.value = 'edit'; editingAgentId.value = agent.id
  agentForm.value = { name: agent.name, kind: agent.kind, command: agent.command, args: agent.args.join(' '), enabled: agent.enabled, transport: agent.transport ?? 'acp', reasoning_effort: agent.reasoning_effort ?? null }
  agentFormError.value = ''; agentModal.value = true
}
function fillAgentPreset(kind: string) {
  agentForm.value.kind = kind
  if (kind === 'codex' || kind === 'claude-code') {
    const detected = detectedAgents.value.find(item => item.kind === kind)
    agentForm.value.transport = 'cli'
    agentForm.value.command = detected?.command || (kind === 'codex' ? 'codex' : 'claude')
    agentForm.value.args = ''
  } else {
    agentForm.value.transport = 'acp'
    agentForm.value.reasoning_effort = null
  }
  if (agentForm.value.name.trim()) return
  const preset = AGENT_KINDS.find(item => item.key === kind)
  if (preset && kind !== 'custom') agentForm.value.name = preset.label
}
async function saveAgent() {
  const name = agentForm.value.name.trim(), command = agentForm.value.command.trim()
  if (!name || !command) { agentFormError.value = '请填写 Agent 名称和启动命令'; return }
  agentSaving.value = true; agentFormError.value = ''
  try {
    const path = agentModalMode.value === 'create' ? '/api/ai/agents' : '/api/ai/agents/' + editingAgentId.value
    await apiRequest(path, {
      method: agentModalMode.value === 'create' ? 'POST' : 'PUT',
      body: JSON.stringify({ name, kind: agentForm.value.kind, command, args: agentForm.value.args.trim() ? agentForm.value.args.trim().split(/\s+/) : [], enabled: agentForm.value.enabled, transport: agentForm.value.transport, reasoning_effort: agentForm.value.transport === 'cli' ? agentForm.value.reasoning_effort : null }),
    })
    agentModal.value = false
    await loadAi()
    flash(agentModalMode.value === 'create' ? 'Agent 已添加，可点击「测试」验证连接' : 'Agent 已更新')
  } catch (error) { agentFormError.value = friendly(error) }
  finally { agentSaving.value = false }
}
async function refreshDetection() {
  detecting.value = true
  try { await loadAi(); flash('本机 CLI 检测已刷新') }
  catch (error) { flash('检测失败：' + friendly(error)) }
  finally { detecting.value = false }
}
async function addDetectedAgent(detected: DetectedAgent) {
  if (!detected.available || detectedConfigured(detected)) return
  addingDetected.value = detected.kind
  try {
    await apiRequest('/api/ai/agents', {
      method: 'POST',
      body: JSON.stringify({ name: detected.name, kind: detected.kind, command: detected.command, args: [], enabled: true, transport: 'cli', reasoning_effort: null }),
    })
    await loadAi()
    flash(`${detected.name} 已接入，可在对话输入框中选择`)
  } catch (error) { flash('接入失败：' + friendly(error)) }
  finally { addingDetected.value = '' }
}
function askDeleteAgent(agent: AiAgent) { agentConfirmTarget.value = agent; agentConfirmOpen.value = true }
async function deleteAgent() {
  if (!agentConfirmTarget.value) return
  agentDeleting.value = true
  try {
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/agents/' + agentConfirmTarget.value.id, { method: 'DELETE' })
    agentConfirmOpen.value = false
    flash('Agent 已删除')
  } catch (error) { flash('删除失败：' + friendly(error)) }
  finally { agentDeleting.value = false }
}
async function testAgent(agent: AiAgent) {
  agentTesting.value = agent.id
  try {
    agentTestResults.value = { ...agentTestResults.value, [agent.id]: await apiRequest<TestResult>('/api/ai/agents/' + agent.id + '/test', { method: 'POST' }) }
  } catch (error) {
    agentTestResults.value = { ...agentTestResults.value, [agent.id]: { ok: false, latency_ms: 0, error: friendly(error) } }
  } finally { agentTesting.value = '' }
}
async function setActiveAgent(agentId: string | null) {
  try {
    aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/agents/active', { method: 'PUT', body: JSON.stringify({ agent_id: agentId }) })
    flash(agentId ? '默认对话已切换到该 Agent' : '已恢复内置 Sculk Agent（模型直连）')
  } catch (error) { flash('切换失败：' + friendly(error)) }
}
</script>

<template>
  <div class="s-group">
    <h2 style="display:flex;align-items:center;justify-content:space-between">智能体<button class="s-btn primary" @click="openAgentCreate"><Plus/>添加 Agent</button></h2>
    <p class="desc">自动检测本机 Codex CLI 与 Claude Code CLI；原生 CLI 可直接使用各自的思考强度，其他智能体仍可通过 ACP 协议接入。</p>
    <div class="s-card">
      <div class="s-row" style="background:color-mix(in srgb,var(--accent) 3%,transparent)">
        <p><b>本机 CLI</b><small>进入本页时自动检测；检测到但无法执行时不会冒充为可用。</small></p>
        <button class="s-btn small" :disabled="detecting" @click="refreshDetection"><LoaderCircle v-if="detecting" class="s-spin"/><RefreshCw v-else/>重新检测</button>
      </div>
      <div v-for="detected in detectedAgents" :key="detected.kind" class="s-row">
        <span style="display:grid;place-items:center;width:29px;height:29px;border-radius:7px;color:var(--accent);background:color-mix(in srgb,var(--accent) 10%,transparent);flex:none"><SquareTerminal style="width:14px"/></span>
        <p>
          <b>{{ detected.name }}<em style="margin-left:6px;padding:2px 5px;border-radius:4px;color:#8f84d8;background:rgba(156,140,255,.1);font:normal 7px Inter">原生 CLI</em></b>
          <code v-if="detected.path">{{ detected.path }}</code>
          <small>{{ detected.version || detected.reason || '尚未检测到可执行版本' }}</small>
        </p>
        <span class="s-test" :class="{ok:detected.available}">{{ detectedStatus(detected) }}</span>
        <button v-if="detected.available&&!detectedConfigured(detected)" class="s-btn small primary" :disabled="addingDetected===detected.kind" @click="addDetectedAgent(detected)"><LoaderCircle v-if="addingDetected===detected.kind" class="s-spin"/><Plus v-else/>接入</button>
        <span v-else-if="detectedConfigured(detected)" class="s-test ok"><Check/>已接入</span>
      </div>
      <div v-if="!detectedAgents.length" class="s-row"><p><small>正在等待后端返回 CLI 检测结果。</small></p></div>
    </div>
  </div>

  <div class="s-group">
    <h2>已接入的智能体</h2>
    <div class="s-card">
      <div class="s-row" :style="activeAgent?{}:{background:'color-mix(in srgb,var(--accent) 5%,transparent)'}">
        <span style="display:grid;place-items:center;width:29px;height:29px;border-radius:7px;color:var(--accent);background:color-mix(in srgb,var(--accent) 10%,transparent);flex:none"><Bot style="width:14px"/></span>
        <p><b>Sculk Agent（内置默认）</b><small>直连已配置的模型提供商，按情景绑定选择模型</small></p>
        <span v-if="!activeAgent" class="s-test ok" style="display:flex;align-items:center;gap:4px"><Check style="width:11px"/>默认对话使用</span>
        <button v-else class="s-btn small" @click="setActiveAgent(null)">设为默认</button>
      </div>
      <div v-for="agent in agents" :key="agent.id" class="s-row" :style="{opacity:agent.enabled?1:.55,...(activeAgent===agent.id?{background:'color-mix(in srgb,var(--accent) 5%,transparent)'}:{})}">
        <span style="display:grid;place-items:center;width:29px;height:29px;border-radius:7px;color:#a99dff;background:rgba(156,140,255,.09);flex:none"><Bot style="width:14px"/></span>
        <p>
          <b>{{ agent.name }}<em style="margin-left:6px;padding:2px 5px;border-radius:4px;color:#8f84d8;background:rgba(156,140,255,.1);font:normal 7px Inter">{{ agentKindLabel(agent.kind) }} · {{ agentTransportLabel(agent) }}</em></b>
          <code>{{ agent.command }} {{ agent.args.join(' ') }}</code>
        </p>
        <span v-if="agentTestResults[agent.id]" class="s-test" :class="{ok:agentTestResults[agent.id].ok}">
          <template v-if="agentTestResults[agent.id].ok">✓ {{ agentTestResults[agent.id].latency_ms }} ms<em v-if="agentTestResults[agent.id].reply"> · {{ agentTestResults[agent.id].reply }}</em></template>
          <template v-else>✗ {{ agentTestResults[agent.id].error }}</template>
        </span>
        <span v-if="activeAgent===agent.id" class="s-test ok" style="display:flex;align-items:center;gap:4px"><Check style="width:11px"/>默认对话使用</span>
        <button v-else-if="agent.enabled" class="s-btn small" @click="setActiveAgent(agent.id)">设为默认</button>
        <button class="s-btn small" :disabled="agentTesting===agent.id" @click="testAgent(agent)"><LoaderCircle v-if="agentTesting===agent.id" class="s-spin"/><FlaskConical v-else/>测试</button>
        <button class="s-btn small" @click="openAgentEdit(agent)"><Pencil/></button>
        <button class="s-btn small danger" @click="askDeleteAgent(agent)"><Trash2/></button>
      </div>
    </div>
  </div>

  <div v-if="agentModal" class="s-modal-backdrop" @click.self="agentModal=false">
    <section class="s-modal">
      <header><b>{{ agentModalMode==='create' ? '添加智能体' : '编辑智能体' }}</b><button @click="agentModal=false"><X/></button></header>
      <div class="field"><label>Agent 类型</label><select class="s-select" style="max-width:none" :value="agentForm.kind" @change="fillAgentPreset(($event.target as HTMLSelectElement).value)"><option v-for="kind in AGENT_KINDS" :key="kind.key" :value="kind.key">{{ kind.label }}</option></select></div>
      <div class="field"><label>接入方式</label><select v-model="agentForm.transport" class="s-select" style="max-width:none"><option value="cli" :disabled="agentForm.kind!=='codex'&&agentForm.kind!=='claude-code'">原生 CLI</option><option value="acp">ACP 协议</option></select><small>原生 CLI 可传入模型思考强度；ACP 仅使用适配器自身公开的能力。</small></div>
      <div class="field"><label>名称</label><input class="s-input" v-model="agentForm.name" placeholder="例如：Codex CLI"/></div>
      <div class="field"><label>启动命令</label><input class="s-input" v-model="agentForm.command" placeholder="可执行文件或命令名"/><small>{{ agentCommandHint }}</small></div>
      <div class="field"><label>启动参数（空格分隔）</label><input class="s-input" v-model="agentForm.args" placeholder="例如：acp 或 @zed-industries/claude-code-acp"/></div>
      <div v-if="agentForm.transport==='cli'" class="field"><label>默认思考强度</label><select v-model="agentForm.reasoning_effort" class="s-select" style="max-width:none"><option :value="null">跟随对话或 CLI 默认值</option><option v-for="effort in formReasoningEfforts" :key="effort.key" :value="effort.key">{{ effort.label }} · {{ effort.hint }}</option></select></div>
      <label class="check"><input v-model="agentForm.enabled" type="checkbox"/><span>启用该 Agent</span></label>
      <p v-if="agentFormError" class="s-error">{{ agentFormError }}</p>
      <footer><button class="s-btn" @click="agentModal=false">取消</button><button class="s-btn primary" :disabled="agentSaving" @click="saveAgent"><LoaderCircle v-if="agentSaving" class="s-spin"/>{{ agentModalMode==='create' ? '添加' : '保存' }}</button></footer>
    </section>
  </div>

  <div v-if="agentConfirmOpen" class="s-modal-backdrop" @click.self="agentConfirmOpen=false">
    <section class="s-modal">
      <header><b>删除 {{ agentConfirmTarget?.name }}？</b><button @click="agentConfirmOpen=false"><X/></button></header>
      <p class="confirm-body">该 Agent 的接入配置会被移除；若它是默认对话 Agent，将恢复为内置 Sculk Agent。</p>
      <footer><button class="s-btn" @click="agentConfirmOpen=false">取消</button><button class="s-btn danger-solid" :disabled="agentDeleting" @click="deleteAgent"><LoaderCircle v-if="agentDeleting" class="s-spin"/>确认删除</button></footer>
    </section>
  </div>
</template>
