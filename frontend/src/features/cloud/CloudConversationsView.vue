<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  Activity, Archive, Bot, ChevronRight, CircleStop, Clock3, FileText,
  Folder, MessageSquareText, Plus, RefreshCw, RotateCcw, Send, ShieldAlert,
  SquareTerminal, TriangleAlert, UserRound,
} from 'lucide-vue-next'
import { CloudApiError, cloudRequest } from './client'
import type {
  AgentTaskOperation, AgentTaskStatus, AgentTaskView, CloudAgent, CloudConversation,
  CloudConversationMessage, CloudTeam,
} from './types'
import './cloud-conversations.css'

interface OperationSpec {
  value: Exclude<AgentTaskOperation, 'task.rollback'>
  label: string
  permission: 'read' | 'write' | 'full'
  risk: 'low' | 'high' | 'critical'
}

interface ConversationDetail {
  conversation: CloudConversation
  messages: CloudConversationMessage[]
}

type ConfirmAction = 'rollback' | 'resume' | 'restart' | 'stop'

const operations: OperationSpec[] = [
  { value: 'shell.exec', label: '执行 Shell 命令', permission: 'full', risk: 'critical' },
  { value: 'host.inspect', label: '检查主机', permission: 'read', risk: 'low' },
  { value: 'workspace.list', label: '列出工作区', permission: 'read', risk: 'low' },
  { value: 'log.tail', label: '读取日志末尾', permission: 'read', risk: 'low' },
  { value: 'workspace.create_directory', label: '创建目录', permission: 'write', risk: 'high' },
  { value: 'server.properties.update', label: '更新服务端配置', permission: 'write', risk: 'high' },
]

const conversations = ref<CloudConversation[]>([])
const messages = ref<CloudConversationMessage[]>([])
const tasks = ref<AgentTaskView[]>([])
const agents = ref<CloudAgent[]>([])
const teams = ref<CloudTeam[]>([])
const selectedTeamId = ref('')
const selectedConversationId = ref('')
const createForm = ref({ title: '', agent_id: '' })
const messageContent = ref('')
const planOpen = ref(false)
const planContent = ref('')
const planAgentId = ref('')
const operation = ref<OperationSpec['value']>('shell.exec')
const shellForm = ref({ command: '', cwd: '', timeout_seconds: 120, confirmed: false })
const pathForm = ref({ path: '.', max_entries: 200, lines: 200, max_bytes: 65536 })
const directoryPath = ref('backups/manual')
const propertiesPath = ref('server.properties')
const propertyChanges = ref('{\n  "motd": "A Sculk server",\n  "max-players": 60\n}')
const confirmation = ref<{ taskId: string; action: ConfirmAction } | null>(null)
const busy = ref('')
const error = ref('')
const refreshWarning = ref('')
let refreshTimer = 0
let refreshInFlight = false
let pendingPlanRequest: { signature: string; idempotencyKey: string } | null = null
const retryIdempotencyKeys = new Map<string, string>()

const currentConversation = computed(() => conversations.value.find(item => item.id === selectedConversationId.value) || null)
const taskAgents = computed(() => agents.value.filter(agent =>
  agent.status === 'active' && agent.online && agent.capabilities.includes('tasks-v1'),
))
const currentOperation = computed(() => operations.find(item => item.value === operation.value) || operations[0])
const planAgent = computed(() => taskAgents.value.find(agent => agent.id === planAgentId.value) || null)
const planAgentReady = computed(() => !!planAgent.value && agentSupports(planAgent.value, currentOperation.value))
const requiresTeamApproval = computed(() => currentOperation.value.risk !== 'low')
const canCreatePlan = computed(() => !!planContent.value.trim() && planAgentReady.value
  && (operation.value !== 'shell.exec' || shellForm.value.confirmed)
  && (!requiresTeamApproval.value || !!selectedTeamId.value) && !busy.value)

watch(taskAgents, items => {
  if (!items.some(agent => agent.id === planAgentId.value)) {
    const preferred = currentConversation.value?.agent_id
    planAgentId.value = items.find(agent => agent.id === preferred)?.id || items[0]?.id || ''
  }
}, { immediate: true })
watch(teams, items => {
  if (selectedTeamId.value && items.some(team => team.id === selectedTeamId.value)) return
  selectedTeamId.value = items.length === 1 ? items[0].id : ''
}, { immediate: true })

