<script setup lang="ts">
import { computed, ref } from 'vue'
import { Bot, Check, FlaskConical, LoaderCircle, Pencil, Plus, Trash2, X } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import { AGENT_KINDS } from '../types'
import type { AiAgent, AiSettingsView, TestResult } from '../types'
import { aiSettings, flash, friendly, loadAi } from '../store'

const agentModal = ref(false)
const agentModalMode = ref<'create' | 'edit'>('create')
const editingAgentId = ref('')
const agentSaving = ref(false)
const agentFormError = ref('')
const agentForm = ref({ name: '', kind: 'custom', command: '', args: '', enabled: true })
const agentTesting = ref('')
const agentTestResults = ref<Record<string, TestResult>>({})
const agentConfirmOpen = ref(false)
const agentConfirmTarget = ref<AiAgent | null>(null)
const agentDeleting = ref(false)

const agents = computed(() => aiSettings.value?.agents ?? [])
const activeAgent = computed(() => aiSettings.value?.active_agent ?? null)
const agentKindLabel = (kind: string) => AGENT_KINDS.find(item => item.key === kind)?.label ?? kind
const agentCommandHint = computed(() => AGENT_KINDS.find(item => item.key === agentForm.value.kind)?.commandHint ?? '')

function openAgentCreate() {
  agentModalMode.value = 'create'; editingAgentId.value = ''
  agentForm.value = { name: '', kind: 'custom', command: '', args: '', enabled: true }
  agentFormError.value = ''; agentModal.value = true
}
function openAgentEdit(agent: AiAgent) {
  agentModalMode.value = 'edit'; editingAgentId.value = agent.id
  agentForm.value = { name: agent.name, kind: agent.kind, command: agent.command, args: agent.args.join(' '), enabled: agent.enabled }
  agentFormError.value = ''; agentModal.value = true
}
function fillAgentPreset(kind: string) {
  agentForm.value.kind = kind
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
      body: JSON.stringify({ name, kind: agentForm.value.kind, command, args: agentForm.value.args.trim() ? agentForm.value.args.trim().split(/\s+/) : [], enabled: agentForm.value.enabled }),
    })
    agentModal.value = false
    await loadAi()
    flash(agentModalMode.value === 'create' ? 'Agent 已添加，可点击「测试」验证 ACP 握手' : 'Agent 已更新')
  } catch (error) { agentFormError.value = friendly(error) }
  finally { agentSaving.value = false }
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
    <p class="desc">默认使用内置 Sculk Agent（模型直连）；也可以通过 ACP 协议（stdio JSON-RPC）接入 Codex CLI、Claude Code CLI、OpenClaw、Hermes 或其他自定义智能体。</p>
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
          <b>{{ agent.name }}<em style="margin-left:6px;padding:2px 5px;border-radius:4px;color:#8f84d8;background:rgba(156,140,255,.1);font:normal 6px Inter">{{ agentKindLabel(agent.kind) }}</em></b>
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
      <header><b>{{ agentModalMode==='create' ? '添加 ACP Agent' : '编辑 ACP Agent' }}</b><button @click="agentModal=false"><X/></button></header>
      <div class="field"><label>Agent 类型</label><select class="s-select" style="max-width:none" :value="agentForm.kind" @change="fillAgentPreset(($event.target as HTMLSelectElement).value)"><option v-for="kind in AGENT_KINDS" :key="kind.key" :value="kind.key">{{ kind.label }}</option></select></div>
      <div class="field"><label>名称</label><input class="s-input" v-model="agentForm.name" placeholder="例如：Codex CLI"/></div>
      <div class="field"><label>启动命令</label><input class="s-input" v-model="agentForm.command" placeholder="可执行文件或命令名"/><small>{{ agentCommandHint }}</small></div>
      <div class="field"><label>启动参数（空格分隔）</label><input class="s-input" v-model="agentForm.args" placeholder="例如：acp 或 @zed-industries/claude-code-acp"/></div>
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
