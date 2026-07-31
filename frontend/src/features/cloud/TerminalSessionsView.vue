<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import {
  CheckCircle2, CircleStop, Clock3, MonitorUp, Plus, RefreshCw, ShieldAlert,
  SquareTerminal, TriangleAlert, Wifi, WifiOff,
} from 'lucide-vue-next'
import { CloudApiError, cloudRequest } from './client'
import type { CloudAgent, CloudTerminalEvent, CloudTerminalSession } from './types'
import './terminal-sessions.css'

interface TerminalEventsResponse {
  session: CloudTerminalSession
  events: CloudTerminalEvent[]
}

const sessions = ref<CloudTerminalSession[]>([])
const agents = ref<CloudAgent[]>([])
const selectedSessionId = ref('')
const terminalHost = ref<HTMLElement | null>(null)
const createForm = ref({ agent_id: '', title: '', cwd: '' })
const busy = ref('')
const error = ref('')
const refreshWarning = ref('')
const approvalConfirmed = ref(false)
const lastSeq = ref(0)
const replaying = ref(false)

let terminal: Terminal | null = null
let fitAddon: FitAddon | null = null
let resizeObserver: ResizeObserver | null = null
let listTimer = 0
let eventTimer = 0
let resizeTimer = 0
let inputTimer = 0
let inputBuffer = ''
let inputSending = false
let pendingInput: { idempotencyKey: string; data: string } | null = null
let eventInFlight = false
let sessionListInFlight = false
let disposed = false
let replayGeneration = 0
const terminalDisposables: Array<{ dispose: () => void }> = []

const terminalAgents = computed(() => agents.value.filter(agent =>
  agent.status === 'active' && agent.online
  && agent.permissions.includes('full')
  && agent.capabilities.includes('shell-v1')
  && agent.capabilities.includes('terminal-v1'),
))
const selectedSession = computed(() => sessions.value.find(item => item.id === selectedSessionId.value) || null)
const canType = computed(() => selectedSession.value?.status === 'running')
const canApprove = computed(() => isAwaitingApproval(selectedSession.value?.status))
const canTerminate = computed(() => !!selectedSession.value && !isTerminalStatus(selectedSession.value.status))

watch(terminalAgents, items => {
  if (!items.some(agent => agent.id === createForm.value.agent_id)) createForm.value.agent_id = items[0]?.id || ''
}, { immediate: true })

watch(selectedSessionId, () => {
  approvalConfirmed.value = false
  beginReplay()
})

function isAwaitingApproval(status?: string) {
  return status === 'awaiting_approval' || status === 'pending_approval'
}

function isTerminalStatus(status: string) {
  return ['exited', 'failed', 'terminated', 'cancelled'].includes(status)
}