watch(selectedConversationId, id => {
  messages.value = []
  confirmation.value = null
  const preferred = conversations.value.find(item => item.id === id)?.agent_id
  if (preferred && taskAgents.value.some(agent => agent.id === preferred)) planAgentId.value = preferred
  if (id) void loadConversation(id)
})

watch(operation, next => {
  shellForm.value.confirmed = false
  if (next === 'log.tail' && pathForm.value.path === '.') pathForm.value.path = 'logs/latest.log'
  if (next === 'workspace.list' && pathForm.value.path === 'logs/latest.log') pathForm.value.path = '.'
})

function normalizeConversationList(value: CloudConversation[] | { conversations: CloudConversation[] }) {
  return Array.isArray(value) ? value : value.conversations
}

function normalizeDetail(value: ConversationDetail | (CloudConversation & { messages?: CloudConversationMessage[] })) {
  if ('conversation' in value) return value
  return { conversation: value, messages: value.messages || [] }
}

function agentSupports(agent: CloudAgent, spec: OperationSpec) {
  if (!agent.permissions.includes(spec.permission) && !agent.permissions.includes('full')) return false
  return spec.value !== 'shell.exec' || agent.capabilities.includes('shell-v1')
}

function eligibility(agent: CloudAgent) {
  if (agentSupports(agent, currentOperation.value)) return '可执行'
  if (!agent.permissions.includes(currentOperation.value.permission) && !agent.permissions.includes('full')) {
    return `缺少${permissionLabel(currentOperation.value.permission)}权限`
  }
  return '缺少 shell-v1 能力'
}

function permissionLabel(permission: string) {
  return ({ read: '读取', write: '写入', process: '进程控制', full: '完整执行' } as Record<string, string>)[permission] || permission
}

function riskLabel(risk: string) {
  return ({ low: '低风险', high: '高风险', critical: '关键风险' } as Record<string, string>)[risk] || risk
}

function operationLabel(value: AgentTaskOperation) {
  return value === 'task.rollback' ? '回滚任务' : operations.find(item => item.value === value)?.label || value
}

function statusLabel(status: AgentTaskStatus) {
  return ({
    awaiting_approval: '等待批准', queued: '等待 Agent', leased: 'Agent 已领取', running: '执行中',
    succeeded: '已成功', failed: '已失败', cancelled: '已取消',
  } as Record<AgentTaskStatus, string>)[status]
}

function isTaskStopping(task: AgentTaskView) {
  return task.status === 'running' && task.cancel_requested
}

function taskStatusLabel(task: AgentTaskView) {
  return isTaskStopping(task) ? '正在停止' : statusLabel(task.status)
}

function executionModeLabel(mode: AgentTaskView['execution_mode']) {
  return ({ original: '首次执行', restart: '从头重执行', resume: '检查点恢复' } as const)[mode]
}

function checkpointKindLabel(kind: 'progress' | 'result') {
  return kind === 'result' ? '结果检查点' : '进度检查点'
}

function confirmationTitle(action: ConfirmAction) {
  return ({
    rollback: '确认创建回滚任务？',
    resume: '确认从检查点恢复？', restart: '确认从头重新执行？', stop: '确认停止运行中的 Shell？',
  } as const)[action]
}

function confirmationDescription(task: AgentTaskView, action: ConfirmAction) {
  if (action === 'stop') return '停止会终止这项 Shell 任务的整个进程树；已经执行的命令及其产生的副作用不会撤销。'
  if (action === 'resume') return '新任务会跳过检查点前已完成的步骤，避免重复已记录的副作用；风险操作仍需批准。'
  if (action === 'restart') return '新任务会从第一步重新执行，可能重复此前已经产生的副作用；风险操作仍需批准。'
  if (task.operation === 'shell.exec') return 'Shell 命令将以 Agent 系统账户权限执行，执行后不可回滚。'
  return '回滚会创建一项新的高风险任务，并再次等待团队批准。'
}

