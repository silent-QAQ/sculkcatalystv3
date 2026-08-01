<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  Activity, Archive, ChevronRight, CircleStop, Clock3, FileText,
  Folder, List, Play, RefreshCw, RotateCcw, ShieldAlert, SquareTerminal, TriangleAlert,
} from 'lucide-vue-next'
import { CloudApiError, cloudRequest } from './client'
import type {
  AgentTaskOperation, AgentTaskStatus, AgentTaskView, CloudAgent, CloudTeam,
} from './types'
import './agent-tasks.css'

interface OperationSpec {
  value: Exclude<AgentTaskOperation, 'task.rollback'>
  label: string
  description: string
  permission: 'read' | 'write' | 'full'
  risk: 'low' | 'high' | 'critical'
}

type ConfirmAction = 'rollback' | 'resume' | 'restart' | 'stop'

const operations: OperationSpec[] = [
  { value: 'shell.exec', label: '执行 Shell 命令', description: '使用 Agent 所属系统账户执行命令', permission: 'full', risk: 'critical' },
  { value: 'host.inspect', label: '检查主机', description: '读取系统与工作区概况', permission: 'read', risk: 'low' },
  { value: 'workspace.list', label: '列出工作区', description: '查看工作区内的文件和目录', permission: 'read', risk: 'low' },
  { value: 'log.tail', label: '读取日志末尾', description: '读取指定日志的最后若干行', permission: 'read', risk: 'low' },
  { value: 'workspace.create_directory', label: '创建目录', description: '在 Agent 工作区内创建相对目录', permission: 'write', risk: 'high' },
  { value: 'server.properties.update', label: '更新服务器配置', description: '更新 server.properties 的受支持字段', permission: 'write', risk: 'high' },
]

const tasks = ref<AgentTaskView[]>([])
const agents = ref<CloudAgent[]>([])
const teams = ref<CloudTeam[]>([])
const selectedTaskId = ref('')
const selectedAgentId = ref('')
const selectedTeamId = ref('')
const operation = ref<OperationSpec['value']>('shell.exec')
const busy = ref('')
const error = ref('')
const refreshWarning = ref('')
const confirmation = ref<{ taskId: string; action: ConfirmAction } | null>(null)
const refreshInFlight = ref(false)
const retryIdempotencyKeys = new Map<string, string>()
let refreshTimer = 0

const shellForm = ref({ command: '', cwd: '', timeout_seconds: 120, confirmed: false })
const pathForm = ref({ path: '.', max_entries: 200, lines: 200, max_bytes: 65536 })
const directoryPath = ref('backups/manual')
const propertiesPath = ref('server.properties')
const propertyChanges = ref('{\n  "motd": "A Sculk server",\n  "max-players": 60\n}')
const idempotencyKey = ref(newIdempotencyKey())

const taskAgents = computed(() => agents.value.filter(agent =>
  agent.status === 'active' && agent.online && agent.capabilities.includes('tasks-v1'),
))
const currentOperation = computed(() => operations.find(item => item.value === operation.value) || operations[0])
const selectedAgent = computed(() => taskAgents.value.find(agent => agent.id === selectedAgentId.value) || null)
const selectedTask = computed(() => tasks.value.find(task => task.id === selectedTaskId.value) || null)
const activeTaskCount = computed(() => tasks.value.filter(task =>
  ['awaiting_approval', 'queued', 'leased', 'running'].includes(task.status),
).length)
const awaitingApprovalCount = computed(() => tasks.value.filter(task => task.status === 'awaiting_approval').length)
const selectedAgentReady = computed(() => selectedAgent.value ? agentSupports(selectedAgent.value, currentOperation.value) : false)
const shellReady = computed(() => operation.value !== 'shell.exec' || shellForm.value.confirmed)
const requiresTeamApproval = computed(() => currentOperation.value.risk !== 'low')
const canSubmit = computed(() => !!selectedAgent.value && selectedAgentReady.value && shellReady.value
  && (!requiresTeamApproval.value || !!selectedTeamId.value) && !busy.value)

