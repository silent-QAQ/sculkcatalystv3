<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  Check, CheckCircle2, Clipboard, Clock3, Copy, Cpu, Download, Fingerprint, Laptop,
  RadioTower, RefreshCw, ShieldCheck, Trash2, TriangleAlert,
} from 'lucide-vue-next'
import { agentChecksumsPath, agentDownloadPath, createAgentBootstrap, cloudRequest } from './client'
import type { AgentBootstrapDownload } from './client'
import type { AgentPairingCreated, CloudAgent } from './types'
import './agent-devices.css'

const agents = ref<CloudAgent[]>([])
const pairing = ref<AgentPairingCreated | null>(null)
const bootstrap = ref<AgentBootstrapDownload | null>(null)
const busy = ref('')
const error = ref('')
const copied = ref('')
const revokeCandidate = ref('')
const now = ref(Date.now())
const agentName = ref('mc-host')
const workspaceLabel = ref('minecraft')
const windowsWorkspaceRoot = ref('C:\\Minecraft')
const linuxWorkspaceRoot = ref('/srv/minecraft')
let refreshTimer = 0
let clockTimer = 0

const cloudOrigin = computed(() => {
  const configured = String(import.meta.env.VITE_CLOUD_PUBLIC_URL || '').trim()
  if (!configured) return window.location.origin
  try {
    const parsed = new URL(configured)
    if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password) return window.location.origin
    return parsed.origin
  } catch {
    return window.location.origin
  }
})

const pairingExpired = computed(() => pairing.value ? new Date(pairing.value.expires_at).getTime() <= now.value : false)
const bootstrapExpired = computed(() => {
  if (!bootstrap.value?.expiresAt) return false
  const expiry = new Date(bootstrap.value.expiresAt).getTime()
  return !Number.isFinite(expiry) || expiry <= now.value
})
const linuxBootstrapCommand = computed(() => {
  const filename = bootstrap.value?.configFilename || 'sculk-agent.json'
  // Keep the published static filename so the sidecar JSON is discovered by
  // the portable Agent without requiring a manual rename.
  const executable = 'sculk-agent-linux-x86_64'
  return `chmod +x ./${executable} && ./${executable} run --config "./${filename}"`
})
function pairCommand(executable: string, workspaceRoot: string) {
  return pairing.value
    ? `${executable} pair --cloud "${cloudOrigin.value}" --code "${pairing.value.pairing_code}" --name "mc-host" --workspace "minecraft" --workspace-root "${workspaceRoot}" --permissions "full" --capabilities "heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1"`
    : ''
}
const windowsCommand = computed(() => pairCommand('.\\sculk-agent.exe', windowsWorkspaceRoot.value.trim()))
const linuxCommand = computed(() => pairCommand('./sculk-agent', linuxWorkspaceRoot.value.trim()))
const activeAgents = computed(() => agents.value.filter(agent => agent.status !== 'revoked'))
const revokedAgents = computed(() => agents.value.filter(agent => agent.status === 'revoked'))

function errorText(value: unknown) {
  return value instanceof Error ? value.message : String(value)
}

function formatDate(value?: string | null) {
  if (!value) return '从未连接'
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit',
  }).format(new Date(value))
}

function statusLabel(agent: CloudAgent) {
  if (agent.status === 'revoked') return '已撤销'
  if (agent.status === 'claimed') return '等待确认'
  return agent.online ? '在线' : '离线'
}

function permissionLabel(permission: string) {
  return ({ read: '读取', write: '写入', process: '进程控制', full: '完整执行' } as Record<string, string>)[permission] || permission
}

async function loadAgents(quiet = false) {
  if (!quiet) busy.value = 'refresh'
  try {
    agents.value = await cloudRequest<CloudAgent[]>('/api/cloud/agents')
    error.value = ''
  } catch (value) {
    if (!quiet) error.value = errorText(value)
  } finally {
    if (!quiet) busy.value = ''
  }
}

async function createPairing() {
  busy.value = 'pairing'
  error.value = ''
  try {
    pairing.value = await cloudRequest<AgentPairingCreated>('/api/cloud/agent-pairings', { method: 'POST' })
    revokeCandidate.value = ''
  } catch (value) {
    error.value = errorText(value)
  } finally {
    busy.value = ''
  }
}

function triggerRemoteDownload(downloadUrl: string) {
  const link = document.createElement('a')
  link.href = downloadUrl
  link.download = ''
  link.rel = 'noopener noreferrer'
  document.body.append(link)
  link.click()
  link.remove()
}

function safeConfigFilename(filename: string | null) {
  const candidate = filename?.trim() || 'sculk-agent.json'
  return /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/i.test(candidate) ? candidate : 'sculk-agent.json'
}