function agentName(id: string) {
  return agents.value.find(agent => agent.id === id)?.name || `Agent ${id.slice(0, 8)}`
}

function formatDate(value?: string | null) {
  if (!value) return '—'
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit',
  }).format(new Date(value))
}

function formatJson(value: unknown) {
  if (typeof value === 'string') return value
  try { return JSON.stringify(value, null, 2) } catch { return String(value) }
}

function readableError(value: unknown) {
  if (!(value instanceof CloudApiError)) return value instanceof Error ? value.message : String(value)
  const known: Record<string, string> = {
    conversation_not_found: '找不到这段对话，请刷新后重试。',
    agent_not_found: '找不到目标 Agent，请刷新后重试。',
    agent_not_active: '目标 Agent 尚未确认或已被撤销。',
    agent_capability_missing: '目标 Agent 不支持这项操作。',
    agent_permission_missing: '目标 Agent 缺少执行这项操作所需的权限。',
    agent_task_team_required: '高风险任务需要选择审批团队；请先创建或加入团队。',
    team_access_denied: '当前账号不是所选团队成员。',
    agent_task_approval_pending: '任务正在等待团队审批，请到“审批”页由其他合资格成员处理。',
    agent_task_approval_rejected: '该任务的团队审批已拒绝或取消。',
    agent_task_approval_invalid: '任务与审批关联无效，请重新创建任务。',
    agent_task_approval_missing: '任务缺少有效审批关联，请重新创建任务。',
    approval_self_forbidden: '审批请求人不能处理自己的审批。',
    agent_task_not_awaiting_approval: '任务状态已经变化，无法再次批准。',
    agent_task_running: '任务已经开始执行，当前不能从云端取消。',
    agent_task_not_cancellable: '当前任务状态不允许取消。',
    agent_task_not_interruptible: '当前任务无法中断；只有正在运行的 Shell 任务支持停止。',
    rollback_not_available: '这项任务没有可用的回滚数据。',
    agent_task_not_terminal: '任务尚未结束，暂时不能恢复或重新执行。',
    resume_checkpoint_unavailable: '当前没有可用于恢复的检查点，请改为从头重新执行。',
    rollback_retry_forbidden: '回滚任务不能恢复或重新执行。',
    idempotency_conflict: '本次操作标识已用于其他请求，请重新操作。',
  }
  return known[value.code] || value.message || `请求失败（HTTP ${value.status}）`
}

async function loadConversation(id: string, quiet = false) {
  try {
    const raw = await cloudRequest<ConversationDetail | (CloudConversation & { messages?: CloudConversationMessage[] })>(`/api/cloud/conversations/${id}`)
    if (id !== selectedConversationId.value) return
    const detail = normalizeDetail(raw)
    messages.value = detail.messages
    await loadMissingLinkedTasks(detail.messages)
    const index = conversations.value.findIndex(item => item.id === detail.conversation.id)
    if (index >= 0) conversations.value[index] = detail.conversation
    else conversations.value.unshift(detail.conversation)
    if (!planAgentId.value && detail.conversation.agent_id) planAgentId.value = detail.conversation.agent_id
  } catch (value) {
    const text = readableError(value)
    if (quiet) refreshWarning.value = `对话刷新失败：${text}`
    else error.value = text
  }
}

async function loadMissingLinkedTasks(items: CloudConversationMessage[]) {
  const known = new Set(tasks.value.map(task => task.id))
  const missing = [...new Set(items
    .map(message => message.linked_task_id)
    .filter((id): id is string => !!id && !known.has(id)))]
  if (!missing.length) return
  const results = await Promise.allSettled(missing.map(id =>
    cloudRequest<AgentTaskView>(`/api/cloud/agent-tasks/${id}`),
  ))
  for (const result of results) {
    if (result.status === 'fulfilled' && !tasks.value.some(task => task.id === result.value.id)) {
      tasks.value.push(result.value)
    }
  }
}