function statusLabel(status: string) {
  const labels: Record<string, string> = {
    awaiting_approval: '等待批准', pending_approval: '等待批准', approved: '等待 Agent',
    pending: '等待 Agent', queued: '等待 Agent', starting: '正在启动', running: '已连接',
    terminating: '正在终止', exited: '已退出',
    failed: '启动失败', terminated: '已终止', cancelled: '已取消',
  }
  return labels[status] || status
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

function readableError(value: unknown) {
  if (!(value instanceof CloudApiError)) return value instanceof Error ? value.message : String(value)
  const known: Record<string, string> = {
    agent_not_found: '找不到目标 Agent，请刷新后重试。',
    agent_not_active: '目标 Agent 尚未确认或已被撤销。',
    agent_offline: '目标 Agent 当前离线。',
    agent_capability_missing: '目标 Agent 不支持持久终端。',
    agent_permission_missing: '目标 Agent 没有完整执行权限。',
    terminal_not_awaiting_approval: '会话状态已变化，无法再次批准。',
    terminal_state_conflict: '终端尚未运行或已经结束。',
  }
  return known[value.code] || value.message || `请求失败（HTTP ${value.status}）`
}

function normalizeSessions(value: CloudTerminalSession[] | { sessions: CloudTerminalSession[] }) {
  return Array.isArray(value) ? value : value.sessions
}

function upsertSession(session: CloudTerminalSession) {
  const index = sessions.value.findIndex(item => item.id === session.id)
  if (index >= 0) sessions.value[index] = session
  else sessions.value.unshift(session)
}

async function loadSessions(quiet = false) {
  if (sessionListInFlight) return
  sessionListInFlight = true
  if (!quiet) busy.value = 'refresh'
  try {
    const [sessionResponse, nextAgents] = await Promise.all([
      cloudRequest<CloudTerminalSession[] | { sessions: CloudTerminalSession[] }>('/api/cloud/terminal-sessions'),
      cloudRequest<CloudAgent[]>('/api/cloud/agents'),
    ])
    const nextSessions = normalizeSessions(sessionResponse)
    sessions.value = nextSessions
    agents.value = nextAgents
    if (!selectedSessionId.value || !nextSessions.some(item => item.id === selectedSessionId.value)) {
      selectedSessionId.value = nextSessions[0]?.id || ''
    }
    error.value = ''
    refreshWarning.value = ''
  } catch (value) {
    const message = readableError(value)
    if (quiet) refreshWarning.value = `会话列表刷新失败：${message}`
    else error.value = message
  } finally {
    sessionListInFlight = false
    if (!quiet) busy.value = ''
  }
}

async function createSession() {
  if (!createForm.value.agent_id || busy.value) return
  busy.value = 'create'
  error.value = ''
  try {
    const session = await cloudRequest<CloudTerminalSession>('/api/cloud/terminal-sessions', {
      method: 'POST',
      body: JSON.stringify({
        agent_id: createForm.value.agent_id,
        ...(createForm.value.title.trim() ? { title: createForm.value.title.trim() } : {}),
        ...(createForm.value.cwd.trim() ? { cwd: createForm.value.cwd.trim() } : {}),
        cols: clampCols(terminal?.cols || 100),
        rows: clampRows(terminal?.rows || 30),
      }),
    })
    upsertSession(session)
    selectedSessionId.value = session.id
    approvalConfirmed.value = false
    createForm.value.title = ''
  } catch (value) {
    error.value = readableError(value)
  } finally {
    busy.value = ''
  }
}

async function approveSession() {
  const session = selectedSession.value
  if (!session || !approvalConfirmed.value || busy.value) return
  busy.value = `approve:${session.id}`
  error.value = ''
  try {
    const updated = await cloudRequest<CloudTerminalSession | void>(`/api/cloud/terminal-sessions/${session.id}/approve`, { method: 'POST' })
    if (updated) upsertSession(updated)
    else await loadSessions(true)
    approvalConfirmed.value = false
  } catch (value) {
    error.value = readableError(value)
    await loadSessions(true)
  } finally {
    busy.value = ''
  }
}

async function terminateSession() {
  const session = selectedSession.value
  if (!session || busy.value) return
  busy.value = `terminate:${session.id}`
  error.value = ''
  try {
    const updated = await cloudRequest<CloudTerminalSession | void>(`/api/cloud/terminal-sessions/${session.id}/terminate`, { method: 'POST' })
    if (updated) upsertSession(updated)
    else await loadSessions(true)
  } catch (value) {
    error.value = readableError(value)
    await loadSessions(true)
  } finally {
    busy.value = ''
  }
}

function bytesToBase64(text: string) {
  const bytes = new TextEncoder().encode(text)
  let binary = ''
  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
  }
  return btoa(binary)
}

function takeInputChunk(value: string, maximumBytes = 8192) {
  let bytes = 0
  let end = 0
  for (const character of value) {
    const size = new TextEncoder().encode(character).length
    if (bytes + size > maximumBytes) break
    bytes += size
    end += character.length
  }
  return { data: value.slice(0, end), remaining: value.slice(end) }
}

function clampCols(value: number) {
  return Math.min(400, Math.max(20, value))
}

function clampRows(value: number) {
  return Math.min(200, Math.max(5, value))
}

function base64ToBytes(value: string) {
  const binary = atob(value)
  return Uint8Array.from(binary, character => character.charCodeAt(0))
}

function queueInput(data: string) {
  if (!canType.value) return
  inputBuffer += data
  window.clearTimeout(inputTimer)
  inputTimer = window.setTimeout(() => void flushInput(), 35)
}

async function flushInput() {
  if (inputSending || (!inputBuffer && !pendingInput) || !canType.value || !selectedSession.value) return
  const sessionId = selectedSession.value.id
  if (!pendingInput) {
    const chunk = takeInputChunk(inputBuffer)
    if (!chunk.data) return
    inputBuffer = chunk.remaining
    pendingInput = { idempotencyKey: `terminal-input:${crypto.randomUUID()}`, data: chunk.data }
  }
  const request = pendingInput
  inputSending = true
  try {
    await cloudRequest<void>(`/api/cloud/terminal-sessions/${sessionId}/input`, {
      method: 'POST',
      body: JSON.stringify({
        data_base64: bytesToBase64(request.data), idempotency_key: request.idempotencyKey,
      }),
    })
    pendingInput = null
  } catch (value) {
    error.value = readableError(value)
  } finally {
    inputSending = false
    if (inputBuffer || pendingInput) window.setTimeout(() => void flushInput(), pendingInput ? 800 : 0)
  }
}