function downloadBootstrapConfig() {
  if (!bootstrap.value?.configJson || bootstrapExpired.value) return
  const json = typeof bootstrap.value.configJson === 'string'
    ? bootstrap.value.configJson
    : JSON.stringify(bootstrap.value.configJson, null, 2) || '{}'
  const objectUrl = URL.createObjectURL(new Blob([json], { type: 'application/json' }))
  const link = document.createElement('a')
  link.href = objectUrl
  link.download = safeConfigFilename(bootstrap.value.configFilename)
  document.body.append(link)
  link.click()
  link.remove()
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0)
}

async function createBootstrap(platform: 'windows' | 'linux') {
  const workspaceRoot = (platform === 'windows' ? windowsWorkspaceRoot.value : linuxWorkspaceRoot.value).trim()
  if (!agentName.value.trim() || !workspaceLabel.value.trim() || !workspaceRoot) {
    error.value = '请填写主机名称、工作区名称和对应平台的工作区根目录'
    return
  }
  busy.value = `bootstrap:${platform}`
  error.value = ''
  try {
    const generated = await createAgentBootstrap({
      platform,
      name: agentName.value.trim(),
      workspaceLabel: workspaceLabel.value.trim(),
      workspaceRoot,
    })
    bootstrap.value = generated
    triggerRemoteDownload(generated.downloadUrl)
    if (generated.platform === 'windows' && generated.configJson) downloadBootstrapConfig()
  } catch (value) {
    error.value = errorText(value)
  } finally {
    busy.value = ''
  }
}

async function confirmAgent(agent: CloudAgent) {
  busy.value = `confirm:${agent.id}`
  error.value = ''
  try {
    const confirmed = await cloudRequest<CloudAgent>(`/api/cloud/agents/${agent.id}/confirm`, { method: 'POST' })
    agents.value = agents.value.map(item => item.id === confirmed.id ? confirmed : item)
    pairing.value = null
  } catch (value) {
    error.value = errorText(value)
  } finally {
    busy.value = ''
  }
}

async function revokeAgent(agent: CloudAgent) {
  if (revokeCandidate.value !== agent.id) {
    revokeCandidate.value = agent.id
    return
  }
  busy.value = `revoke:${agent.id}`
  error.value = ''
  try {
    await cloudRequest<void>(`/api/cloud/agents/${agent.id}`, { method: 'DELETE' })
    revokeCandidate.value = ''
    await loadAgents(true)
  } catch (value) {
    error.value = errorText(value)
  } finally {
    busy.value = ''
  }
}

async function copyValue(key: string, value: string) {
  try {
    await navigator.clipboard.writeText(value)
    copied.value = key
    window.setTimeout(() => { if (copied.value === key) copied.value = '' }, 1800)
  } catch {
    error.value = '无法写入剪贴板，请手动复制命令'
  }
}

onMounted(() => {
  void loadAgents()
  refreshTimer = window.setInterval(() => void loadAgents(true), 5000)
  clockTimer = window.setInterval(() => { now.value = Date.now() }, 1000)
})
onUnmounted(() => {
  window.clearInterval(refreshTimer)
  window.clearInterval(clockTimer)
})
</script>