async function loadData(quiet = false) {
  if (refreshInFlight) return
  refreshInFlight = true
  if (!quiet) busy.value = 'refresh'
  try {
    refreshWarning.value = ''
    const [conversationResponse, nextAgents, nextTasks, nextTeams] = await Promise.all([
      cloudRequest<CloudConversation[] | { conversations: CloudConversation[] }>('/api/cloud/conversations'),
      cloudRequest<CloudAgent[]>('/api/cloud/agents'),
      cloudRequest<AgentTaskView[]>('/api/cloud/agent-tasks'),
      cloudRequest<CloudTeam[]>('/api/cloud/teams'),
    ])
    const nextConversations = normalizeConversationList(conversationResponse)
    conversations.value = nextConversations
    agents.value = nextAgents
    tasks.value = nextTasks
    teams.value = nextTeams
    if (!selectedConversationId.value || !nextConversations.some(item => item.id === selectedConversationId.value)) {
      selectedConversationId.value = nextConversations[0]?.id || ''
    } else if (selectedConversationId.value) {
      await loadConversation(selectedConversationId.value, quiet)
    }
    error.value = ''
  } catch (value) {
    const text = readableError(value)
    if (quiet) refreshWarning.value = `状态刷新失败：${text}`
    else error.value = text
  } finally {
    refreshInFlight = false
    if (!quiet) busy.value = ''
  }
}

async function createConversation() {
  if (busy.value) return
  busy.value = 'create-conversation'
  error.value = ''
  try {
    const raw = await cloudRequest<ConversationDetail | (CloudConversation & { messages?: CloudConversationMessage[] })>('/api/cloud/conversations', {
      method: 'POST',
      body: JSON.stringify({
        ...(createForm.value.title.trim() ? { title: createForm.value.title.trim() } : {}),
        ...(createForm.value.agent_id ? { agent_id: createForm.value.agent_id } : {}),
      }),
    })
    const detail = normalizeDetail(raw)
    conversations.value.unshift(detail.conversation)
    selectedConversationId.value = detail.conversation.id
    messages.value = detail.messages
    createForm.value.title = ''
  } catch (value) {
    error.value = readableError(value)
  } finally {
    busy.value = ''
  }
}

async function sendMessage() {
  const content = messageContent.value.trim()
  if (!content || !currentConversation.value || busy.value) return
  busy.value = 'message'
  error.value = ''
  try {
    await cloudRequest<unknown>(`/api/cloud/conversations/${currentConversation.value.id}/messages`, {
      method: 'POST', body: JSON.stringify({ content }),
    })
    messageContent.value = ''
    await loadConversation(currentConversation.value.id)
  } catch (value) {
    error.value = readableError(value)
  } finally {
    busy.value = ''
  }
}

function buildInput(): Record<string, unknown> {
  switch (operation.value) {
    case 'host.inspect': return {}
    case 'workspace.list': return { path: pathForm.value.path, max_entries: Number(pathForm.value.max_entries) }
    case 'log.tail': return { path: pathForm.value.path, lines: Number(pathForm.value.lines), max_bytes: Number(pathForm.value.max_bytes) }
    case 'workspace.create_directory': return { path: directoryPath.value }
    case 'server.properties.update': {
      const changes = JSON.parse(propertyChanges.value) as unknown
      if (!changes || typeof changes !== 'object' || Array.isArray(changes)) throw new Error('配置变更必须是 JSON 对象。')
      return { path: propertiesPath.value, changes }
    }
    case 'shell.exec': {
      if (!shellForm.value.command.trim()) throw new Error('请输入要执行的 Shell 命令。')
      return {
        command: shellForm.value.command,
        ...(shellForm.value.cwd.trim() ? { cwd: shellForm.value.cwd.trim() } : {}),
        timeout_seconds: Number(shellForm.value.timeout_seconds),
      }
    }
  }
}

