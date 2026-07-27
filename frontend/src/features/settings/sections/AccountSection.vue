<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import {
  Activity, Check, CheckCircle2, CircleDashed, Cloud, Copy, Download, KeyRound,
  LockKeyhole, LogIn, LogOut, MailPlus, Monitor, Plus, RefreshCw, Save, Server,
  Settings2, ShieldCheck, SquareTerminal, Trash2, Upload, UserPlus, Users, XCircle,
} from 'lucide-vue-next'
import { cloudRequest, cloudSession, setCloudSession } from '../../cloud/client'
import type {
  ApiTokenCreated, ApiTokenItem, AuthResponse, CloudApproval, CloudDevice, CloudProfile,
  CloudStatus, CloudTeam, DeploymentCapability, Invitation, RelayProvider, SyncedSettings,
  TeamMember, UsageSummary,
} from '../../cloud/types'
import type { UiSettings } from '../types'
import { saveUi, uiSettings } from '../store'

type CloudTab = 'overview' | 'api' | 'team' | 'approvals' | 'deployments'
type AuthMode = 'login' | 'register'

const tab = ref<CloudTab>('overview')
const authMode = ref<AuthMode>('login')
const status = ref<CloudStatus | null>(null)
const profile = ref<CloudProfile | null>(null)
const devices = ref<CloudDevice[]>([])
const synced = ref<SyncedSettings | null>(null)
const teams = ref<CloudTeam[]>([])
const activeTeamId = ref('')
const members = ref<TeamMember[]>([])
const approvals = ref<CloudApproval[]>([])
const tokens = ref<ApiTokenItem[]>([])
const usage = ref<UsageSummary | null>(null)
const provider = ref<RelayProvider | null>(null)
const capability = ref<DeploymentCapability | null>(null)
const busy = ref('')
const message = ref('')
const messageKind = ref<'ok' | 'error'>('ok')
let messageTimer = 0

const authForm = ref({ email: '', password: '', nickname: '', device_name: 'Sculk 桌面工作台' })
const profileForm = ref({ nickname: '', avatar_url: '', locale: 'zh-CN' })
const tokenForm = ref({ label: '默认工作流', expires_in_days: 90 })
const createdToken = ref('')
const copied = ref(false)
const teamName = ref('')
const invitationCode = ref('')
const inviteForm = ref({ email: '', role: 'member' })
const createdInvitation = ref<Invitation | null>(null)
const approvalForm = ref({ title: '', summary: '', risk: 'medium' })
const decisionComment = ref<Record<string, string>>({})
const providerForm = ref({ name: '', base_url: '', api_key: '', default_model: '', enabled: true })

const activeTeam = computed(() => teams.value.find(item => item.id === activeTeamId.value) || null)
const pendingApprovals = computed(() => approvals.value.filter(item => item.status === 'pending').length)
const usageMax = computed(() => Math.max(1, ...(usage.value?.daily.map(item => item.total_tokens) || [1])))
const isAdmin = computed(() => profile.value?.role === 'admin')
const syncAge = computed(() => synced.value ? relativeTime(synced.value.updated_at) : '尚未同步')

const tabs: { key: CloudTab; label: string; icon: typeof Cloud; badge?: () => number }[] = [
  { key: 'overview', label: '账号概览', icon: Cloud },
  { key: 'api', label: 'API 中转', icon: Activity },
  { key: 'team', label: '我的团队', icon: Users },
  { key: 'approvals', label: '远程审批', icon: ShieldCheck, badge: () => pendingApprovals.value },
  { key: 'deployments', label: '云部署', icon: Server },
]