function queueResize(cols: number, rows: number) {
  window.clearTimeout(resizeTimer)
  resizeTimer = window.setTimeout(() => void sendResize(cols, rows), 180)
}

async function sendResize(cols: number, rows: number) {
  const session = selectedSession.value
  if (!session || session.status !== 'running') return
  try {
    await cloudRequest<void>(`/api/cloud/terminal-sessions/${session.id}/resize`, {
      method: 'POST', body: JSON.stringify({ cols: clampCols(cols), rows: clampRows(rows) }),
    })
  } catch (value) {
    refreshWarning.value = `终端尺寸同步失败：${readableError(value)}`
  }
}

function writeEvent(event: CloudTerminalEvent) {
  if (!terminal) return
  if (event.data_base64) {
    try { terminal.write(base64ToBytes(event.data_base64)) } catch { terminal.writeln('\r\n[无法解码终端输出]') }
    return
  }
  if (typeof event.data === 'string' && ['output', 'stdout', 'stderr', 'data'].includes(event.kind)) {
    terminal.write(event.data)
  }
}

function beginReplay() {
  replayGeneration += 1
  lastSeq.value = 0
  replaying.value = !!selectedSessionId.value
  inputBuffer = ''
  pendingInput = null
  terminal?.reset()
  if (selectedSessionId.value) void pollEvents(replayGeneration)
}

async function pollEvents(generation: number) {
  if (disposed || eventInFlight || generation !== replayGeneration || !selectedSessionId.value) return
  eventInFlight = true
  const sessionId = selectedSessionId.value
  try {
    const response = await cloudRequest<TerminalEventsResponse>(
      `/api/cloud/terminal-sessions/${sessionId}/events?after_seq=${lastSeq.value}&limit=500`,
    )
    if (generation !== replayGeneration || sessionId !== selectedSessionId.value) return
    upsertSession(response.session)
    for (const event of response.events) {
      if (event.seq <= lastSeq.value) continue
      writeEvent(event)
      lastSeq.value = event.seq
    }
    replaying.value = response.events.length >= 500
    refreshWarning.value = ''
  } catch (value) {
    if (generation === replayGeneration) refreshWarning.value = `终端连接暂时中断：${readableError(value)}；正在自动恢复。`
  } finally {
    eventInFlight = false
    if (!disposed) {
      const nextGeneration = replayGeneration
      eventTimer = window.setTimeout(
        () => void pollEvents(nextGeneration),
        generation === nextGeneration && !replaying.value ? 300 : 0,
      )
    }
  }
}

function initializeTerminal() {
  if (!terminalHost.value) return
  terminal = new Terminal({
    cursorBlink: true, convertEol: false, scrollback: 10000,
    fontFamily: "'Cascadia Code', Consolas, monospace", fontSize: 12,
    theme: { background: '#090d11', foreground: '#d5e0e5', cursor: '#67b6cb', selectionBackground: '#315361' },
  })
  fitAddon = new FitAddon()
  terminal.loadAddon(fitAddon)
  terminal.open(terminalHost.value)
  fitAddon.fit()
  terminalDisposables.push(terminal.onData(queueInput), terminal.onResize(size => queueResize(size.cols, size.rows)))
  resizeObserver = new ResizeObserver(() => {
    try { fitAddon?.fit() } catch {}
  })
  resizeObserver.observe(terminalHost.value)
}

onMounted(async () => {
  await nextTick()
  initializeTerminal()
  await loadSessions()
  listTimer = window.setInterval(() => void loadSessions(true), 3000)
})

onUnmounted(() => {
  disposed = true
  replayGeneration += 1
  window.clearInterval(listTimer)
  window.clearTimeout(eventTimer)
  window.clearTimeout(resizeTimer)
  window.clearTimeout(inputTimer)
  resizeObserver?.disconnect()
  for (const disposable of terminalDisposables) disposable.dispose()
  terminal?.dispose()
})
</script>