async function createPlan() {
  if (!canCreatePlan.value || !currentConversation.value || !planAgent.value) return
  busy.value = 'plan'
  error.value = ''
  try {
    const input = buildInput()
    const signature = JSON.stringify({
      conversation_id: currentConversation.value.id,
      content: planContent.value.trim(),
      agent_id: planAgent.value.id,
      operation: operation.value,
      input,
    })
    if (!pendingPlanRequest || pendingPlanRequest.signature !== signature) {
      pendingPlanRequest = {
        signature,
        idempotencyKey: `conversation:${crypto.randomUUID()}`,
      }
    }
    await cloudRequest<unknown>(`/api/cloud/conversations/${currentConversation.value.id}/plans`, {
      method: 'POST',
      body: JSON.stringify({
        content: planContent.value.trim(), agent_id: planAgent.value.id,
        ...(requiresTeamApproval.value ? { team_id: selectedTeamId.value } : {}),
        operation: operation.value, input, idempotency_key: pendingPlanRequest.idempotencyKey,
      }),
    })
    pendingPlanRequest = null
    planContent.value = ''
    shellForm.value.confirmed = false
    planOpen.value = false
    await Promise.all([loadConversation(currentConversation.value.id), loadTasks()])
  } catch (value) {
    error.value = readableError(value)
  } finally {
    busy.value = ''
  }
}

async function loadTasks() {
  tasks.value = await cloudRequest<AgentTaskView[]>('/api/cloud/agent-tasks')
}

function linkedTask(message: CloudConversationMessage) {
  return message.linked_task_id ? tasks.value.find(task => task.id === message.linked_task_id) || null : null
}

async function performAction(task: AgentTaskView, action: 'cancel' | 'rollback') {
  if (action === 'rollback'
    && (confirmation.value?.taskId !== task.id || confirmation.value.action !== action)) {
    confirmation.value = { taskId: task.id, action }
    return
  }
  busy.value = `${action}:${task.id}`
  error.value = ''
  try {
    const updated = await cloudRequest<AgentTaskView>(`/api/cloud/agent-tasks/${task.id}/${action}`, { method: 'POST' })
    const index = tasks.value.findIndex(item => item.id === updated.id)
    if (index >= 0) tasks.value[index] = updated
    else tasks.value.unshift(updated)
    confirmation.value = null
  } catch (value) {
    error.value = readableError(value)
    confirmation.value = null
    try { await loadTasks() } catch {}
  } finally {
    busy.value = ''
  }
}

async function retryTask(task: AgentTaskView, mode: 'resume' | 'restart') {
  if (confirmation.value?.taskId !== task.id || confirmation.value.action !== mode) {
    confirmation.value = { taskId: task.id, action: mode }
    return
  }
  const key = `${task.id}:${mode}`
  const idempotencyKey = retryIdempotencyKeys.get(key) || `retry:${crypto.randomUUID()}`
  retryIdempotencyKeys.set(key, idempotencyKey)
  busy.value = `${mode}:${task.id}`
  error.value = ''
  let created: AgentTaskView
  try {
    created = await cloudRequest<AgentTaskView>(`/api/cloud/agent-tasks/${task.id}/retry`, {
      method: 'POST',
      body: JSON.stringify({ mode, idempotency_key: idempotencyKey }),
    })
  } catch (value) {
    error.value = readableError(value)
    confirmation.value = null
    try { await loadTasks() } catch {}
    busy.value = ''
    return
  }
  retryIdempotencyKeys.delete(key)
  tasks.value = [created, ...tasks.value.filter(item => item.id !== created.id)]
  confirmation.value = null
  try {
    await loadTasks()
    if (currentConversation.value) await loadConversation(currentConversation.value.id)
  } catch (value) {
    refreshWarning.value = `新任务已创建，但刷新对话失败：${readableError(value)}`
  }
  busy.value = ''
}

function confirmTaskAction(task: AgentTaskView, action: ConfirmAction) {
  if (action === 'resume' || action === 'restart') void retryTask(task, action)
  else if (action === 'stop') void performAction(task, 'cancel')
  else void performAction(task, action)
}

function requestTaskStop(task: AgentTaskView) {
  confirmation.value = { taskId: task.id, action: 'stop' }
}

onMounted(() => {
  void loadData()
  refreshTimer = window.setInterval(() => void loadData(true), 3000)
})
onUnmounted(() => window.clearInterval(refreshTimer))
</script>