watch(taskAgents, items => {
  if (!items.some(agent => agent.id === selectedAgentId.value)) selectedAgentId.value = items[0]?.id || ''
}, { immediate: true })
watch(teams, items => {
  if (selectedTeamId.value && items.some(team => team.id === selectedTeamId.value)) return
  selectedTeamId.value = items.length === 1 ? items[0].id : ''
}, { immediate: true })
watch(operation, next => {
  confirmation.value = null
  if (next === 'shell.exec') shellForm.value.confirmed = false
  if (next === 'log.tail' && pathForm.value.path === '.') pathForm.value.path = 'logs/latest.log'
  if (next === 'workspace.list' && pathForm.value.path === 'logs/latest.log') pathForm.value.path = '.'
})

function newIdempotencyKey() {
  return `web:${crypto.randomUUID()}`
}

function agentSupports(agent: CloudAgent, spec: OperationSpec) {
  if (!agent.permissions.includes(spec.permission) && !agent.permissions.includes('full')) return false
  return spec.value !== 'shell.exec' || agent.capabilities.includes('shell-v1')
}

function agentEligibility(agent: CloudAgent) {
  if (agentSupports(agent, currentOperation.value)) return '可执行'
  if (!agent.permissions.includes(currentOperation.value.permission) && !agent.permissions.includes('full')) return `缺少 ${permissionLabel(currentOperation.value.permission)}权限`
  return '缺少 shell-v1 能力'
}