<template>
  <section class="terminal-sessions">
    <article class="cloud-panel terminal-hero">
      <span><MonitorUp/></span>
      <div><small>PERSISTENT TERMINAL</small><h3>持久终端</h3><p>从云端连接已配对主机；网络中断或刷新页面后，可重新进入会话并恢复输出。</p></div>
      <button class="cloud-icon-btn" title="刷新会话" :disabled="busy==='refresh'" @click="loadSessions()"><RefreshCw :class="{'s-spin':busy==='refresh'}"/></button>
    </article>

    <div v-if="error" class="terminal-notice error"><TriangleAlert/>{{error}}</div>
    <div v-if="refreshWarning" class="terminal-notice warning"><WifiOff/>{{refreshWarning}}</div>

    <article class="cloud-panel terminal-create">
      <header><div><h3>新建终端会话</h3><p>会话创建后必须由你手动批准，Agent 才会启动系统 Shell。</p></div><Plus/></header>
      <form @submit.prevent="createSession">
        <label>目标 Agent<select v-model="createForm.agent_id" required><option value="" disabled>{{terminalAgents.length?'选择在线 Agent':'暂无支持终端的在线 Agent'}}</option><option v-for="agent in terminalAgents" :key="agent.id" :value="agent.id">{{agent.name}} · {{agent.workspace_label}}</option></select></label>
        <label>会话名称（可选）<input v-model="createForm.title" maxlength="80" placeholder="例如：服务端维护"/></label>
        <label>初始目录（可选）<input v-model="createForm.cwd" maxlength="1024" placeholder="留空使用 Agent 工作目录"/></label>
        <button class="cloud-primary" :disabled="!createForm.agent_id||!!busy"><RefreshCw v-if="busy==='create'" class="s-spin"/><Plus v-else/>创建并等待批准</button>
      </form>
      <div class="terminal-risk"><ShieldAlert/><p><b>终端拥有完整的系统账户权限</b><small>终端可访问 Agent 进程所属账户能够访问的文件、程序和网络资源。Agent 进程退出时，终端会话也会结束。</small></p></div>
    </article>

    <div class="terminal-layout">
      <article class="cloud-panel terminal-list">
        <header><div><h3>可恢复会话</h3><p>{{sessions.length}} 个会话</p></div><Clock3/></header>
        <button v-for="session in sessions" :key="session.id" :class="{active:selectedSessionId===session.id}" @click="selectedSessionId=session.id">
          <span :class="session.status"><Wifi v-if="session.status==='running'"/><SquareTerminal v-else/></span>
          <p><b>{{session.title||'远程终端'}}</b><small>{{agentName(session.agent_id)}} · {{formatDate(session.updated_at)}}</small></p>
          <em>{{statusLabel(session.status)}}</em>
        </button>
        <div v-if="!sessions.length" class="terminal-empty"><SquareTerminal/>尚无终端会话</div>
      </article>

      <article class="cloud-panel terminal-console">
        <header v-if="selectedSession">
          <div><h3>{{selectedSession.title||'远程终端'}}</h3><p>{{agentName(selectedSession.agent_id)}} · {{selectedSession.cwd||'Agent 工作目录'}} · {{selectedSession.cols}}×{{selectedSession.rows}}</p></div>
          <span :class="selectedSession.status">{{statusLabel(selectedSession.status)}}</span>
        </header>
        <header v-else><div><h3>远程终端</h3><p>选择已有会话或新建会话</p></div></header>

        <div v-if="selectedSession && canApprove" class="terminal-approval">
          <ShieldAlert/><p><b>批准后将启动完整权限 Shell</b><small>命令会以 Agent 进程所属系统账户执行。请仅在你信任该主机和当前工作环境时批准。</small></p>
          <label><input v-model="approvalConfirmed" type="checkbox"/>我已确认权限范围</label>
          <button class="cloud-primary" :disabled="!approvalConfirmed||!!busy" @click="approveSession"><CheckCircle2/>批准并连接</button>
        </div>
        <div class="terminal-toolbar" v-if="selectedSession">
          <span v-if="replaying"><RefreshCw class="s-spin"/>正在恢复历史输出</span>
          <span v-else-if="canType"><Wifi/>连接正常，可直接输入</span>
          <span v-else><Clock3/>{{isTerminalStatus(selectedSession.status)?'会话已经结束':'等待 Agent 建立终端'}}</span>
          <button v-if="canTerminate" :disabled="!!busy" @click="terminateSession"><CircleStop/>终止会话</button>
        </div>
        <div ref="terminalHost" class="terminal-host" :class="{disabled:!canType}"/>
        <footer v-if="selectedSession">
          <span>最后活动：{{formatDate(selectedSession.last_seen_at)}}</span>
          <span v-if="selectedSession.exit_code!==null&&selectedSession.exit_code!==undefined">退出码：{{selectedSession.exit_code}}</span>
          <span v-if="selectedSession.error" class="error">{{selectedSession.error}}</span>
        </footer>
      </article>
    </div>
  </section>
</template>