<template>
  <section class="cloud-conversations">
    <article class="cloud-panel conversation-hero">
      <span><MessageSquareText/></span><div><small>CONVERSATIONS</small><h3>对话与执行计划</h3><p>记录主机操作意图，将计划转换为可追踪任务，并在对话中完成审批。</p></div>
      <button class="cloud-icon-btn" title="刷新对话" :disabled="busy==='refresh'" @click="loadData()"><RefreshCw :class="{'s-spin':busy==='refresh'}"/></button>
    </article>
    <div v-if="error" class="conversation-notice error"><TriangleAlert/>{{error}}</div>
    <div v-if="refreshWarning" class="conversation-notice"><Clock3/>{{refreshWarning}}</div>

    <article class="cloud-panel conversation-create">
      <form @submit.prevent="createConversation">
        <label>新对话名称<input v-model="createForm.title" maxlength="80" placeholder="例如：检查服务器状态"/></label>
        <label>默认 Agent（可选）<select v-model="createForm.agent_id"><option value="">稍后选择</option><option v-for="agent in taskAgents" :key="agent.id" :value="agent.id">{{agent.name}} · {{agent.workspace_label}}</option></select></label>
        <button class="cloud-primary" :disabled="!!busy"><Plus/>新建对话</button>
      </form>
    </article>

    <div class="conversation-layout">
      <article class="cloud-panel conversation-list">
        <header><div><h3>对话</h3><p>{{conversations.length}} 段记录</p></div><MessageSquareText/></header>
        <button v-for="item in conversations" :key="item.id" :class="{active:selectedConversationId===item.id}" @click="selectedConversationId=item.id">
          <span><MessageSquareText/></span><p><b>{{item.title||'新对话'}}</b><small>{{item.agent_id?agentName(item.agent_id):'尚未指定 Agent'}} · {{formatDate(item.updated_at)}}</small></p><ChevronRight/>
        </button>
        <div v-if="!conversations.length" class="conversation-empty"><MessageSquareText/>新建对话后即可发送消息和执行计划</div>
      </article>

      <article class="cloud-panel conversation-thread">
        <header><div><h3>{{currentConversation?.title||'选择一段对话'}}</h3><p v-if="currentConversation">{{currentConversation.agent_id?agentName(currentConversation.agent_id):'计划可单独选择 Agent'}}</p><p v-else>查看消息与任务状态</p></div><Bot/></header>

        <div v-if="currentConversation" class="message-stream">
          <article v-for="message in messages" :key="message.id" class="conversation-message" :class="[message.role,message.kind]">
            <span><UserRound v-if="message.role==='user'"/><Bot v-else/></span>
            <div class="message-body"><header><b>{{message.role==='user'?'你':message.role==='assistant'?'助手':'系统'}}</b><time>{{formatDate(message.created_at)}}</time></header><p>{{message.content}}</p>
              <section v-if="message.kind==='plan'" class="conversation-plan-card">
                <template v-if="linkedTask(message)">
                  <header><span><SquareTerminal/>{{operationLabel(linkedTask(message)!.operation)}}</span><em :class="[linkedTask(message)!.status,{stopping:isTaskStopping(linkedTask(message)!)}]">{{taskStatusLabel(linkedTask(message)!)}}</em></header>
                  <div class="plan-meta"><span>{{agentName(linkedTask(message)!.agent_id)}}</span><span>{{riskLabel(linkedTask(message)!.risk)}}</span><span>{{permissionLabel(linkedTask(message)!.required_permission)}}权限</span><span>第 {{linkedTask(message)!.attempt_no}} 次</span><span>{{executionModeLabel(linkedTask(message)!.execution_mode)}}</span><span v-if="linkedTask(message)!.team_id">审批团队已绑定</span><span v-if="linkedTask(message)!.cancel_requested_at">停止请求 {{formatDate(linkedTask(message)!.cancel_requested_at)}}</span></div>
                  <div v-if="linkedTask(message)!.latest_checkpoint" class="plan-checkpoint"><Clock3/><p><b>最近检查点 · #{{linkedTask(message)!.latest_checkpoint!.seq}}</b><small>{{checkpointKindLabel(linkedTask(message)!.latest_checkpoint!.kind)}} · {{formatDate(linkedTask(message)!.latest_checkpoint!.created_at)}} · {{linkedTask(message)!.latest_checkpoint!.resumable?'可用于恢复':'仅供记录'}}</small></p></div>
                  <div v-if="confirmation?.taskId===linkedTask(message)!.id" class="plan-confirm">
                    <ShieldAlert/><p><b>{{confirmationTitle(confirmation.action)}}</b><small>{{confirmationDescription(linkedTask(message)!,confirmation.action)}}</small></p>
                    <button @click="confirmation=null">返回</button><button class="confirm" :disabled="!!busy" @click="confirmTaskAction(linkedTask(message)!,confirmation.action)">确认</button>
                  </div>
                  <div v-else class="plan-actions">
                    <span v-if="linkedTask(message)!.status==='awaiting_approval'" class="plan-approval-note"><Clock3/>等待审批团队的其他所有者、管理员或审批人在“远程审批”页处理（申请人不得自批）</span>
                    <button v-if="['awaiting_approval','queued','leased'].includes(linkedTask(message)!.status)" :disabled="!!busy" @click="performAction(linkedTask(message)!,'cancel')"><CircleStop/>取消任务</button>
                    <button v-if="linkedTask(message)!.status==='running'&&linkedTask(message)!.operation==='shell.exec'&&!linkedTask(message)!.cancel_requested" class="stop" :disabled="!!busy" @click="requestTaskStop(linkedTask(message)!)"><CircleStop/>停止运行</button>
                    <button v-if="linkedTask(message)!.status==='succeeded'&&linkedTask(message)!.rollback_available&&linkedTask(message)!.operation!=='shell.exec'" :disabled="!!busy" @click="performAction(linkedTask(message)!,'rollback')"><RotateCcw/>创建回滚任务</button>
                    <button v-if="linkedTask(message)!.operation!=='task.rollback'&&['failed','cancelled'].includes(linkedTask(message)!.status)&&linkedTask(message)!.can_resume" class="resume" :disabled="!!busy" @click="retryTask(linkedTask(message)!,'resume')"><SquareTerminal/>从检查点恢复</button>
                    <button v-if="linkedTask(message)!.operation!=='task.rollback'&&['failed','cancelled','succeeded'].includes(linkedTask(message)!.status)" class="restart" :disabled="!!busy" @click="retryTask(linkedTask(message)!,'restart')"><RefreshCw/>从头重新执行</button>
                    <span v-if="isTaskStopping(linkedTask(message)!)" class="plan-stopping-note"><RefreshCw class="s-spin"/>停止请求已发送，正在等待 Agent 结束进程。</span>
                    <span v-else-if="linkedTask(message)!.status==='running'&&linkedTask(message)!.operation!=='shell.exec'">任务正在 Agent 上执行，当前不能从云端强制终止。</span>
                    <span v-else-if="linkedTask(message)!.operation==='shell.exec'&&linkedTask(message)!.status==='succeeded'">Shell 任务不提供回滚。</span>
                  </div>
                  <section v-if="linkedTask(message)!.error" class="plan-result error"><TriangleAlert/><pre>{{linkedTask(message)!.error}}</pre></section>
                  <section v-if="linkedTask(message)!.output!==null&&linkedTask(message)!.output!==undefined" class="plan-result"><FileText/><pre>{{formatJson(linkedTask(message)!.output)}}</pre></section>
                  <section v-if="linkedTask(message)!.artifacts?.length" class="plan-artifacts"><h4><Archive/>产物</h4><div v-for="artifact in linkedTask(message)!.artifacts" :key="`${artifact.path}:${artifact.name}`"><Folder/><p><b>{{artifact.name}}</b><small>{{artifact.path}}</small></p></div></section>
                  <details class="plan-events"><summary><Activity/>任务事件（{{linkedTask(message)!.events.length}}）</summary><div><article v-for="event in linkedTask(message)!.events" :key="event.seq" :class="event.level"><time>#{{event.seq}} · {{formatDate(event.created_at)}}</time><p>{{event.message}}</p><pre v-if="event.data!==null&&event.data!==undefined">{{formatJson(event.data)}}</pre></article></div></details>
                </template>
                <div v-else class="plan-awaiting"><RefreshCw class="s-spin"/>正在读取真实任务状态</div>
              </section>
            </div>
          </article>
          <div v-if="!messages.length" class="conversation-empty"><Bot/>发送消息，或创建一项可审批的执行计划</div>
        </div>

        <div v-if="currentConversation && planOpen" class="plan-composer">
          <header><div><h3>创建执行计划</h3><p>计划会生成真实远程任务；高风险与 Shell 操作须由其他合资格团队成员在“远程审批”页批准，申请人不得自批。</p></div><button @click="planOpen=false">关闭</button></header>
          <form @submit.prevent="createPlan">
            <label class="wide">计划说明<textarea v-model="planContent" rows="3" maxlength="2000" placeholder="说明这项操作的目标和预期结果" required/></label>
            <label>目标 Agent<select v-model="planAgentId" required><option value="" disabled>选择在线 Agent</option><option v-for="agent in taskAgents" :key="agent.id" :value="agent.id">{{agent.name}} · {{eligibility(agent)}}</option></select></label>
            <label>操作<select v-model="operation"><option v-for="item in operations" :key="item.value" :value="item.value">{{item.label}} · {{riskLabel(item.risk)}}</option></select></label>
            <label v-if="requiresTeamApproval">审批团队<select v-model="selectedTeamId" required><option value="" disabled>{{teams.length?'选择审批团队':'尚未加入团队'}}</option><option v-for="team in teams" :key="team.id" :value="team.id">{{team.name}} · {{team.role}}</option></select></label>
            <div class="plan-operation wide"><ShieldAlert/><span>需要{{permissionLabel(currentOperation.permission)}}权限 · {{riskLabel(currentOperation.risk)}}</span></div>
            <template v-if="operation==='shell.exec'">
              <label class="wide">Shell 命令<textarea v-model="shellForm.command" rows="4" maxlength="32768" spellcheck="false" required/></label>
              <label>工作目录（可选）<input v-model="shellForm.cwd" maxlength="1024" placeholder="留空使用 Agent 工作目录"/></label>
              <label>超时秒数<input v-model.number="shellForm.timeout_seconds" type="number" min="1" max="1800" required/></label>
              <label class="plan-shell-confirm wide"><input v-model="shellForm.confirmed" type="checkbox"/><span><b>确认完整系统账户权限</b><small>Shell 会继承 Agent 进程所属系统账户的权限；计划创建后仍需人工批准，执行后不可回滚。</small></span></label>
            </template>
            <template v-else-if="operation==='workspace.list'"><label>相对路径<input v-model="pathForm.path" maxlength="512" required/></label><label>最多条目<input v-model.number="pathForm.max_entries" type="number" min="1" max="500" required/></label></template>
            <template v-else-if="operation==='log.tail'"><label class="wide">日志相对路径<input v-model="pathForm.path" maxlength="512" required/></label><label>读取行数<input v-model.number="pathForm.lines" type="number" min="1" max="1000" required/></label><label>最大字节<input v-model.number="pathForm.max_bytes" type="number" min="1" max="262144" required/></label></template>
            <template v-else-if="operation==='workspace.create_directory'"><label class="wide">目录相对路径<input v-model="directoryPath" maxlength="512" required/></label></template>
            <template v-else-if="operation==='server.properties.update'"><label class="wide">配置文件相对路径<input v-model="propertiesPath" maxlength="512" required/></label><label class="wide">配置变更（JSON）<textarea v-model="propertyChanges" rows="4" spellcheck="false" required/></label></template>
            <div v-if="planAgent&&!planAgentReady" class="plan-ineligible wide"><TriangleAlert/>{{eligibility(planAgent)}}，请选择其他 Agent。</div>
            <button class="cloud-primary wide" :disabled="!canCreatePlan"><RefreshCw v-if="busy==='plan'" class="s-spin"/><SquareTerminal v-else/>创建计划任务</button>
          </form>
        </div>

        <form v-if="currentConversation" class="message-composer" @submit.prevent="sendMessage">
          <textarea v-model="messageContent" rows="2" maxlength="4000" placeholder="输入消息…" required/>
          <button type="button" @click="planOpen=!planOpen"><SquareTerminal/>{{planOpen?'收起计划':'执行计划'}}</button>
          <button class="cloud-primary" :disabled="!messageContent.trim()||!!busy"><Send/>发送</button>
        </form>
        <div v-else class="conversation-empty thread"><MessageSquareText/>请先选择或新建一段对话</div>
      </article>
    </div>
  </section>
</template>