function flash(text: string, kind: 'ok' | 'error' = 'ok') {
  message.value = text
  messageKind.value = kind
  window.clearTimeout(messageTimer)
  messageTimer = window.setTimeout(() => { message.value = '' }, 3200)
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

function platformName() {
  const agent = navigator.userAgent
  if (agent.includes('Windows')) return 'Windows'
  if (agent.includes('Mac OS')) return 'macOS'
  if (agent.includes('Linux')) return 'Linux'
  return 'Web'
}

function formatDate(value?: string | null) {
  if (!value) return '从未'
  return new Intl.DateTimeFormat('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(value))
}

function relativeTime(value: string) {
  const seconds = Math.max(0, Math.floor((Date.now() - new Date(value).getTime()) / 1000))
  if (seconds < 60) return '刚刚'
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时前`
  return `${Math.floor(seconds / 86400)} 天前`
}

function compactNumber(value: number) {
  return new Intl.NumberFormat('zh-CN', { notation: value >= 10_000 ? 'compact' : 'standard', maximumFractionDigits: 1 }).format(value)
}

function roleLabel(role: string) {
  return ({ owner: '所有者', admin: '管理员', approver: '审批人', member: '成员', user: '用户' } as Record<string, string>)[role] || role
}

function canDecide(teamId: string) {
  return ['owner', 'admin', 'approver'].includes(teams.value.find(item => item.id === teamId)?.role || '')
}

async function initialize() {
  busy.value = 'initialize'
  try {
    status.value = await cloudRequest<CloudStatus>('/api/cloud/status', {}, false)
    if (status.value.available && cloudSession()) await loadWorkspace()
    capability.value = await cloudRequest<DeploymentCapability>('/api/cloud/deployments/capability', {}, false)
  } catch (error) {
    flash(errorText(error), 'error')
  } finally {
    busy.value = ''
  }
}

async function loadWorkspace() {
  const me = await cloudRequest<CloudProfile>('/api/cloud/me')
  profile.value = me
  profileForm.value = { nickname: me.nickname, avatar_url: me.avatar_url, locale: me.locale }
  const results = await Promise.allSettled([
    cloudRequest<CloudDevice[]>('/api/cloud/devices'),
    cloudRequest<SyncedSettings>('/api/cloud/sync/settings'),
    cloudRequest<CloudTeam[]>('/api/cloud/teams'),
    cloudRequest<CloudApproval[]>('/api/cloud/approvals'),
    cloudRequest<ApiTokenItem[]>('/api/cloud/tokens'),
    cloudRequest<UsageSummary>('/api/cloud/usage?days=30'),
  ])
  if (results[0].status === 'fulfilled') devices.value = results[0].value
  if (results[1].status === 'fulfilled') synced.value = results[1].value
  if (results[2].status === 'fulfilled') {
    teams.value = results[2].value
    if (!activeTeamId.value && teams.value.length) activeTeamId.value = teams.value[0].id
  }
  if (results[3].status === 'fulfilled') approvals.value = results[3].value
  if (results[4].status === 'fulfilled') tokens.value = results[4].value
  if (results[5].status === 'fulfilled') usage.value = results[5].value
  if (me.role === 'admin') await loadProvider()
}

async function authenticate() {
  busy.value = 'auth'
  try {
    const path = authMode.value === 'login' ? '/api/cloud/auth/login' : '/api/cloud/auth/register'
    const payload = {
      ...authForm.value,
      platform: platformName(),
      nickname: authForm.value.nickname.trim() || authForm.value.email.split('@')[0],
    }
    const result = await cloudRequest<AuthResponse>(path, { method: 'POST', body: JSON.stringify(payload) }, false)
    setCloudSession(result.access_token)
    profile.value = result.profile
    await loadWorkspace()
    flash(authMode.value === 'login' ? '已登录 Sculk Cloud' : '云账号已创建')
  } catch (error) {
    flash(errorText(error), 'error')
  } finally {
    busy.value = ''
  }
}

async function logout() {
  busy.value = 'logout'
  try { await cloudRequest<void>('/api/cloud/auth/logout', { method: 'POST' }) } catch {}
  setCloudSession('')
  profile.value = null
  devices.value = []
  teams.value = []
  approvals.value = []
  busy.value = ''
  flash('已退出云账号')
}

async function saveProfile() {
  busy.value = 'profile'
  try {
    profile.value = await cloudRequest<CloudProfile>('/api/cloud/me', { method: 'PATCH', body: JSON.stringify(profileForm.value) })
    flash('账号资料已保存')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function pushSettings() {
  if (!synced.value || !uiSettings.value) return
  busy.value = 'sync-up'
  try {
    synced.value = await cloudRequest<SyncedSettings>('/api/cloud/sync/settings', {
      method: 'PUT',
      body: JSON.stringify({ base_version: synced.value.version, payload: { ui: uiSettings.value } }),
    })
    flash(`设置已同步到云端（版本 ${synced.value.version}）`)
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function pullSettings() {
  busy.value = 'sync-down'
  try {
    synced.value = await cloudRequest<SyncedSettings>('/api/cloud/sync/settings')
    const cloudUi = synced.value.payload.ui as UiSettings | undefined
    if (!cloudUi) throw new Error('云端还没有可应用的工作台设置')
    const saved = await saveUi(cloudUi, '')
    if (!saved) throw new Error('云端设置已拉取，但无法写入本地工作台')
    flash(`已应用云端版本 ${synced.value.version}`)
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function revokeDevice(id: string) {
  busy.value = `device:${id}`
  try {
    await cloudRequest<void>(`/api/cloud/devices/${id}`, { method: 'DELETE' })
    devices.value = devices.value.filter(item => item.id !== id)
    flash('设备访问已移除')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function createToken() {
  busy.value = 'token'
  try {
    const result = await cloudRequest<ApiTokenCreated>('/api/cloud/tokens', { method: 'POST', body: JSON.stringify(tokenForm.value) })
    createdToken.value = result.token
    tokens.value.unshift(result.item)
    flash('API Token 已创建，请妥善保存')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function copyToken() {
  await navigator.clipboard.writeText(createdToken.value)
  copied.value = true
  window.setTimeout(() => { copied.value = false }, 1800)
}

async function revokeToken(id: string) {
  busy.value = `token:${id}`
  try {
    await cloudRequest<void>(`/api/cloud/tokens/${id}`, { method: 'DELETE' })
    tokens.value = tokens.value.filter(item => item.id !== id)
    flash('API Token 已撤销')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function loadProvider() {
  try {
    provider.value = await cloudRequest<RelayProvider>('/api/cloud/admin/relay-provider')
    providerForm.value = {
      name: provider.value.name,
      base_url: provider.value.base_url,
      api_key: '',
      default_model: provider.value.default_model,
      enabled: provider.value.enabled,
    }
  } catch (error) { flash(errorText(error), 'error') }
}

async function saveProvider() {
  busy.value = 'provider'
  try {
    provider.value = await cloudRequest<RelayProvider>('/api/cloud/admin/relay-provider', { method: 'PUT', body: JSON.stringify(providerForm.value) })
    providerForm.value.api_key = ''
    flash('中转上游配置已保存')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function createTeam() {
  busy.value = 'team'
  try {
    const item = await cloudRequest<CloudTeam>('/api/cloud/teams', { method: 'POST', body: JSON.stringify({ name: teamName.value }) })
    teams.value.unshift(item)
    activeTeamId.value = item.id
    teamName.value = ''
    flash('团队已创建')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function loadMembers() {
  if (!activeTeamId.value) { members.value = []; return }
  try { members.value = await cloudRequest<TeamMember[]>(`/api/cloud/teams/${activeTeamId.value}/members`) }
  catch (error) { flash(errorText(error), 'error') }
}

async function inviteMember() {
  if (!activeTeamId.value) return
  busy.value = 'invite'
  try {
    createdInvitation.value = await cloudRequest<Invitation>(`/api/cloud/teams/${activeTeamId.value}/invitations`, { method: 'POST', body: JSON.stringify(inviteForm.value) })
    inviteForm.value.email = ''
    flash('邀请已创建')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function acceptInvitation() {
  busy.value = 'accept'
  try {
    const item = await cloudRequest<CloudTeam>('/api/cloud/invitations/accept', { method: 'POST', body: JSON.stringify({ invite_code: invitationCode.value }) })
    teams.value = await cloudRequest<CloudTeam[]>('/api/cloud/teams')
    activeTeamId.value = item.id
    invitationCode.value = ''
    flash('已加入团队')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function createApproval() {
  if (!activeTeamId.value) return
  busy.value = 'approval'
  try {
    const item = await cloudRequest<CloudApproval>('/api/cloud/approvals', {
      method: 'POST',
      body: JSON.stringify({ team_id: activeTeamId.value, ...approvalForm.value, payload: { source: 'sculk-workbench' } }),
    })
    approvals.value.unshift(item)
    approvalForm.value = { title: '', summary: '', risk: 'medium' }
    flash('远程审批已发起')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

async function decideApproval(id: string, decision: 'approved' | 'rejected') {
  busy.value = `approval:${id}`
  try {
    const item = await cloudRequest<CloudApproval>(`/api/cloud/approvals/${id}/decision`, {
      method: 'POST', body: JSON.stringify({ decision, comment: decisionComment.value[id] || '' }),
    })
    const index = approvals.value.findIndex(approval => approval.id === id)
    if (index >= 0) approvals.value[index] = item
    flash(decision === 'approved' ? '审批已通过' : '审批已拒绝')
  } catch (error) { flash(errorText(error), 'error') } finally { busy.value = '' }
}

watch(activeTeamId, loadMembers)
onMounted(initialize)
</script>

<template>
  <div class="cloud-console">
    <header class="cloud-console-head">
      <div class="cloud-mark"><Cloud/></div>
      <div><p>Sculk Cloud</p><h2>{{ profile ? profile.nickname : '连接你的工作台' }}</h2></div>
      <span class="cloud-state" :class="{online:status?.available}"><i/>{{ status?.available ? '服务已连接' : '服务未配置' }}</span>
      <span v-if="profile" class="cloud-plan">{{ profile.plan.toUpperCase() }} · {{ roleLabel(profile.role) }}</span>
      <button v-if="profile" class="cloud-icon-btn" title="退出登录" :disabled="busy==='logout'" @click="logout"><LogOut/></button>
    </header>

    <div v-if="profile" class="sync-rail" aria-label="云同步状态">
      <span class="active"><Monitor/><b>{{ devices.length }}</b><small>已连接设备</small></span>
      <i/>
      <span :class="{active:synced}"><Cloud/><b>v{{ synced?.version || 1 }}</b><small>{{ syncAge }}</small></span>
      <i/>
      <span :class="{active:teams.length}"><Users/><b>{{ teams.length }}</b><small>协作团队</small></span>
    </div>

    <div v-if="message" class="cloud-toast" :class="messageKind">{{ message }}</div>

    <section v-if="!status?.available && !busy" class="cloud-unavailable">
      <div><CircleDashed/><h3>云服务等待配置</h3><p>{{ status?.message || '无法读取 Sculk Cloud 状态' }}</p></div>
      <dl><div><dt>PostgreSQL</dt><dd>DATABASE_URL</dd></div><div><dt>Redis</dt><dd>REDIS_URL</dd></div><div><dt>密钥</dt><dd>SCULK_MASTER_KEY</dd></div></dl>
    </section>

    <section v-else-if="!profile" class="cloud-auth">
      <div class="cloud-auth-copy"><LockKeyhole/><h3>{{ authMode==='login' ? '登录云账号' : '创建云账号' }}</h3><p>设置、设备、团队审批和 API 用量会归入同一个 Sculk Cloud 身份。</p><ul><li><Check/>端到端会话撤销</li><li><Check/>版本化设置同步</li><li><Check/>团队角色与审批审计</li></ul></div>
      <form @submit.prevent="authenticate">
        <div class="cloud-auth-switch"><button type="button" :class="{active:authMode==='login'}" @click="authMode='login'">登录</button><button type="button" :class="{active:authMode==='register'}" @click="authMode='register'">注册</button></div>
        <label v-if="authMode==='register'">昵称<input v-model="authForm.nickname" maxlength="32" autocomplete="nickname" placeholder="你的显示名称"/></label>
        <label>邮箱<input v-model="authForm.email" type="email" autocomplete="email" placeholder="owner@example.com" required/></label>
        <label>密码<input v-model="authForm.password" type="password" :autocomplete="authMode==='login'?'current-password':'new-password'" minlength="8" required/></label>
        <label>设备名称<input v-model="authForm.device_name" maxlength="48"/></label>
        <button class="cloud-primary" :disabled="busy==='auth'"><RefreshCw v-if="busy==='auth'" class="s-spin"/><LogIn v-else-if="authMode==='login'"/><UserPlus v-else/>{{ authMode==='login' ? '登录 Sculk Cloud' : '创建并登录' }}</button>
      </form>
    </section>

    <template v-else>
      <nav class="cloud-tabs">
        <button v-for="item in tabs" :key="item.key" :class="{active:tab===item.key}" :aria-label="item.label" :title="item.label" @click="tab=item.key"><component :is="item.icon"/><span>{{ item.label }}</span><em v-if="item.badge?.()">{{ item.badge() }}</em></button>
      </nav>

      <section v-if="tab==='overview'" class="cloud-view cloud-overview">
        <article class="cloud-panel profile-panel">
          <header><div><h3>账号资料</h3><p>{{ profile.email }}</p></div><span>{{ profile.id.slice(0,8) }}</span></header>
          <div class="profile-avatar"><span v-if="!profileForm.avatar_url">{{ profileForm.nickname.slice(0,1).toUpperCase() }}</span><img v-else :src="profileForm.avatar_url" alt="账号头像"/></div>
          <label>昵称<input v-model="profileForm.nickname" maxlength="32"/></label>
          <label>头像地址<input v-model="profileForm.avatar_url" type="url" placeholder="https://…"/></label>
          <label>界面语言<select v-model="profileForm.locale"><option value="zh-CN">简体中文</option><option value="en-US">English</option></select></label>
          <button class="cloud-primary compact" :disabled="busy==='profile'" @click="saveProfile"><Save/>保存资料</button>
        </article>

        <article class="cloud-panel sync-panel">
          <header><div><h3>多设备同步</h3><p>云端版本 {{ synced?.version || 1 }} · {{ syncAge }}</p></div><RefreshCw :class="{'s-spin':busy.startsWith('sync-')}"/></header>
          <div class="sync-version"><span>本机</span><i/><b>设置快照</b><i/><span>云端 v{{ synced?.version || 1 }}</span></div>
          <div class="cloud-actions"><button :disabled="!!busy" @click="pullSettings"><Download/>应用云端</button><button class="accent" :disabled="!!busy" @click="pushSettings"><Upload/>同步本机</button></div>
          <small>冲突时不会覆盖云端，会要求先拉取最新版本。</small>
        </article>

        <article class="cloud-panel devices-panel">
          <header><div><h3>登录设备</h3><p>{{ devices.length }} 台设备拥有账号访问权</p></div><Monitor/></header>
          <div class="device-list">
            <div v-for="device in devices" :key="device.id"><span><Monitor/></span><p><b>{{ device.name }} <em v-if="device.current">当前设备</em></b><small>{{ device.platform }} · {{ formatDate(device.last_seen_at) }}</small></p><button v-if="!device.current" class="cloud-icon-btn danger" title="移除此设备" :disabled="busy===`device:${device.id}`" @click="revokeDevice(device.id)"><Trash2/></button></div>
          </div>
        </article>
      </section>

      <section v-else-if="tab==='api'" class="cloud-view">
        <div class="usage-strip"><div><span>近 30 天请求</span><b>{{ compactNumber(usage?.requests || 0) }}</b></div><div><span>输入 Token</span><b>{{ compactNumber(usage?.prompt_tokens || 0) }}</b></div><div><span>输出 Token</span><b>{{ compactNumber(usage?.completion_tokens || 0) }}</b></div><div><span>总用量</span><b>{{ compactNumber(usage?.total_tokens || 0) }}</b></div></div>
        <article class="cloud-panel usage-panel">
          <header><div><h3>Token 使用记录</h3><p>按日汇总 · 最近 30 天</p></div><Activity/></header>
          <div v-if="usage?.daily.length" class="usage-chart"><div v-for="day in usage.daily" :key="day.day" :title="`${day.day} · ${day.total_tokens} Token`"><i :style="{height:`${Math.max(5,day.total_tokens/usageMax*100)}%`}"/><span>{{ day.day.slice(5) }}</span></div></div>
          <div v-else class="cloud-empty"><Activity/>还没有中转调用记录</div>
        </article>

        <article class="cloud-panel token-panel">
          <header><div><h3>个人 API Token</h3><p>用于调用 /api/cloud/v1/chat/completions</p></div><KeyRound/></header>
          <form class="cloud-inline-form" @submit.prevent="createToken"><input v-model="tokenForm.label" maxlength="48" placeholder="Token 名称" required/><select v-model.number="tokenForm.expires_in_days"><option :value="30">30 天</option><option :value="90">90 天</option><option :value="365">1 年</option></select><button class="cloud-primary compact" :disabled="busy==='token'"><Plus/>创建</button></form>
          <div v-if="createdToken" class="token-reveal"><LockKeyhole/><code>{{ createdToken }}</code><button class="cloud-icon-btn" title="复制 Token" @click="copyToken"><Check v-if="copied"/><Copy v-else/></button></div>
          <div class="token-list"><div v-for="item in tokens" :key="item.id"><span><KeyRound/></span><p><b>{{ item.label }}</b><code>{{ item.token_prefix }}••••••</code></p><p class="token-usage"><b>{{ compactNumber(item.total_tokens) }} Token</b><small>{{ item.request_count }} 次请求 · {{ formatDate(item.last_used_at) }}</small></p><button class="cloud-icon-btn danger" title="撤销 Token" :disabled="busy===`token:${item.id}`" @click="revokeToken(item.id)"><Trash2/></button></div></div>
        </article>

        <article v-if="isAdmin" class="cloud-panel provider-panel">
          <header><div><h3>中转上游</h3><p>管理员配置 · API Key 使用 AES-256-GCM 加密保存</p></div><Settings2/></header>
          <form @submit.prevent="saveProvider"><label>服务名称<input v-model="providerForm.name" placeholder="OpenAI 官方" required/></label><label>Base URL<input v-model="providerForm.base_url" type="url" placeholder="https://api.openai.com" required/></label><label>默认模型<input v-model="providerForm.default_model" placeholder="gpt-5-mini"/></label><label>API Key<input v-model="providerForm.api_key" type="password" :placeholder="provider?.configured ? '留空以保持当前密钥' : 'sk-…'"/></label><label class="provider-toggle"><input v-model="providerForm.enabled" type="checkbox"/><span>启用中转</span></label><button class="cloud-primary compact" :disabled="busy==='provider'"><Save/>保存上游</button></form>
        </article>
      </section>

      <section v-else-if="tab==='team'" class="cloud-view team-view">
        <div class="team-toolbar"><select v-model="activeTeamId"><option value="" disabled>{{ teams.length ? '选择团队' : '尚未创建团队' }}</option><option v-for="item in teams" :key="item.id" :value="item.id">{{ item.name }} · {{ roleLabel(item.role) }}</option></select><form @submit.prevent="createTeam"><input v-model="teamName" maxlength="48" placeholder="新团队名称" required/><button class="cloud-primary compact" :disabled="busy==='team'"><Plus/>创建团队</button></form></div>
        <article v-if="activeTeam" class="cloud-panel team-summary"><header><div><h3>{{ activeTeam.name }}</h3><p>{{ activeTeam.slug }} · {{ roleLabel(activeTeam.role) }}</p></div><span>{{ activeTeam.member_count }} 人</span></header><div class="member-list"><div v-for="member in members" :key="member.id"><span>{{ member.nickname.slice(0,1).toUpperCase() }}</span><p><b>{{ member.nickname }}</b><small>{{ member.email }}</small></p><em>{{ roleLabel(member.role) }}</em></div></div></article>
        <article v-if="activeTeam && ['owner','admin'].includes(activeTeam.role)" class="cloud-panel invite-panel"><header><div><h3>邀请成员</h3><p>邀请码仅与指定邮箱匹配，7 天内有效</p></div><MailPlus/></header><form class="cloud-inline-form" @submit.prevent="inviteMember"><input v-model="inviteForm.email" type="email" placeholder="member@example.com" required/><select v-model="inviteForm.role"><option value="member">成员</option><option value="approver">审批人</option><option value="admin">管理员</option></select><button class="cloud-primary compact" :disabled="busy==='invite'"><UserPlus/>创建邀请</button></form><div v-if="createdInvitation" class="invite-code"><code>{{ createdInvitation.invite_code }}</code><span>{{ createdInvitation.email }} · {{ roleLabel(createdInvitation.role) }}</span></div></article>
        <article class="cloud-panel join-panel"><header><div><h3>加入团队</h3><p>使用管理员发来的 Sculk Cloud 邀请码</p></div><Users/></header><form class="cloud-inline-form" @submit.prevent="acceptInvitation"><input v-model="invitationCode" placeholder="sci_…" required/><button class="cloud-primary compact" :disabled="busy==='accept'"><Check/>加入</button></form></article>
      </section>

      <section v-else-if="tab==='approvals'" class="cloud-view approvals-view">
        <article class="cloud-panel approval-create"><header><div><h3>发起远程审批</h3><p>{{ activeTeam ? activeTeam.name : '请先创建或加入团队' }}</p></div><ShieldCheck/></header><form @submit.prevent="createApproval"><label>操作标题<input v-model="approvalForm.title" maxlength="120" placeholder="例如：重启正式服并应用新配置" required/></label><label>变更摘要<textarea v-model="approvalForm.summary" rows="3" placeholder="包含影响范围与回滚方式"/></label><label>风险等级<select v-model="approvalForm.risk"><option value="low">低风险</option><option value="medium">中风险</option><option value="high">高风险</option></select></label><button class="cloud-primary compact" :disabled="!activeTeamId||busy==='approval'"><ShieldCheck/>提交审批</button></form></article>
        <div class="approval-list"><article v-for="item in approvals" :key="item.id" class="approval-item" :class="item.status"><header><span :class="`risk ${item.risk}`">{{ {low:'低',medium:'中',high:'高'}[item.risk] }}风险</span><em>{{ item.team_name }}</em><time>{{ formatDate(item.created_at) }}</time></header><h3>{{ item.title }}</h3><p>{{ item.summary || '无补充摘要' }}</p><footer><span><CheckCircle2 v-if="item.status==='approved'"/><XCircle v-else-if="item.status==='rejected'"/><CircleDashed v-else/>{{ {pending:'等待审批',approved:'已通过',rejected:'已拒绝',cancelled:'已取消'}[item.status] }} · {{ item.requester_name }}</span><template v-if="item.status==='pending' && canDecide(item.team_id)"><input v-model="decisionComment[item.id]" placeholder="审批意见（可选）"/><button class="reject" :disabled="busy===`approval:${item.id}`" @click="decideApproval(item.id,'rejected')"><XCircle/>拒绝</button><button class="approve" :disabled="busy===`approval:${item.id}`" @click="decideApproval(item.id,'approved')"><CheckCircle2/>通过</button></template></footer></article><div v-if="!approvals.length" class="cloud-empty"><ShieldCheck/>当前账号没有审批记录</div></div>
      </section>

      <section v-else class="cloud-view deployment-view">
        <header class="deployment-head"><div><Server/><span>PREVIEW API</span></div><h3>开服器云部署</h3><p>接口契约已经固定，计算资源与计费能力尚未开放。</p><em>{{ capability?.api_version }}</em></header>
        <div class="deployment-pipeline"><span class="ready"><Check/>账号与团队</span><i/><span class="ready"><Check/>审批策略</span><i/><span><CircleDashed/>计算资源</span><i/><span><CircleDashed/>区域部署</span></div>
        <article class="cloud-panel endpoint-panel"><header><div><h3>预留接口</h3><p>当前创建请求返回 HTTP 501 和 deployment_planned</p></div><SquareTerminal/></header><div v-for="endpoint in capability?.reserved_endpoints" :key="endpoint"><code>{{ endpoint }}</code><span>reserved</span></div></article>
        <article class="cloud-panel reserved-panel"><Server/><div><h3>部署资源尚未开放</h3><p>账号、团队和审批数据模型会直接复用于未来的云开服工作流。</p></div><button disabled>等待开放</button></article>
      </section>
    </template>
  </div>
</template>