<template>
  <section class="agent-devices">
    <article class="cloud-panel agent-hero">
      <div class="agent-hero-mark"><RadioTower/></div>
      <div>
        <span>SCULK AGENT</span>
        <h3>把 Minecraft 主机接入云控制台</h3>
        <p>Agent 只主动连接 Sculk Cloud，不监听入站端口；主机无需公网 IP、端口映射或直接开放控制台。</p>
      </div>
      <button class="cloud-primary compact" :disabled="!!busy" @click="createPairing">
        <RefreshCw v-if="busy==='pairing'" class="s-spin"/><Laptop v-else/>
        {{ pairing && !pairingExpired ? '生成新的本地配对码' : '使用本地模式' }}
      </button>
    </article>

    <div v-if="error" class="agent-error"><TriangleAlert/>{{ error }}</div>

    <article class="cloud-panel agent-bootstrap-panel">
      <header>
        <div><h3>下载并启动 Agent</h3><p>为当前已登录账号生成短期启动资料。启动资料过期后需重新生成，页面不会保存配置或账号凭据。</p></div>
        <Download/>
      </header>
      <div class="agent-bootstrap-inputs">
        <label>主机名称<input v-model="agentName" maxlength="64" placeholder="mc-host"/></label>
        <label>工作区名称<input v-model="workspaceLabel" maxlength="128" placeholder="minecraft"/></label>
        <label>Windows 工作区根目录<input v-model="windowsWorkspaceRoot" placeholder="C:\Minecraft"/></label>
        <label>Linux 工作区根目录<input v-model="linuxWorkspaceRoot" placeholder="/srv/minecraft"/></label>
      </div>
      <div class="agent-bootstrap-options">
        <section class="agent-bootstrap-option" :class="{ selected: bootstrap?.platform==='windows', expired: bootstrap?.platform==='windows' && bootstrapExpired }">
          <div><Laptop/><p><b>Windows</b><small>下载一键启动包，解压后运行其中的启动文件。</small></p></div>
          <button class="cloud-primary compact" :disabled="!!busy" @click="createBootstrap('windows')"><RefreshCw v-if="busy==='bootstrap:windows'" class="s-spin"/><Download v-else/>{{ bootstrap?.platform==='windows' ? '重新生成并下载' : '生成并下载启动包' }}</button>
          <template v-if="bootstrap?.platform==='windows'">
            <div v-if="!bootstrapExpired" class="agent-bootstrap-actions"><button @click="triggerRemoteDownload(bootstrap.downloadUrl)"><Download/>再次下载 Agent</button><button v-if="bootstrap.configJson" @click="downloadBootstrapConfig"><Download/>下载 {{ safeConfigFilename(bootstrap.configFilename) }}</button></div>
            <p class="agent-bootstrap-note">{{ bootstrapExpired ? '此启动包链接已过期，请重新生成。' : bootstrap.configJson ? `已生成，请将配置文件与 Agent 放在同一目录后运行；有效期至 ${bootstrap.expiresAt ? formatDate(bootstrap.expiresAt) : '服务端失效'}。` : `已生成，下载链接有效至 ${bootstrap.expiresAt ? formatDate(bootstrap.expiresAt) : '服务端失效'}。` }}</p>
          </template>
        </section>
        <section class="agent-bootstrap-option" :class="{ selected: bootstrap?.platform==='linux', expired: bootstrap?.platform==='linux' && bootstrapExpired }">
          <div><Cpu/><p><b>Linux</b><small>下载 Agent 与一次性配置文件，然后在主机终端启动。</small></p></div>
          <button class="cloud-primary compact" :disabled="!!busy" @click="createBootstrap('linux')"><RefreshCw v-if="busy==='bootstrap:linux'" class="s-spin"/><Download v-else/>{{ bootstrap?.platform==='linux' ? '重新生成并下载' : '生成 Linux Agent' }}</button>
          <template v-if="bootstrap?.platform==='linux'">
            <div v-if="!bootstrapExpired" class="agent-bootstrap-actions">
              <button @click="triggerRemoteDownload(bootstrap.downloadUrl)"><Download/>下载 Agent</button>
              <button :disabled="!bootstrap.configJson" @click="downloadBootstrapConfig"><Download/>下载 {{ safeConfigFilename(bootstrap.configFilename) }}</button>
            </div>
            <div v-if="!bootstrapExpired" class="agent-command"><span>Linux</span><code>{{ linuxBootstrapCommand }}</code><button title="复制 Linux 启动命令" @click="copyValue('linux-bootstrap', linuxBootstrapCommand)"><Check v-if="copied==='linux-bootstrap'"/><Clipboard v-else/></button></div>
            <p class="agent-bootstrap-note">{{ bootstrapExpired ? '此配置已过期，请重新生成后重新下载。' : `请将配置文件与 sculk-agent 放在同一目录；启动后仍需在云端确认主机指纹。${bootstrap.configJson ? '' : ' 服务端未返回配置文件，请重新生成。'}` }}</p>
          </template>
        </section>
      </div>
    </article>

    <article v-if="pairing" class="cloud-panel pairing-panel" :class="{ expired: pairingExpired }">
      <header>
        <div><h3>{{ pairingExpired ? '本地配对码已过期' : '本地模式：在 Minecraft 主机上完成配对' }}</h3><p>{{ pairingExpired ? '请重新生成配对码' : `有效期至 ${formatDate(pairing.expires_at)}` }}</p></div>
        <Clock3/>
      </header>
      <div class="pairing-code-row">
        <code>{{ pairing.pairing_code }}</code>
        <button class="cloud-icon-btn" title="复制配对码" :disabled="pairingExpired" @click="copyValue('code', pairing.pairing_code)"><Check v-if="copied==='code'"/><Copy v-else/></button>
      </div>
      <div v-if="!pairingExpired" class="pairing-steps">
        <p><b>1</b> 将对应平台的 <code>sculk-agent</code> 可执行文件放到 Minecraft 主机。</p>
        <div class="agent-downloads">
          <a :href="agentDownloadPath('windows')" download><Download/>Windows x86_64</a>
          <a :href="agentDownloadPath('linux')" download><Download/>Linux x86_64（静态链接）</a>
          <a :href="agentChecksumsPath()" target="_blank" rel="noopener noreferrer">SHA-256 校验值</a>
        </div>
        <p><b>2</b> 填写 Agent 可以操作的工作区根目录。目录必须已经存在；Shell 默认在这里启动，但命令仍继承 Agent 进程所属系统账户的权限。</p>
        <div class="agent-root-inputs"><label>Windows 工作区根目录<input v-model="windowsWorkspaceRoot" placeholder="C:\Minecraft"/></label><label>Linux 工作区根目录<input v-model="linuxWorkspaceRoot" placeholder="/srv/minecraft"/></label></div>
        <p><b>3</b> 运行对应平台的配对命令。默认申请完整执行权限并启用远程任务、Shell 与持久终端；配对后仍需在云端核对指纹并确认。</p>
        <div class="agent-command"><span>Windows PowerShell</span><code>{{ windowsCommand }}</code><button title="复制 Windows 命令" :disabled="!windowsWorkspaceRoot.trim()" @click="copyValue('windows', windowsCommand)"><Check v-if="copied==='windows'"/><Clipboard v-else/></button></div>
        <div class="agent-command"><span>Linux</span><code>{{ linuxCommand }}</code><button title="复制 Linux 命令" :disabled="!linuxWorkspaceRoot.trim()" @click="copyValue('linux', linuxCommand)"><Check v-if="copied==='linux'"/><Clipboard v-else/></button></div>
        <p><b>4</b> 对照主机终端显示的指纹、权限和能力，再在云端确认。完整执行权限允许 Shell 以 Agent 系统账户权限运行。</p>
      </div>
    </article>

    <article class="cloud-panel agent-list-panel">
      <header>
        <div><h3>已连接主机</h3><p>{{ activeAgents.length }} 台可用 · {{ activeAgents.filter(item => item.online).length }} 台在线</p></div>
        <button class="cloud-icon-btn" title="刷新主机状态" :disabled="busy==='refresh'" @click="loadAgents()"><RefreshCw :class="{'s-spin':busy==='refresh'}"/></button>
      </header>
      <div v-if="activeAgents.length" class="agent-list">
        <section v-for="agent in activeAgents" :key="agent.id" class="agent-card" :class="[agent.status, { online: agent.online }]">
          <div class="agent-platform"><Cpu/><i/></div>
          <div class="agent-main">
            <div class="agent-title"><h4>{{ agent.name }}</h4><span>{{ statusLabel(agent) }}</span></div>
            <p>{{ agent.platform }} · Agent {{ agent.version }} · {{ agent.workspace_label }}</p>
            <div class="agent-fingerprint"><Fingerprint/><code>{{ agent.fingerprint }}</code></div>
            <div class="agent-permissions"><span v-for="permission in agent.permissions" :key="permission"><ShieldCheck/>{{ permissionLabel(permission) }}</span><em v-if="!agent.permissions.length">未申请主机权限</em></div>
            <div class="agent-capabilities"><span v-for="capability in agent.capabilities" :key="capability">{{capability}}</span><em v-if="!agent.capabilities.length">未声明能力</em></div>
            <small>最近心跳：{{ formatDate(agent.last_seen_at) }}</small>
          </div>
          <div class="agent-actions">
            <button v-if="agent.status==='claimed'" class="cloud-primary compact" :disabled="busy===`confirm:${agent.id}`" @click="confirmAgent(agent)"><CheckCircle2/>指纹一致，确认</button>
            <template v-if="agent.status!=='revoked'">
              <button v-if="revokeCandidate!==agent.id" class="cloud-icon-btn danger" title="撤销 Agent" :disabled="!!busy" @click="revokeAgent(agent)"><Trash2/></button>
              <button v-else class="agent-revoke-confirm" :disabled="busy===`revoke:${agent.id}`" @click="revokeAgent(agent)">确认撤销</button>
            </template>
          </div>
        </section>
      </div>
      <div v-else class="cloud-empty"><RadioTower/>还没有连接 Minecraft 主机</div>
      <details v-if="revokedAgents.length" class="revoked-agents"><summary>已撤销的主机（{{ revokedAgents.length }}）</summary><p v-for="agent in revokedAgents" :key="agent.id">{{ agent.name }} · {{ agent.platform }} · {{ formatDate(agent.revoked_at) }}</p></details>
    </article>

    <article class="cloud-panel agent-capability">
      <header><div><h3>Agent 能力模型</h3><p>每台主机的实际权限与能力以上方声明为准</p></div><ShieldCheck/></header>
      <div><span><CheckCircle2/>安全配对与独立凭据</span><span><CheckCircle2/>出站心跳与在线状态</span><span><CheckCircle2/>远程任务与人工批准</span><span><CheckCircle2/>可恢复的完整权限终端</span></div>
    </article>
  </section>
</template>