function operationLabel(value: AgentTaskOperation) {
  if (value === 'task.rollback') return '回滚任务'
  return operations.find(item => item.value === value)?.label || value
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

function permissionLabel(permission: string) {
  return ({ read: '读取', write: '写入', process: '进程控制', full: '完整执行' } as Record<string, string>)[permission] || permission
}

function riskLabel(risk: string) {
  return ({ low: '低风险', high: '高风险', critical: '关键风险' } as Record<string, string>)[risk] || risk
}

function executionModeLabel(mode: AgentTaskView['execution_mode']) {
  return ({ original: '首次执行', restart: '从头重新执行', resume: '从检查点恢复' } as const)[mode]
}

function checkpointKindLabel(kind: 'progress' | 'result') {
  return kind === 'result' ? '结果检查点' : '进度检查点'
}

function taskSource(task: AgentTaskView) {
  if (task.retry_of_task_id) return { label: task.execution_mode === 'resume' ? '恢复来源' : '重执行来源', id: task.retry_of_task_id }
  if (task.rollback_source_task_id) return { label: '回滚来源', id: task.rollback_source_task_id }
  if (task.source_task_id) return { label: '来源任务', id: task.source_task_id }
  return null
}

function confirmationTitle(action: ConfirmAction) {
  return ({
    rollback: '确认创建回滚任务？',
    resume: '确认从检查点恢复？', restart: '确认从头重新执行？', stop: '确认停止运行中的 Shell？',
  } as const)[action]
}

function confirmationDescription(task: AgentTaskView, action: ConfirmAction) {
  if (action === 'stop') return '停止会终止这项 Shell 任务的整个进程树；已经执行的命令及其产生的副作用不会撤销。'
  if (action === 'resume') return '新任务会从最近的可恢复检查点继续，并跳过检查点前已完成的步骤，避免重复已记录的副作用；风险操作仍需批准。'
  if (action === 'restart') return '新任务会从第一步重新执行，可能重复此前已经产生的副作用；风险操作仍需批准。'
  if (task.operation === 'shell.exec') return '该 Shell 命令将以 Agent 系统账户权限执行，且执行后不可回滚。'
  return '回滚会创建一条新的高风险任务，并再次等待团队批准。'
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

function formatBytes(value?: number) {
  if (value === undefined) return ''
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GB`
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MB`
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KB`
  return `${value} B`
}

function formatJson(value: unknown) {
  if (typeof value === 'string') return value
  try { return JSON.stringify(value, null, 2) } catch { return String(value) }
}

function taskSummary(task: AgentTaskView) {
  if (task.operation === 'shell.exec') return String(task.input.command || '').split(/\r?\n/, 1)[0] || 'Shell 命令'
  if ('path' in task.input) return String(task.input.path)
  if (task.operation === 'host.inspect') return '主机与工作区信息'
  if (task.operation === 'task.rollback') return `来源任务 ${String(task.input.source_task_id || '').slice(0, 8)}`
  return operationLabel(task.operation)
}

function eventMessage(message: string) {
  return ({
    'Task created': '任务已创建',
    'Task approved': '任务已批准，等待 Agent 领取',
    'Task cancelled by user': '用户已取消任务',
    'Rollback task created and awaiting approval': '回滚任务已创建，等待批准',
  } as Record<string, string>)[message] || message
}

function readableError(value: unknown) {
  if (!(value instanceof CloudApiError)) return value instanceof Error ? value.message : String(value)
  const known: Record<string, string> = {
    agent_not_found: '找不到所选 Agent，请刷新后重试。',
    agent_not_active: '该 Agent 尚未确认或已被撤销。',
    agent_capability_missing: '该 Agent 没有执行此操作所需的能力。',
    agent_permission_missing: '该 Agent 没有执行此操作所需的权限。',
    agent_task_team_required: '高风险任务需要选择审批团队；请先创建或加入团队。',
    team_access_denied: '当前账号不是所选团队成员。',
    agent_task_approval_pending: '任务正在等待团队审批，请到“审批”页由其他合资格成员处理。',
    agent_task_approval_rejected: '该任务的团队审批已拒绝或取消。',
    agent_task_approval_invalid: '任务与审批关联无效，请重新创建任务。',
    agent_task_approval_missing: '任务缺少有效审批关联，请重新创建任务。',
    approval_self_forbidden: '审批请求人不能处理自己的审批。',
    agent_task_not_awaiting_approval: '任务状态已变化，无法再次批准。',
    agent_task_running: '任务已经开始执行，当前不能从云端取消。',
    agent_task_not_cancellable: '当前任务状态不允许取消。',
    agent_task_not_interruptible: '当前任务无法中断；只有正在运行的 Shell 任务支持停止。',
    rollback_not_available: '该任务没有可用的回滚数据。',
    agent_task_not_terminal: '任务尚未结束，暂时不能恢复或重新执行。',
    resume_checkpoint_unavailable: '当前没有可用于恢复的检查点，请改为从头重新执行。',
    rollback_retry_forbidden: '回滚任务不能恢复或重新执行。',
    idempotency_conflict: '本次提交标识已用于其他任务，请重新提交。',
  }
  return known[value.code] || value.message || `请求失败（HTTP ${value.status}）`
}

async function loadData(quiet = false) {
  if (refreshInFlight.value) return
  refreshInFlight.value = true
  if (!quiet) busy.value = 'refresh'
  try {
    const [nextAgents, nextTasks, nextTeams] = await Promise.all([
      cloudRequest<CloudAgent[]>('/api/cloud/agents'),
      cloudRequest<AgentTaskView[]>('/api/cloud/agent-tasks'),
      cloudRequest<CloudTeam[]>('/api/cloud/teams'),
    ])
    agents.value = nextAgents
    tasks.value = nextTasks
    teams.value = nextTeams
    if (!selectedTaskId.value || !nextTasks.some(task => task.id === selectedTaskId.value)) {
      selectedTaskId.value = nextTasks[0]?.id || ''
    }
    error.value = ''
    refreshWarning.value = ''
  } catch (value) {
    const text = readableError(value)
    if (quiet) refreshWarning.value = `状态刷新失败：${text}`
    else error.value = text
  } finally {
    refreshInFlight.value = false
    if (!quiet) busy.value = ''
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
      if (!shellForm.value.command.length) throw new Error('请输入要执行的 Shell 命令。')
      const input: Record<string, unknown> = {
        command: shellForm.value.command,
        timeout_seconds: Number(shellForm.value.timeout_seconds),
      }
      if (shellForm.value.cwd.trim()) input.cwd = shellForm.value.cwd.trim()
      return input
    }
  }
}

async function createTask() {
  if (!canSubmit.value || !selectedAgent.value) return
  busy.value = 'create'
  error.value = ''
  try {
    const task = await cloudRequest<AgentTaskView>('/api/cloud/agent-tasks', {
      method: 'POST',
      body: JSON.stringify({
        agent_id: selectedAgent.value.id,
        ...(requiresTeamApproval.value ? { team_id: selectedTeamId.value } : {}),
        operation: operation.value,
        input: buildInput(),
        idempotency_key: idempotencyKey.value,
      }),
    })
    tasks.value = [task, ...tasks.value.filter(item => item.id !== task.id)]
    selectedTaskId.value = task.id
    idempotencyKey.value = newIdempotencyKey()
    if (operation.value === 'shell.exec') shellForm.value.confirmed = false
  } catch (value) {
    error.value = readableError(value)
  } finally {
    busy.value = ''
  }
}

function updateTask(task: AgentTaskView) {
  const index = tasks.value.findIndex(item => item.id === task.id)
  if (index >= 0) tasks.value[index] = task
  else tasks.value.unshift(task)
  selectedTaskId.value = task.id
}

async function performAction(task: AgentTaskView, action: ConfirmAction | 'cancel') {
  if (action === 'rollback'
    && (confirmation.value?.taskId !== task.id || confirmation.value.action !== action)) {
    confirmation.value = { taskId: task.id, action }
    return
  }
  busy.value = `${action}:${task.id}`
  error.value = ''
  try {
    const updated = await cloudRequest<AgentTaskView>(`/api/cloud/agent-tasks/${task.id}/${action}`, { method: 'POST' })
    updateTask(updated)
    confirmation.value = null
  } catch (value) {
    error.value = readableError(value)
    confirmation.value = null
    await loadData(true)
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
  try {
    const created = await cloudRequest<AgentTaskView>(`/api/cloud/agent-tasks/${task.id}/retry`, {
      method: 'POST',
      body: JSON.stringify({ mode, idempotency_key: idempotencyKey }),
    })
    retryIdempotencyKeys.delete(key)
    updateTask(created)
    confirmation.value = null
  } catch (value) {
    error.value = readableError(value)
    confirmation.value = null
    await loadData(true)
  } finally {
    busy.value = ''
  }
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
  <section class="agent-tasks">
    <article class="cloud-panel task-console-hero">
      <span><SquareTerminal/></span>
      <div><small>REMOTE TASKS</small><h3>远程任务控制台</h3><p>向在线 Agent 提交主机任务，并在云端查看真实事件、输出和产物。</p></div>
      <div class="task-console-stats"><b>{{activeTaskCount}}</b><small>进行中</small><b>{{awaitingApprovalCount}}</b><small>待批准</small></div>
      <button class="cloud-icon-btn" title="刷新任务" :disabled="busy==='refresh'" @click="loadData()"><RefreshCw :class="{'s-spin':busy==='refresh'}"/></button>
    </article>

    <div v-if="error" class="agent-task-error"><TriangleAlert/>{{error}}</div>
    <div v-if="refreshWarning" class="agent-task-warning"><Clock3/>{{refreshWarning}}；当前显示上次成功获取的数据。</div>

    <article class="cloud-panel task-create-panel">
      <header><div><h3>创建远程任务</h3><p>仅显示在线并支持 tasks-v1 的 Agent</p></div><Play/></header>
      <form class="task-create-form" @submit.prevent="createTask">
        <label>目标 Agent
          <select v-model="selectedAgentId" required>
            <option value="" disabled>{{taskAgents.length ? '选择在线 Agent' : '暂无可执行任务的在线 Agent'}}</option>
            <option v-for="agent in taskAgents" :key="agent.id" :value="agent.id">{{agent.name}} · {{agent.workspace_label}} · {{agentEligibility(agent)}}</option>
          </select>
        </label>
        <label>操作
          <select v-model="operation">
            <option v-for="item in operations" :key="item.value" :value="item.value">{{item.label}} · {{riskLabel(item.risk)}}</option>
          </select>
        </label>
        <label v-if="requiresTeamApproval">审批团队
          <select v-model="selectedTeamId" required>
            <option value="" disabled>{{teams.length ? '选择审批团队' : '尚未加入团队'}}</option>
            <option v-for="team in teams" :key="team.id" :value="team.id">{{team.name}} · {{team.role}}</option>
          </select>
        </label>
        <div class="task-operation-note"><ShieldAlert/><span><b>{{currentOperation.description}}</b><small>需要 {{permissionLabel(currentOperation.permission)}}权限 · {{riskLabel(currentOperation.risk)}}</small></span></div>

        <template v-if="operation==='shell.exec'">
          <label class="wide">Shell 命令
            <textarea v-model="shellForm.command" rows="5" maxlength="32768" spellcheck="false" placeholder="输入要在 Agent 主机上执行的完整命令" required/>
          </label>
          <label>工作目录（可选）<input v-model="shellForm.cwd" maxlength="1024" placeholder="留空使用 Agent 的 workspace-root"/></label>
          <label>超时秒数<input v-model.number="shellForm.timeout_seconds" type="number" min="1" max="1800" required/></label>
          <label class="shell-confirm wide"><input v-model="shellForm.confirmed" type="checkbox"/><span><b>确认系统账户权限</b><small>Shell 命令将继承 Agent 进程所属系统账户的权限，可能读取、修改或删除该账户可访问的数据。命令内容不会被前端改写；提交后仍需人工批准才会执行。</small></span></label>
        </template>
        <template v-else-if="operation==='workspace.list'">
          <label>相对路径<input v-model="pathForm.path" maxlength="512" placeholder="." required/></label>
          <label>最多条目<input v-model.number="pathForm.max_entries" type="number" min="1" max="500" required/></label>
        </template>
        <template v-else-if="operation==='log.tail'">
          <label class="wide">日志相对路径<input v-model="pathForm.path" maxlength="512" placeholder="logs/latest.log" required/></label>
          <label>读取行数<input v-model.number="pathForm.lines" type="number" min="1" max="1000" required/></label>
          <label>最大字节<input v-model.number="pathForm.max_bytes" type="number" min="1" max="262144" required/></label>
        </template>
        <template v-else-if="operation==='workspace.create_directory'">
          <label class="wide">目录相对路径<input v-model="directoryPath" maxlength="512" placeholder="backups/manual" required/></label>
        </template>
        <template v-else-if="operation==='server.properties.update'">
          <label class="wide">配置文件相对路径<input v-model="propertiesPath" maxlength="512" placeholder="server.properties" required/></label>
          <label class="wide">配置变更（JSON）<textarea v-model="propertyChanges" rows="5" spellcheck="false" required/></label>
          <small class="wide property-hint">支持 motd、max-players、difficulty、gamemode、pvp、white-list、view-distance 和 simulation-distance。</small>
        </template>

        <div v-if="selectedAgent && !selectedAgentReady" class="agent-ineligible wide"><TriangleAlert/>{{agentEligibility(selectedAgent)}}，请选择其他 Agent 或重新配对并授予所需能力。</div>
        <button class="cloud-primary task-submit wide" :disabled="!canSubmit">
          <RefreshCw v-if="busy==='create'" class="s-spin"/><SquareTerminal v-else/>
          {{operation==='shell.exec'?'创建 Shell 任务并等待团队批准':currentOperation.risk==='low'?'创建并加入队列':'创建任务并等待团队批准'}}
        </button>
      </form>
    </article>

    <div class="task-console-grid">
      <article class="cloud-panel task-list-panel">
        <header><div><h3>任务记录</h3><p>约每 3 秒刷新一次</p></div><List/></header>
        <button v-for="task in tasks" :key="task.id" class="task-list-item" :class="[task.status,{selected:selectedTaskId===task.id,stopping:isTaskStopping(task)}]" @click="selectedTaskId=task.id;confirmation=null">
          <span class="task-status-dot"><Activity/></span>
          <p><b>{{operationLabel(task.operation)}} · 第 {{task.attempt_no}} 次</b><small>{{agentName(task.agent_id)}} · {{taskSummary(task)}}</small></p>
          <em>{{taskStatusLabel(task)}}</em><ChevronRight/>
        </button>
        <div v-if="!tasks.length" class="cloud-empty"><Clock3/>尚无远程任务</div>
      </article>

      <article v-if="selectedTask" class="cloud-panel task-detail-panel">
        <header class="task-detail-head">
          <div><span :class="`risk ${selectedTask.risk}`">{{riskLabel(selectedTask.risk)}}</span><h3>{{operationLabel(selectedTask.operation)}}</h3><p>{{agentName(selectedTask.agent_id)}} · {{selectedTask.id}}</p></div>
          <em :class="[selectedTask.status,{stopping:isTaskStopping(selectedTask)}]">{{taskStatusLabel(selectedTask)}}</em>
        </header>

        <div v-if="confirmation?.taskId===selectedTask.id" class="task-action-confirm">
          <ShieldAlert/><p><b>{{confirmationTitle(confirmation.action)}}</b><small>{{confirmationDescription(selectedTask,confirmation.action)}}</small></p>
          <button @click="confirmation=null">返回</button><button class="confirm" :disabled="!!busy" @click="confirmTaskAction(selectedTask,confirmation.action)">确认</button>
        </div>
        <div v-else class="task-detail-actions">
          <span v-if="selectedTask.status==='awaiting_approval'" class="task-approval-note"><Clock3/>等待审批团队处理（请求人不能自批）</span>
          <button v-if="['awaiting_approval','queued','leased'].includes(selectedTask.status)" :disabled="!!busy" @click="performAction(selectedTask,'cancel')"><CircleStop/>取消任务</button>
          <button v-if="selectedTask.status==='running'&&selectedTask.operation==='shell.exec'&&!selectedTask.cancel_requested" class="stop" :disabled="!!busy" @click="requestTaskStop(selectedTask)"><CircleStop/>停止运行</button>
          <button v-if="selectedTask.status==='succeeded'&&selectedTask.rollback_available&&selectedTask.operation!=='shell.exec'" :disabled="!!busy" @click="performAction(selectedTask,'rollback')"><RotateCcw/>创建回滚任务</button>
          <button v-if="selectedTask.operation!=='task.rollback'&&['failed','cancelled'].includes(selectedTask.status)&&selectedTask.can_resume" class="resume" :disabled="!!busy" @click="retryTask(selectedTask,'resume')"><Play/>从检查点恢复</button>
          <button v-if="selectedTask.operation!=='task.rollback'&&['failed','cancelled','succeeded'].includes(selectedTask.status)" class="restart" :disabled="!!busy" @click="retryTask(selectedTask,'restart')"><RefreshCw/>从头重新执行</button>
          <span v-if="isTaskStopping(selectedTask)" class="task-stopping-note"><RefreshCw class="s-spin"/>停止请求已发送 · {{formatDate(selectedTask.cancel_requested_at)}}</span>
          <span v-else-if="selectedTask.status==='running'&&selectedTask.operation!=='shell.exec'">任务正在 Agent 上执行，当前不能从云端强制终止。</span>
          <span v-else-if="selectedTask.operation==='shell.exec'&&selectedTask.status==='succeeded'">Shell 任务不提供回滚。</span>
        </div>

        <dl class="task-facts">
          <div><dt>所需权限</dt><dd>{{permissionLabel(selectedTask.required_permission)}}</dd></div>
          <div><dt>执行尝试</dt><dd>第 {{selectedTask.attempt_no}} 次</dd></div>
          <div><dt>执行方式</dt><dd>{{executionModeLabel(selectedTask.execution_mode)}}</dd></div>
          <div><dt>任务谱系</dt><dd class="task-mono">{{selectedTask.lineage_id}}</dd></div>
          <div><dt>创建时间</dt><dd>{{formatDate(selectedTask.created_at)}}</dd></div>
          <div><dt>开始时间</dt><dd>{{formatDate(selectedTask.started_at)}}</dd></div>
          <div><dt>完成时间</dt><dd>{{formatDate(selectedTask.completed_at)}}</dd></div>
          <div v-if="selectedTask.cancel_requested_at"><dt>停止请求</dt><dd>{{formatDate(selectedTask.cancel_requested_at)}}</dd></div>
          <div v-if="selectedTask.cancel_acknowledged_at"><dt>Agent 已确认</dt><dd>{{formatDate(selectedTask.cancel_acknowledged_at)}}</dd></div>
          <div v-if="selectedTask?.team_id"><dt>审批团队</dt><dd>{{teams.find(team => team.id === selectedTask?.team_id)?.name || selectedTask?.team_id}}</dd></div>
          <div v-if="selectedTask.approval_id"><dt>审批记录</dt><dd class="task-mono">{{selectedTask.approval_id}}</dd></div>
        </dl>

        <section class="task-checkpoint-summary">
          <div v-if="selectedTask.latest_checkpoint">
            <Clock3/><p><b>最近检查点 · #{{selectedTask.latest_checkpoint.seq}}</b><small>{{checkpointKindLabel(selectedTask.latest_checkpoint.kind)}} · {{formatDate(selectedTask.latest_checkpoint.created_at)}} · {{selectedTask.latest_checkpoint.resumable?'可用于恢复':'仅供记录'}}</small></p>
          </div>
          <div v-else><Clock3/><p><b>尚无检查点</b><small>任务执行到可恢复阶段后会在这里记录。</small></p></div>
          <div v-if="taskSource(selectedTask)"><RotateCcw/><p><b>{{taskSource(selectedTask)!.label}}</b><small class="task-mono">{{taskSource(selectedTask)!.id}}</small></p></div>
          <div v-if="selectedTask.resume_checkpoint_id"><Play/><p><b>本次恢复检查点</b><small class="task-mono">{{selectedTask.resume_checkpoint_id}}</small></p></div>
        </section>

        <details class="task-json" open><summary>任务输入</summary><pre>{{formatJson(selectedTask.input)}}</pre></details>
        <section v-if="selectedTask.error" class="task-result error"><header><TriangleAlert/>错误</header><pre>{{selectedTask.error}}</pre></section>
        <section v-if="selectedTask.output!==null&&selectedTask.output!==undefined" class="task-result"><header><FileText/>输出</header><pre>{{formatJson(selectedTask.output)}}</pre></section>

        <section v-if="selectedTask.artifacts?.length" class="task-artifacts">
          <header><Archive/>产物</header>
          <div v-for="artifact in selectedTask.artifacts" :key="`${artifact.path}:${artifact.name}`"><Folder v-if="artifact.kind==='directory'"/><FileText v-else/><p><b>{{artifact.name}}</b><small>{{artifact.path}}<template v-if="artifact.size_bytes!==undefined"> · {{formatBytes(artifact.size_bytes)}}</template></small></p><em>{{artifact.kind}}</em></div>
        </section>

        <section class="task-events">
          <header><Activity/>实时事件 <span>{{selectedTask.events.length}}</span></header>
          <div v-if="selectedTask.events.length" class="task-event-list">
            <article v-for="event in selectedTask.events" :key="event.seq" :class="event.level"><i/><time>#{{event.seq}} · {{formatDate(event.created_at)}}</time><p>{{eventMessage(event.message)}}</p><pre v-if="event.data!==null&&event.data!==undefined">{{formatJson(event.data)}}</pre></article>
          </div>
          <div v-else class="cloud-empty"><Clock3/>等待 Agent 事件</div>
        </section>
      </article>
      <article v-else class="cloud-panel task-detail-empty"><SquareTerminal/><h3>选择一条任务查看详情</h3><p>这里会显示状态、事件、输出和产物元数据。</p></article>
    </div>
  </section>
</template>
