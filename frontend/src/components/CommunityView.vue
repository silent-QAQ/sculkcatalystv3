<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  AlertTriangle, BarChart3, Box, Check, ChevronDown, ChevronUp, Database,
  LoaderCircle, MapPin, MessageSquareText, Pencil, Plus, PlugZap, RefreshCw, Save,
  Search, Trash2, Users, Vote, X,
} from 'lucide-vue-next'
import { apiRequest } from '../lib/api'

const props = defineProps<{ serverId: string }>()

type PlayerSort = 'name' | 'status' | 'level' | 'updated_at'
type SortOrder = 'asc' | 'desc'
type PapiValueStatus = 'ok' | 'unresolved' | 'unavailable'

interface PlayerPosition {
  world: string
  x: number
  y: number
  z: number
}

interface PlayerListItem {
  id: string
  uuid?: string
  name: string
  status: string
  display_name?: string | null
  role?: string | null
  level?: number | null
  position?: PlayerPosition | null
  updated_at?: string | null
  source: string
}

interface Item {
  id: string
  count: number
  slot?: number
  name?: string | null
  lore: string[]
  container_kind?: 'shulker_box' | 'bundle' | 'container' | null
  contents?: Item[]
}

interface InventorySlot {
  slot: number
  item?: Item | null
}

interface Inventory {
  slots: InventorySlot[]
}

interface PlayerProfile extends PlayerListItem {
  note?: string | null
  tags: string[]
  inventory: Inventory
  ender_chest: Inventory
}

interface PlayerSource {
  kind: string
  label: string
  detail: string
}

interface PlayerListResponse {
  players: PlayerListItem[]
  source: PlayerSource
  total: number
}

interface PapiField {
  id: string
  label: string
  placeholder: string
  enabled: boolean
}

interface PapiFieldsResponse {
  fields: PapiField[]
  available: boolean
  detail: string
}

interface PapiValue {
  field_id: string
  label: string
  placeholder: string
  value?: string | null
  status: PapiValueStatus
}

interface PlayerPapiResponse {
  available: boolean
  detail: string
  values: PapiValue[]
}

interface Feedback {
  id: string
  server_id: string
  player: string
  content: string
  category: string
  sentiment: string
  status: string
}

interface PollOption {
  id: string
  label: string
  votes: number
}

interface Poll {
  id: string
  server_id: string
  title: string
  status: string
  options: PollOption[]
  closes_at: string
}

interface CommunityResponse {
  feedback: Feedback[]
  polls: Poll[]
}

interface FeedbackCluster {
  categories: Record<string, number>
  summary: string
}

interface GridCell {
  slot: number
  item: Item | null
}

const query = ref('')
const sort = ref<{ key: PlayerSort; order: SortOrder }>({ key: 'name', order: 'asc' })
const players = ref<PlayerListItem[]>([])
const playerSource = ref<PlayerSource | null>(null)
const playerTotal = ref(0)
const playerLoading = ref(false)
const playerError = ref('')

const feedback = ref<Feedback[]>([])
const polls = ref<Poll[]>([])
const communityError = ref('')
const cluster = ref<FeedbackCluster | null>(null)
const showPoll = ref(false)
const pollTitle = ref('')
const pollOptions = ref(['', ''])

const showProfile = ref(false)
const selectedPlayer = ref<PlayerListItem | null>(null)
const profile = ref<PlayerProfile | null>(null)
const profileLoading = ref(false)
const profileSaving = ref(false)
const profileError = ref('')
const profileEditMode = ref(false)
const profileDraft = ref({ display_name: '', role: '', note: '', tags: '' })

const hoveredContainer = ref<Item | null>(null)
const pinnedContainer = ref<Item | null>(null)

const papiFields = ref<PapiField[]>([])
const papiAvailable = ref(false)
const papiDetail = ref('尚未读取 PlaceholderAPI 状态')
const papiLoading = ref(false)
const papiError = ref('')
const playerPapiValues = ref<PapiValue[]>([])
const playerPapiLoading = ref(false)
const playerPapiError = ref('')
const showPapiManager = ref(false)
const papiDraft = ref<PapiField[]>([])
const papiSaving = ref(false)
const papiSaveError = ref('')

let playerRequest = 0
let profileRequest = 0
let papiRequest = 0
let queryTimer: ReturnType<typeof setTimeout> | undefined

const onlinePlayers = computed(() => players.value.filter((player) => player.status === 'online').length)
const enabledPapiFieldCount = computed(() => papiFields.value.filter((field) => field.enabled).length)
const serverFeedback = computed(() => feedback.value.filter((item) => item.server_id === props.serverId))
const serverPolls = computed(() => polls.value.filter((poll) => poll.server_id === props.serverId))
const activeContainerPreview = computed(() => pinnedContainer.value ?? hoveredContainer.value)
const inventoryCells = computed(() => toInventoryGrid(profile.value?.inventory, 36))
const enderChestCells = computed(() => toInventoryGrid(profile.value?.ender_chest, 27))

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

function cloneFields(fields: PapiField[]) {
  return fields.map((field) => ({ ...field }))
}

function normalizeProfile(value: PlayerProfile): PlayerProfile {
  return {
    ...value,
    tags: Array.isArray(value.tags) ? value.tags.filter((tag) => typeof tag === 'string') : [],
    inventory: { slots: Array.isArray(value.inventory?.slots) ? value.inventory.slots : [] },
    ender_chest: { slots: Array.isArray(value.ender_chest?.slots) ? value.ender_chest.slots : [] },
  }
}

function playerPath(playerId: string) {
  return `/api/servers/${encodeURIComponent(props.serverId)}/players/${encodeURIComponent(playerId)}`
}

async function loadPlayers() {
  const serverId = props.serverId
  if (!serverId) return
  const request = ++playerRequest
  const params = new URLSearchParams({
    query: query.value.trim(),
    sort: sort.value.key,
    order: sort.value.order,
  })
  playerLoading.value = true
  playerError.value = ''
  try {
    const response = await apiRequest<PlayerListResponse>(`/api/servers/${encodeURIComponent(serverId)}/players?${params}`)
    if (request !== playerRequest || serverId !== props.serverId) return
    players.value = Array.isArray(response.players) ? response.players : []
    playerSource.value = response.source ?? null
    playerTotal.value = Number.isFinite(response.total) ? response.total : players.value.length
  } catch (error) {
    if (request !== playerRequest || serverId !== props.serverId) return
    playerError.value = errorMessage(error)
    playerSource.value = null
    playerTotal.value = 0
    players.value = []
  } finally {
    if (request === playerRequest) playerLoading.value = false
  }
}

async function loadCommunity() {
  const serverId = props.serverId
  if (!serverId) return
  communityError.value = ''
  try {
    const response = await apiRequest<CommunityResponse>('/api/community')
    if (serverId !== props.serverId) return
    feedback.value = Array.isArray(response.feedback) ? response.feedback : []
    polls.value = Array.isArray(response.polls) ? response.polls : []
  } catch (error) {
    if (serverId === props.serverId) communityError.value = errorMessage(error)
  }
}

async function loadPapiFields() {
  const serverId = props.serverId
  if (!serverId) return
  papiLoading.value = true
  papiError.value = ''
  try {
    const response = await apiRequest<PapiFieldsResponse>(`/api/servers/${encodeURIComponent(serverId)}/papi/fields`)
    if (serverId !== props.serverId) return
    papiFields.value = Array.isArray(response.fields) ? response.fields : []
    papiDraft.value = cloneFields(papiFields.value)
    papiAvailable.value = response.available
    papiDetail.value = response.detail || (response.available ? 'PlaceholderAPI 已连接' : 'PlaceholderAPI 当前不可用')
  } catch (error) {
    if (serverId !== props.serverId) return
    papiFields.value = []
    papiDraft.value = []
    papiAvailable.value = false
    papiError.value = errorMessage(error)
    papiDetail.value = '未能读取 PlaceholderAPI 配置'
  } finally {
    if (serverId === props.serverId) papiLoading.value = false
  }
}

function schedulePlayerLoad() {
  if (queryTimer) clearTimeout(queryTimer)
  queryTimer = setTimeout(() => {
    queryTimer = undefined
    void loadPlayers()
  }, 260)
}

function changeSort(key: PlayerSort) {
  sort.value = sort.value.key === key
    ? { key, order: sort.value.order === 'asc' ? 'desc' : 'asc' }
    : { key, order: key === 'name' ? 'asc' : 'desc' }
  void loadPlayers()
}

function sortLabel(key: PlayerSort) {
  const labels: Record<PlayerSort, string> = {
    name: '玩家',
    status: '状态',
    level: '等级',
    updated_at: '更新',
  }
  return labels[key]
}

function resetProfileDraft(value: PlayerProfile) {
  profileDraft.value = {
    display_name: value.display_name ?? '',
    role: value.role ?? '',
    note: value.note ?? '',
    tags: value.tags.join(', '),
  }
}

async function openProfile(player: PlayerListItem) {
  const serverId = props.serverId
  selectedPlayer.value = player
  profile.value = null
  profileError.value = ''
  profileEditMode.value = false
  playerPapiValues.value = []
  playerPapiError.value = ''
  hoveredContainer.value = null
  pinnedContainer.value = null
  showProfile.value = true
  const request = ++profileRequest
  profileLoading.value = true
  try {
    const response = await apiRequest<PlayerProfile>(`${playerPath(player.id)}`)
    if (request !== profileRequest || serverId !== props.serverId || selectedPlayer.value?.id !== player.id) return
    profile.value = normalizeProfile(response)
    resetProfileDraft(profile.value)
    void loadPlayerPapi(profile.value.id)
  } catch (error) {
    if (request === profileRequest && serverId === props.serverId) profileError.value = errorMessage(error)
  } finally {
    if (request === profileRequest) profileLoading.value = false
  }
}

function closeProfile() {
  profileRequest += 1
  papiRequest += 1
  showProfile.value = false
  selectedPlayer.value = null
  profile.value = null
  profileEditMode.value = false
  hoveredContainer.value = null
  pinnedContainer.value = null
  playerPapiValues.value = []
}

function closePapiManager() {
  showPapiManager.value = false
  papiSaveError.value = ''
}

function openPapiManager() {
  papiDraft.value = cloneFields(papiFields.value)
  papiSaveError.value = ''
  showPapiManager.value = true
}

function newPapiFieldId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  return `papi-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
}

function addPapiField() {
  papiDraft.value = [
    ...papiDraft.value,
    { id: newPapiFieldId(), label: '', placeholder: '', enabled: true },
  ]
}

function removePapiField(id: string) {
  papiDraft.value = papiDraft.value.filter((field) => field.id !== id)
}

async function savePapiFields() {
  const fields = papiDraft.value.map((field) => ({
    ...field,
    label: field.label.trim(),
    placeholder: field.placeholder.trim(),
  }))
  if (fields.some((field) => !field.label || !field.placeholder)) {
    papiSaveError.value = '请完整填写变量名称和占位符。'
    return
  }
  papiSaving.value = true
  papiSaveError.value = ''
  try {
    const response = await apiRequest<PapiFieldsResponse>(`/api/servers/${encodeURIComponent(props.serverId)}/papi/fields`, {
      method: 'PUT',
      body: JSON.stringify({ fields }),
    })
    papiFields.value = Array.isArray(response.fields) ? response.fields : []
    papiDraft.value = cloneFields(papiFields.value)
    papiAvailable.value = response.available
    papiDetail.value = response.detail || (response.available ? 'PlaceholderAPI 已连接' : 'PlaceholderAPI 当前不可用')
    showPapiManager.value = false
    if (profile.value) void loadPlayerPapi(profile.value.id)
  } catch (error) {
    papiSaveError.value = errorMessage(error)
  } finally {
    papiSaving.value = false
  }
}

async function loadPlayerPapi(playerId: string) {
  const serverId = props.serverId
  const request = ++papiRequest
  playerPapiLoading.value = true
  playerPapiError.value = ''
  try {
    const response = await apiRequest<PlayerPapiResponse>(`${playerPath(playerId)}/papi`)
    if (request !== papiRequest || serverId !== props.serverId || profile.value?.id !== playerId) return
    papiAvailable.value = response.available
    papiDetail.value = response.detail || (response.available ? 'PlaceholderAPI 已连接' : 'PlaceholderAPI 当前不可用')
    playerPapiValues.value = Array.isArray(response.values) ? response.values : []
  } catch (error) {
    if (request === papiRequest && serverId === props.serverId) {
      playerPapiError.value = errorMessage(error)
      playerPapiValues.value = []
    }
  } finally {
    if (request === papiRequest) playerPapiLoading.value = false
  }
}

async function saveProfile() {
  if (!profile.value || profileSaving.value) return
  const playerId = profile.value.id
  profileSaving.value = true
  profileError.value = ''
  try {
    const response = await apiRequest<PlayerProfile>(playerPath(playerId), {
      method: 'PUT',
      body: JSON.stringify({
        display_name: profileDraft.value.display_name.trim(),
        role: profileDraft.value.role.trim(),
        note: profileDraft.value.note.trim(),
        tags: profileDraft.value.tags.split(/[,，\n]/).map((tag) => tag.trim()).filter(Boolean),
      }),
    })
    const updated = normalizeProfile(response)
    if (profile.value?.id !== playerId) return
    profile.value = updated
    selectedPlayer.value = { ...selectedPlayer.value!, ...updated }
    players.value = players.value.map((player) => player.id === playerId ? { ...player, ...updated } : player)
    resetProfileDraft(updated)
    profileEditMode.value = false
  } catch (error) {
    profileError.value = errorMessage(error)
  } finally {
    profileSaving.value = false
  }
}

function cancelProfileEdit() {
  if (profile.value) resetProfileDraft(profile.value)
  profileEditMode.value = false
}

function toInventoryGrid(inventory: Inventory | undefined, minimumSlots: number): GridCell[] {
  const itemBySlot = new Map<number, Item>()
  for (const entry of inventory?.slots ?? []) {
    const slot = Number(entry?.slot)
    if (!Number.isInteger(slot) || slot < 0 || slot >= 54 || !entry.item) continue
    itemBySlot.set(slot, entry.item)
  }
  const largestSlot = Math.max(minimumSlots - 1, ...itemBySlot.keys())
  const length = Math.min(54, Math.max(minimumSlots, Math.ceil((largestSlot + 1) / 9) * 9))
  return Array.from({ length }, (_, slot) => ({ slot, item: itemBySlot.get(slot) ?? null }))
}

function containerCells(item: Item): GridCell[] {
  const contents = Array.isArray(item.contents) ? item.contents : []
  const itemBySlot = new Map<number, Item>()
  contents.forEach((entry, index) => {
    const slot = Number.isInteger(entry.slot) && (entry.slot as number) >= 0 ? entry.slot as number : index
    if (slot < 54) itemBySlot.set(slot, entry)
  })
  const largestSlot = Math.max(8, ...itemBySlot.keys())
  const length = Math.min(54, Math.max(9, Math.ceil((largestSlot + 1) / 9) * 9))
  return Array.from({ length }, (_, slot) => ({ slot, item: itemBySlot.get(slot) ?? null }))
}

function itemName(item: Item) {
  return item.name?.trim() || item.id.replace(/^minecraft:/, '').replace(/_/g, ' ')
}

function itemShortName(item: Item) {
  const label = itemName(item)
  return label.length > 16 ? `${label.slice(0, 15)}...` : label
}

function itemClass(item: Item) {
  return item.container_kind === 'shulker_box' ? 'shulker' : item.container_kind === 'bundle' ? 'bundle' : item.container_kind ? 'container' : ''
}

function hasContents(item: Item | null) {
  return Boolean(item && item.container_kind && Array.isArray(item.contents))
}

function containerKindLabel(item: Item) {
  if (item.container_kind === 'shulker_box') return '潜影盒'
  if (item.container_kind === 'bundle') return '收纳袋'
  return '容器'
}

function itemTooltip(item: Item) {
  const lore = Array.isArray(item.lore) && item.lore.length ? `\n${item.lore.join('\n')}` : ''
  const container = hasContents(item) ? `\n${containerKindLabel(item)}：${item.contents?.length ?? 0} 个物品` : ''
  return `${itemName(item)} x${item.count}${container}${lore}`
}

function previewContainer(item: Item | null) {
  hoveredContainer.value = hasContents(item) ? item : null
}

function clearContainerPreview(item: Item | null) {
  // Keep the last hover visible while the pointer moves from the grid into the preview.
  // Hovering another slot (including an empty slot) replaces it immediately.
  if (pinnedContainer.value === item) return
}

function toggleContainerPreview(item: Item | null) {
  if (!hasContents(item)) return
  pinnedContainer.value = pinnedContainer.value === item ? null : item
}

function statusClass(status: string) {
  return ['online', 'offline', 'banned'].includes(status) ? status : 'unknown'
}

function statusLabel(status: string) {
  if (status === 'online') return '在线'
  if (status === 'offline') return '离线'
  if (status === 'banned') return '已封禁'
  return status || '未知'
}

function papiStatusLabel(status: PapiValueStatus) {
  if (status === 'ok') return '已解析'
  if (status === 'unresolved') return '未解析'
  return '不可用'
}

function formatPosition(position?: PlayerPosition | null) {
  if (!position) return '未记录'
  const format = (value: number) => Number.isFinite(value) ? value.toFixed(1).replace(/\.0$/, '') : '?'
  return `${position.world} ${format(position.x)}, ${format(position.y)}, ${format(position.z)}`
}

function formatUpdatedAt(value?: string | null) {
  if (!value) return '未记录'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('zh-CN', { hour12: false })
}

function sourceSummary() {
  if (!playerSource.value) return '数据源尚未返回'
  return playerSource.value.detail || playerSource.value.label || playerSource.value.kind
}

async function clusterFeedback() {
  try {
    cluster.value = await apiRequest<FeedbackCluster>('/api/feedback/cluster', { method: 'POST' })
  } catch (error) {
    communityError.value = errorMessage(error)
  }
}

async function createPoll() {
  const options = pollOptions.value.filter((option) => option.trim())
  if (!pollTitle.value.trim() || options.length < 2) return
  try {
    await apiRequest<Poll>('/api/polls', {
      method: 'POST',
      body: JSON.stringify({ server_id: props.serverId, title: pollTitle.value, options }),
    })
    pollTitle.value = ''
    pollOptions.value = ['', '']
    showPoll.value = false
    await loadCommunity()
  } catch (error) {
    communityError.value = errorMessage(error)
  }
}

async function voteFor(poll: Poll, option: PollOption) {
  try {
    await apiRequest<Poll>(`/api/polls/${encodeURIComponent(poll.id)}/vote`, {
      method: 'POST',
      body: JSON.stringify({ option_id: option.id }),
    })
    await loadCommunity()
  } catch (error) {
    communityError.value = errorMessage(error)
  }
}

function refreshPlayerArea() {
  void Promise.all([loadPlayers(), loadPapiFields()])
  if (profile.value) void loadPlayerPapi(profile.value.id)
}

function handleKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  if (showPapiManager.value) closePapiManager()
  else if (showProfile.value) closeProfile()
}

watch(query, schedulePlayerLoad)
watch(() => props.serverId, (serverId) => {
  playerRequest += 1
  profileRequest += 1
  papiRequest += 1
  if (queryTimer) {
    clearTimeout(queryTimer)
    queryTimer = undefined
  }
  players.value = []
  playerSource.value = null
  playerTotal.value = 0
  playerError.value = ''
  feedback.value = []
  polls.value = []
  cluster.value = null
  closeProfile()
  closePapiManager()
  if (!serverId) return
  void Promise.all([loadPlayers(), loadCommunity(), loadPapiFields()])
}, { immediate: true })

onMounted(() => window.addEventListener('keydown', handleKeydown))
onUnmounted(() => {
  if (queryTimer) clearTimeout(queryTimer)
  window.removeEventListener('keydown', handleKeydown)
})
</script>

<template>
  <div class="community-scroll">
    <section class="community-hero">
      <div>
        <span><Users/></span>
        <p><small>PLAYER MANAGEMENT</small><b>玩家管理</b><em>按服务器数据源查看玩家资料、物品与自定义变量。</em></p>
      </div>
      <div class="hero-actions">
        <button class="icon-button" title="刷新玩家与变量状态" :disabled="playerLoading || papiLoading" @click="refreshPlayerArea"><RefreshCw :class="{spin:playerLoading || papiLoading}"/></button>
        <button class="papi-button" @click="openPapiManager"><PlugZap/>PlaceholderAPI</button>
      </div>
    </section>

    <div class="player-stats">
      <article><span class="mint"><Users/></span><p><small>玩家记录</small><b>{{ playerTotal }}</b><em>当前服务器</em></p></article>
      <article><span class="green"><Check/></span><p><small>当前页在线</small><b>{{ onlinePlayers }}</b><em>搜索结果中</em></p></article>
      <article><span class="violet"><PlugZap/></span><p><small>显示变量</small><b>{{ enabledPapiFieldCount }}</b><em>{{ papiAvailable ? 'PlaceholderAPI 可用' : '等待插件可用' }}</em></p></article>
      <article><span class="amber"><MessageSquareText/></span><p><small>待处理反馈</small><b>{{ serverFeedback.filter((item) => item.status === 'new').length }}</b><em>当前服务器</em></p></article>
    </div>

    <section class="player-management-card">
      <header class="player-card-header">
        <div>
          <small>PLAYER DIRECTORY</small>
          <b>玩家列表</b>
          <em class="source-status" :class="{available: !!playerSource}"><Database/>{{ playerSource?.label || '数据源尚未返回' }}</em>
        </div>
        <div class="player-tools">
          <label class="player-search"><Search/><input v-model="query" placeholder="搜索名称、身份或 UUID"/></label>
          <button class="icon-button" title="刷新列表" :disabled="playerLoading" @click="loadPlayers"><RefreshCw :class="{spin:playerLoading}"/></button>
        </div>
      </header>
      <p v-if="playerSource" class="source-detail">{{ sourceSummary() }}</p>
      <p v-if="playerError" class="inline-error"><AlertTriangle/>{{ playerError }}</p>

      <div class="player-table-wrap">
        <div class="player-table">
          <div class="table-head">
            <button class="sort-button" :class="{active:sort.key === 'name'}" @click="changeSort('name')"><span>{{ sortLabel('name') }}</span><ChevronUp v-if="sort.key === 'name' && sort.order === 'asc'"/><ChevronDown v-else-if="sort.key === 'name'"/></button>
            <button class="sort-button" :class="{active:sort.key === 'status'}" @click="changeSort('status')"><span>{{ sortLabel('status') }}</span><ChevronUp v-if="sort.key === 'status' && sort.order === 'asc'"/><ChevronDown v-else-if="sort.key === 'status'"/></button>
            <button class="sort-button" :class="{active:sort.key === 'level'}" @click="changeSort('level')"><span>{{ sortLabel('level') }}</span><ChevronUp v-if="sort.key === 'level' && sort.order === 'asc'"/><ChevronDown v-else-if="sort.key === 'level'"/></button>
            <span>坐标</span>
            <button class="sort-button" :class="{active:sort.key === 'updated_at'}" @click="changeSort('updated_at')"><span>{{ sortLabel('updated_at') }}</span><ChevronUp v-if="sort.key === 'updated_at' && sort.order === 'asc'"/><ChevronDown v-else-if="sort.key === 'updated_at'"/></button>
            <span>操作</span>
          </div>
          <div v-if="playerLoading && !players.length" class="table-state"><LoaderCircle class="spin"/><span>正在读取玩家记录</span></div>
          <div v-else-if="!playerError && !players.length" class="table-state"><Users/><span>{{ query.trim() ? '没有匹配的玩家' : '数据源中尚无玩家记录' }}</span></div>
          <article v-for="player in players" :key="player.id" class="player-row" @dblclick="openProfile(player)">
            <button class="player-identity" :title="`查看 ${player.display_name || player.name}`" @click="openProfile(player)">
              <i>{{ (player.display_name || player.name).slice(0, 1) }}</i>
              <span><b>{{ player.display_name || player.name }}</b><small>{{ player.role || '未设置身份' }}<template v-if="player.uuid"> · {{ player.uuid }}</template></small></span>
            </button>
            <span class="player-status" :class="statusClass(player.status)">{{ statusLabel(player.status) }}</span>
            <span class="level-value">{{ player.level ?? '-' }}</span>
            <span class="position-value"><MapPin/><em>{{ formatPosition(player.position) }}</em></span>
            <time>{{ formatUpdatedAt(player.updated_at) }}</time>
            <button class="row-action" title="查看玩家详情" @click="openProfile(player)"><Pencil/><span>详情</span></button>
          </article>
        </div>
      </div>
    </section>

    <section v-if="showPoll" class="poll-creator">
      <header><div><small>NEW POLL</small><b>创建玩法投票</b></div><button class="icon-button" title="关闭" @click="showPoll=false"><X/></button></header>
      <input v-model="pollTitle" placeholder="投票标题"/>
      <div class="option-inputs"><input v-for="(_, index) in pollOptions" :key="index" v-model="pollOptions[index]" :placeholder="`选项 ${index + 1}`"/></div>
      <footer><button @click="pollOptions.push('')"><Plus/>添加选项</button><button class="primary" @click="createPoll"><Vote/>发布投票</button></footer>
    </section>

    <div class="content-grid">
      <section class="feedback-card">
        <header><div><small>意见收集</small><b>玩家反馈</b></div><button @click="clusterFeedback"><BarChart3/>AI 聚类</button></header>
        <p v-if="communityError" class="inline-error"><AlertTriangle/>{{ communityError }}</p>
        <div v-if="!serverFeedback.length && !communityError" class="community-empty"><MessageSquareText/><span>当前服务器暂无反馈</span></div>
        <article v-for="item in serverFeedback" :key="item.id"><span :class="item.sentiment"></span><p><b>{{ item.player }} · {{ item.category }}</b><small>{{ item.content }}</small></p></article>
        <div v-if="cluster" class="cluster-result"><b>{{ cluster.summary }}</b><span><i v-for="(count, name) in cluster.categories" :key="name">{{ name }} {{ count }}</i></span></div>
      </section>
      <aside>
        <section class="papi-status-card" :class="{available:papiAvailable}">
          <header><div><small>PLACEHOLDERAPI</small><b>变量显示</b></div><button class="icon-button" title="管理变量" @click="openPapiManager"><PlugZap/></button></header>
          <p><i/><span>{{ papiDetail }}</span></p>
          <footer><span>{{ enabledPapiFieldCount }} 个字段已启用</span><button @click="openPapiManager"><Pencil/>管理字段</button></footer>
        </section>
        <section class="poll-card">
          <header><div><small>进行中</small><b>玩法投票</b></div><button class="icon-button" title="发起投票" @click="showPoll=true"><Plus/></button></header>
          <div v-if="!serverPolls.length" class="community-empty"><Vote/><span>当前服务器暂无投票</span></div>
          <article v-for="poll in serverPolls" :key="poll.id"><b>{{ poll.title }}</b><button v-for="option in poll.options" :key="option.id" @click="voteFor(poll, option)"><span>{{ option.label }}</span><em>{{ option.votes }} 票</em></button></article>
        </section>
      </aside>
    </div>

    <div v-if="showProfile" class="community-modal-backdrop" @click.self="closeProfile">
      <section class="player-modal" aria-modal="true" role="dialog" :aria-label="selectedPlayer ? `${selectedPlayer.display_name || selectedPlayer.name} 的玩家详情` : '玩家详情'">
        <header class="modal-header">
          <div>
            <small>PLAYER PROFILE</small>
            <h2>{{ profile?.display_name || profile?.name || selectedPlayer?.display_name || selectedPlayer?.name || '玩家详情' }}</h2>
            <p><span class="player-status" :class="statusClass(profile?.status || selectedPlayer?.status || '')">{{ statusLabel(profile?.status || selectedPlayer?.status || '') }}</span><span>{{ profile?.uuid || selectedPlayer?.uuid || 'UUID 未记录' }}</span></p>
          </div>
          <button class="icon-button" title="关闭" @click="closeProfile"><X/></button>
        </header>

        <main class="profile-body">
          <div v-if="profileLoading" class="profile-state"><LoaderCircle class="spin"/><span>正在读取玩家资料、背包与末影箱</span></div>
          <div v-else-if="profileError && !profile" class="profile-state error"><AlertTriangle/><span>{{ profileError }}</span><button @click="selectedPlayer && openProfile(selectedPlayer)"><RefreshCw/>重试</button></div>
          <template v-else-if="profile">
            <section class="profile-overview">
              <header><div><small>基本资料</small><b>玩家信息</b></div><div class="profile-actions"><button v-if="!profileEditMode" class="icon-button" title="编辑玩家资料" @click="profileEditMode=true"><Pencil/></button><template v-else><button class="text-button" :disabled="profileSaving" @click="cancelProfileEdit">取消</button><button class="save-button" :disabled="profileSaving" @click="saveProfile"><LoaderCircle v-if="profileSaving" class="spin"/><Save v-else/>保存</button></template></div></header>
              <div v-if="profileEditMode" class="profile-edit-form">
                <label>显示名称<input v-model="profileDraft.display_name" maxlength="64" placeholder="留空使用游戏名"/></label>
                <label>身份<input v-model="profileDraft.role" maxlength="64" placeholder="例如：会员、管理员"/></label>
                <label class="wide">标签<input v-model="profileDraft.tags" maxlength="300" placeholder="用逗号分隔，例如：建筑, 常驻"/></label>
                <label class="wide">备注<textarea v-model="profileDraft.note" rows="3" maxlength="500" placeholder="仅用于管理备注"/></label>
              </div>
              <div v-else class="profile-facts">
                <p><small>游戏名</small><b>{{ profile.name }}</b></p>
                <p><small>身份</small><b>{{ profile.role || '未设置' }}</b></p>
                <p><small>等级</small><b>{{ profile.level ?? '未记录' }}</b></p>
                <p><small>最近更新</small><b>{{ formatUpdatedAt(profile.updated_at) }}</b></p>
                <p class="wide"><small>坐标</small><b><MapPin/>{{ formatPosition(profile.position) }}</b></p>
                <p v-if="profile.tags.length" class="wide tags"><small>标签</small><span><i v-for="tag in profile.tags" :key="tag">{{ tag }}</i></span></p>
                <p v-if="profile.note" class="wide note"><small>备注</small><b>{{ profile.note }}</b></p>
              </div>
              <p v-if="profileError" class="inline-error"><AlertTriangle/>{{ profileError }}</p>
            </section>

            <section class="inventory-section">
              <header><div><small>INVENTORY</small><b>背包</b><em>{{ inventoryCells.filter((cell) => cell.item).length }} / {{ inventoryCells.length }} 个格位有物品</em></div></header>
              <div class="inventory-layout" :class="{'has-preview':!!activeContainerPreview}">
                <div class="inventory-grid" aria-label="背包物品格">
                  <button v-for="cell in inventoryCells" :key="cell.slot" class="inventory-slot" :class="[cell.item ? itemClass(cell.item) : 'empty', {active:activeContainerPreview === cell.item}]" :disabled="!cell.item" :title="cell.item ? itemTooltip(cell.item) : `格位 ${cell.slot + 1}`" @mouseenter="previewContainer(cell.item)" @mouseleave="clearContainerPreview(cell.item)" @focus="previewContainer(cell.item)" @blur="clearContainerPreview(cell.item)" @click="toggleContainerPreview(cell.item)">
                    <Box v-if="cell.item"/><span v-if="cell.item" class="item-name">{{ itemShortName(cell.item) }}</span><i v-if="cell.item && cell.item.count > 1">{{ cell.item.count }}</i>
                  </button>
                </div>
                <aside v-if="activeContainerPreview" class="container-preview" @mouseenter="previewContainer(activeContainerPreview)">
                  <header><div><small>{{ containerKindLabel(activeContainerPreview) }}</small><b>{{ itemName(activeContainerPreview) }}</b></div><button class="icon-button" title="关闭预览" @click="pinnedContainer=null;hoveredContainer=null"><X/></button></header>
                  <p>{{ activeContainerPreview.contents?.length || 0 }} 个已读取物品</p>
                  <div class="nested-grid">
                    <button v-for="cell in containerCells(activeContainerPreview)" :key="cell.slot" class="nested-slot" :class="cell.item ? itemClass(cell.item) : 'empty'" :disabled="!cell.item" :title="cell.item ? itemTooltip(cell.item) : `格位 ${cell.slot + 1}`" @mouseenter="previewContainer(cell.item)" @focus="previewContainer(cell.item)" @click="toggleContainerPreview(cell.item)"><Box v-if="cell.item"/><i v-if="cell.item && cell.item.count > 1">{{ cell.item.count }}</i></button>
                  </div>
                </aside>
              </div>
            </section>

            <section class="inventory-section">
              <header><div><small>ENDER CHEST</small><b>末影箱</b><em>{{ enderChestCells.filter((cell) => cell.item).length }} / {{ enderChestCells.length }} 个格位有物品</em></div></header>
              <div class="inventory-layout" :class="{'has-preview':!!activeContainerPreview}">
                <div class="inventory-grid ender-grid" aria-label="末影箱物品格">
                  <button v-for="cell in enderChestCells" :key="cell.slot" class="inventory-slot" :class="[cell.item ? itemClass(cell.item) : 'empty', {active:activeContainerPreview === cell.item}]" :disabled="!cell.item" :title="cell.item ? itemTooltip(cell.item) : `格位 ${cell.slot + 1}`" @mouseenter="previewContainer(cell.item)" @mouseleave="clearContainerPreview(cell.item)" @focus="previewContainer(cell.item)" @blur="clearContainerPreview(cell.item)" @click="toggleContainerPreview(cell.item)">
                    <Box v-if="cell.item"/><span v-if="cell.item" class="item-name">{{ itemShortName(cell.item) }}</span><i v-if="cell.item && cell.item.count > 1">{{ cell.item.count }}</i>
                  </button>
                </div>
                <aside v-if="activeContainerPreview" class="container-preview" @mouseenter="previewContainer(activeContainerPreview)">
                  <header><div><small>{{ containerKindLabel(activeContainerPreview) }}</small><b>{{ itemName(activeContainerPreview) }}</b></div><button class="icon-button" title="关闭预览" @click="pinnedContainer=null;hoveredContainer=null"><X/></button></header>
                  <p>{{ activeContainerPreview.contents?.length || 0 }} 个已读取物品</p>
                  <div class="nested-grid">
                    <button v-for="cell in containerCells(activeContainerPreview)" :key="cell.slot" class="nested-slot" :class="cell.item ? itemClass(cell.item) : 'empty'" :disabled="!cell.item" :title="cell.item ? itemTooltip(cell.item) : `格位 ${cell.slot + 1}`" @mouseenter="previewContainer(cell.item)" @focus="previewContainer(cell.item)" @click="toggleContainerPreview(cell.item)"><Box v-if="cell.item"/><i v-if="cell.item && cell.item.count > 1">{{ cell.item.count }}</i></button>
                  </div>
                </aside>
              </div>
            </section>

            <section class="papi-values-section">
              <header><div><small>PLACEHOLDERAPI</small><b>指定变量</b><em :class="{available:papiAvailable}">{{ papiDetail }}</em></div><span><button class="icon-button" title="刷新变量" :disabled="playerPapiLoading" @click="loadPlayerPapi(profile.id)"><RefreshCw :class="{spin:playerPapiLoading}"/></button><button class="icon-button" title="管理变量字段" @click="openPapiManager"><PlugZap/></button></span></header>
              <div v-if="playerPapiLoading" class="papi-state"><LoaderCircle class="spin"/>正在解析指定变量</div>
              <p v-else-if="playerPapiError" class="inline-error"><AlertTriangle/>{{ playerPapiError }}</p>
              <div v-else-if="!playerPapiValues.length" class="papi-state"><PlugZap/>尚未添加可显示的变量字段</div>
              <div v-else class="papi-values"><article v-for="value in playerPapiValues" :key="value.field_id" :class="value.status"><p><small>{{ value.label }}</small><code>{{ value.placeholder }}</code></p><b>{{ value.value || papiStatusLabel(value.status) }}</b><em>{{ papiStatusLabel(value.status) }}</em></article></div>
            </section>
          </template>
        </main>
      </section>
    </div>

    <div v-if="showPapiManager" class="community-modal-backdrop manager-backdrop" @click.self="closePapiManager">
      <section class="papi-manager-modal" aria-modal="true" role="dialog" aria-label="管理 PlaceholderAPI 显示变量">
        <header class="modal-header"><div><small>PLACEHOLDERAPI FIELDS</small><h2>显示变量</h2><p><span :class="{available:papiAvailable}">{{ papiDetail }}</span></p></div><span class="modal-header-actions"><button class="icon-button" title="刷新字段" :disabled="papiLoading || papiSaving" @click="loadPapiFields"><RefreshCw :class="{spin:papiLoading}"/></button><button class="icon-button" title="关闭" :disabled="papiSaving" @click="closePapiManager"><X/></button></span></header>
        <main>
          <p class="manager-hint">添加后会在打开玩家详情时解析并显示对应变量。变量不可用时会保留明确状态。</p>
          <p v-if="papiError" class="inline-error"><AlertTriangle/>{{ papiError }}</p>
          <div class="papi-field-list">
            <article v-for="(field, index) in papiDraft" :key="field.id">
              <label class="field-enabled"><input v-model="field.enabled" type="checkbox"/><span>{{ field.enabled ? '显示' : '隐藏' }}</span></label>
              <label>名称<input v-model="field.label" maxlength="64" :placeholder="`变量 ${index + 1}`"/></label>
              <label>占位符<input v-model="field.placeholder" maxlength="256" placeholder="例如 %player_level%"/></label>
              <button class="icon-button delete-field" title="删除变量" :disabled="papiSaving" @click="removePapiField(field.id)"><Trash2/></button>
            </article>
          </div>
          <button class="add-field" :disabled="papiSaving" @click="addPapiField"><Plus/>添加变量</button>
          <p v-if="papiSaveError" class="inline-error"><AlertTriangle/>{{ papiSaveError }}</p>
        </main>
        <footer><button class="text-button" :disabled="papiSaving" @click="closePapiManager">取消</button><button class="save-button" :disabled="papiSaving" @click="savePapiFields"><LoaderCircle v-if="papiSaving" class="spin"/><Save v-else/>保存字段</button></footer>
      </section>
    </div>
  </div>
</template>

<style scoped>
.community-scroll{position:relative;flex:1;overflow:auto;padding:18px;color:#e8edf2}.community-hero,.player-management-card,.feedback-card,.poll-card,.poll-creator,.papi-status-card{border:1px solid rgba(255,255,255,.075);border-radius:8px;background:#11161c}.community-hero{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:18px;background:linear-gradient(120deg,rgba(156,140,255,.07),rgba(50,213,176,.035) 52%,#11161c)}.community-hero>div:first-child{display:flex;align-items:center;gap:12px}.community-hero>div:first-child>span{width:43px;height:43px;display:grid;place-items:center;border-radius:8px;color:#a89cff;background:rgba(156,140,255,.1)}.community-hero svg{width:20px}.community-hero p{display:flex;flex-direction:column;margin:0}.community-hero small,.player-management-card header small,.feedback-card header small,.poll-card header small,.poll-creator header small,.papi-status-card header small,.modal-header small,.profile-overview small,.inventory-section small,.papi-values-section small{color:#66727f;font-size:8px;text-transform:uppercase}.community-hero b{margin-top:4px;font-size:15px}.community-hero em{margin-top:5px;color:#6c7885;font:normal 8px Inter}.hero-actions{display:flex;align-items:center;gap:7px}.icon-button{width:30px;height:30px;display:grid;place-items:center;padding:0;border:1px solid rgba(255,255,255,.08);border-radius:6px;color:#85919d;background:#161d24}.icon-button:hover:not(:disabled){color:#dce4ea;border-color:rgba(50,213,176,.22);background:rgba(50,213,176,.07)}.icon-button:disabled{opacity:.45;cursor:not-allowed}.icon-button svg{width:14px}.papi-button,.save-button,.text-button,.row-action,.add-field,.papi-status-card footer button{height:31px;display:flex;align-items:center;justify-content:center;gap:6px;padding:0 10px;border:1px solid rgba(156,140,255,.22);border-radius:6px;color:#c0b7ff;background:rgba(156,140,255,.08);font-size:9px}.papi-button svg,.save-button svg,.row-action svg,.add-field svg,.papi-status-card footer button svg{width:13px}.player-stats{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:9px;margin-top:10px}.player-stats article{display:flex;align-items:center;gap:10px;padding:13px;border:1px solid rgba(255,255,255,.075);border-radius:8px;background:#12171d}.player-stats article>span{width:32px;height:32px;display:grid;place-items:center;flex:none;border-radius:7px}.player-stats svg{width:16px}.mint{color:#32d5b0;background:rgba(50,213,176,.09)}.green{color:#82d98d;background:rgba(83,190,100,.1)}.violet{color:#a99dff;background:rgba(156,140,255,.1)}.amber{color:#f3a75c;background:rgba(243,167,92,.09)}.player-stats p{display:flex;min-width:0;flex-direction:column;margin:0}.player-stats small{color:#697582;font-size:8px}.player-stats b{margin-top:2px;font-size:14px}.player-stats em{overflow:hidden;margin-top:2px;color:#56616e;font:normal 7px Inter;text-overflow:ellipsis;white-space:nowrap}.player-management-card{margin-top:10px;padding:15px}.player-card-header,.feedback-card>header,.poll-card>header,.poll-creator>header,.papi-status-card>header{display:flex;align-items:center;justify-content:space-between;gap:12px}.player-card-header>div:first-child,.feedback-card header>div,.poll-card header>div,.poll-creator header>div,.papi-status-card header>div{display:flex;min-width:0;flex-direction:column}.player-card-header b,.feedback-card header b,.poll-card header b,.poll-creator header b,.papi-status-card header b{margin-top:4px;font-size:12px}.source-status{display:flex;align-items:center;gap:4px;margin-top:5px;color:#d3a36e;font:normal 8px Inter}.source-status.available{color:#70ceb7}.source-status svg{width:11px}.player-tools{display:flex;align-items:center;gap:6px}.player-search{height:30px;display:flex;align-items:center;gap:6px;padding:0 9px;border:1px solid rgba(255,255,255,.08);border-radius:6px;color:#65717e;background:#0e1318}.player-search svg{width:13px}.player-search input{width:180px;min-width:0;border:0;outline:0;color:#d1d9e0;background:transparent;font-size:9px}.source-detail{margin:9px 0 0;color:#6d7985;font-size:8px;line-height:1.5}.inline-error{display:flex;align-items:flex-start;gap:7px;margin:10px 0 0;padding:9px;border-radius:6px;color:#ec989e;background:rgba(255,107,114,.065);font-size:8px;line-height:1.55}.inline-error svg{width:13px;flex:none}.player-table-wrap{min-width:0;margin-top:12px;overflow-x:auto}.player-table{min-width:720px}.table-head,.player-row{display:grid;grid-template-columns:minmax(180px,1.55fr) .75fr .55fr minmax(155px,1.25fr) minmax(114px,.95fr) 64px;align-items:center;gap:8px}.table-head{min-height:30px;padding:0 8px;color:#687480;font-size:7px;text-transform:uppercase}.table-head>span{padding-left:6px}.sort-button{height:26px;display:flex;align-items:center;gap:2px;padding:0 6px;border:0;border-radius:5px;color:inherit;background:transparent;font-size:7px;text-align:left;text-transform:uppercase}.sort-button:hover,.sort-button.active{color:#c9d2da;background:rgba(255,255,255,.045)}.sort-button svg{width:11px}.player-row{min-height:54px;padding:7px 8px;border-top:1px solid rgba(255,255,255,.055);color:#7c8794;font-size:8px}.player-row:hover{background:rgba(50,213,176,.025)}.player-identity{display:flex;align-items:center;min-width:0;gap:8px;padding:0;border:0;color:inherit;background:transparent;text-align:left}.player-identity i{width:28px;height:28px;display:grid;place-items:center;flex:none;border-radius:6px;color:#8ee2cf;background:rgba(50,213,176,.09);font:normal 10px Inter}.player-identity span{display:flex;min-width:0;flex-direction:column}.player-identity b{overflow:hidden;color:#c9d0d8;font-size:9px;text-overflow:ellipsis;white-space:nowrap}.player-identity small{overflow:hidden;margin-top:4px;color:#66727f;font-size:7px;text-overflow:ellipsis;white-space:nowrap}.player-status{width:max-content;padding:3px 6px;border-radius:5px;color:#697683;background:rgba(255,255,255,.05);font:normal 7px Inter}.player-status.online{color:#67d2b6;background:rgba(50,213,176,.08)}.player-status.banned{color:#ff9096;background:rgba(255,107,114,.08)}.level-value{color:#c7d0d8;font:600 10px Inter}.position-value{display:flex;align-items:center;min-width:0;gap:5px}.position-value svg{width:12px;color:#aa9eff}.position-value em{overflow:hidden;color:#8995a0;font:normal 8px Inter;text-overflow:ellipsis;white-space:nowrap}.player-row time{overflow:hidden;color:#65717d;font:7px Inter;text-overflow:ellipsis;white-space:nowrap}.row-action{height:27px;padding:0 7px;border-color:rgba(255,255,255,.08);color:#9ba7b3;background:#171d24;font-size:7px}.table-state{min-height:130px;display:flex;align-items:center;justify-content:center;gap:8px;color:#687480;font-size:9px}.table-state svg{width:18px}.content-grid{display:grid;grid-template-columns:minmax(0,1.5fr) minmax(250px,.82fr);gap:10px;margin-top:10px}.feedback-card,.poll-card,.poll-creator,.papi-status-card{padding:15px}.content-grid>aside{display:grid;align-content:start;gap:10px}.feedback-card header>button{display:flex;align-items:center;gap:4px;border:0;color:#a99dff;background:transparent;font-size:8px}.feedback-card header svg{width:13px}.feedback-card>article{display:flex;gap:8px;padding:9px 0;border-top:1px solid rgba(255,255,255,.055)}.feedback-card>article>span{width:6px;height:6px;flex:none;margin-top:3px;border-radius:50%;background:#818c98}.feedback-card>article>span.positive{background:#32d5b0}.feedback-card>article>span.negative{background:#ff747b}.feedback-card>article p{display:flex;flex-direction:column;margin:0}.feedback-card>article b{font-size:8px}.feedback-card>article small{margin-top:4px;color:#687480;font-size:7px;line-height:1.5}.community-empty{display:flex;align-items:center;justify-content:center;gap:7px;min-height:82px;color:#65717d;font-size:8px;text-align:center}.community-empty svg{width:15px}.cluster-result{padding:9px;margin-top:7px;border-radius:6px;color:#9bcfc3;background:rgba(50,213,176,.05);font-size:8px;line-height:1.5}.cluster-result span{display:flex;flex-wrap:wrap;gap:4px;margin-top:6px}.cluster-result i{padding:2px 4px;border-radius:4px;background:rgba(50,213,176,.07);font-style:normal}.poll-card>article{padding-top:10px;margin-top:10px;border-top:1px solid rgba(255,255,255,.055)}.poll-card>article>b{display:block;margin-bottom:8px;font-size:9px}.poll-card>article>button{width:100%;display:flex;justify-content:space-between;padding:7px 8px;margin-top:5px;border:1px solid rgba(255,255,255,.065);border-radius:6px;color:#87929f;background:#0e1318;font-size:7px;text-align:left}.poll-card>article>button:hover{border-color:rgba(156,140,255,.2)}.poll-card em{color:#a69bf4;font-style:normal}.papi-status-card{border-color:rgba(156,140,255,.14);background:rgba(156,140,255,.035)}.papi-status-card.available{border-color:rgba(50,213,176,.17);background:rgba(50,213,176,.035)}.papi-status-card>p{display:flex;align-items:flex-start;gap:6px;margin:12px 0 0;color:#77838f;font-size:8px;line-height:1.55}.papi-status-card>p i{width:6px;height:6px;flex:none;margin-top:3px;border-radius:50%;background:#d4a76e}.papi-status-card.available>p i{background:#32d5b0}.papi-status-card footer{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-top:12px}.papi-status-card footer>span{color:#66727f;font-size:7px}.papi-status-card footer button{height:26px;padding:0 7px;border-color:rgba(156,140,255,.18);font-size:7px}.poll-creator{margin-top:10px}.poll-creator>input,.option-inputs input{width:100%;height:31px;padding:0 9px;border:1px solid rgba(255,255,255,.08);border-radius:6px;outline:0;color:#cbd2da;background:#0e1318;font-size:8px}.option-inputs{display:grid;grid-template-columns:repeat(2,1fr);gap:7px;margin-top:7px}.poll-creator footer{display:flex;justify-content:flex-end;gap:6px;margin-top:9px}.poll-creator footer button{height:27px;display:flex;align-items:center;gap:4px;padding:0 8px;border:1px solid rgba(255,255,255,.08);border-radius:6px;color:#85909d;background:#171d24;font-size:7px}.poll-creator footer button.primary{border:0;color:#161121;background:#a89cff}.poll-creator footer svg{width:11px}.community-modal-backdrop{position:fixed;inset:0;z-index:70;display:grid;place-items:center;padding:20px;background:rgba(2,5,8,.72);backdrop-filter:blur(10px)}.player-modal,.papi-manager-modal{display:flex;flex-direction:column;width:min(1100px,calc(100vw - 40px));max-height:calc(100vh - 40px);overflow:hidden;border:1px solid rgba(255,255,255,.12);border-radius:8px;background:#12171d;box-shadow:0 30px 100px rgba(0,0,0,.55)}.papi-manager-modal{width:min(760px,calc(100vw - 40px))}.modal-header{display:flex;align-items:flex-start;justify-content:space-between;gap:14px;padding:18px 20px;border-bottom:1px solid rgba(255,255,255,.07);background:#10151b}.modal-header h2{margin:5px 0 0;font-size:16px}.modal-header p{display:flex;gap:7px;margin:7px 0 0;color:#697582;font-size:8px}.modal-header p>span.available{color:#70ceb7}.profile-body{min-height:0;overflow:auto;padding:18px 20px 24px}.profile-state{min-height:240px;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:9px;color:#6c7884;font-size:9px;text-align:center}.profile-state svg{width:24px}.profile-state.error{color:#e79a9e}.profile-state button{height:28px;display:flex;align-items:center;gap:5px;margin-top:4px;padding:0 9px;border:1px solid rgba(255,107,114,.2);border-radius:6px;color:#ef9da2;background:rgba(255,107,114,.06);font-size:8px}.profile-state button svg{width:12px}.profile-overview,.inventory-section,.papi-values-section{margin-top:12px;padding-top:12px;border-top:1px solid rgba(255,255,255,.065)}.profile-overview{margin-top:0;padding-top:0;border-top:0}.profile-overview>header,.inventory-section>header,.papi-values-section>header{display:flex;align-items:center;justify-content:space-between;gap:10px}.profile-overview header>div:first-child,.inventory-section header>div,.papi-values-section header>div{display:flex;min-width:0;flex-direction:column}.profile-overview header b,.inventory-section header b,.papi-values-section header b{margin-top:4px;font-size:12px}.inventory-section header em,.papi-values-section header em{margin-top:4px;color:#697582;font:normal 8px Inter;text-transform:none}.papi-values-section header em.available{color:#70ceb7}.profile-actions,.papi-values-section>header>span{display:flex;align-items:center;gap:6px}.text-button{border-color:rgba(255,255,255,.08);color:#8995a1;background:#171d24}.save-button{border:0;color:#06251e;background:#32d5b0;font-weight:700}.save-button:disabled,.text-button:disabled{opacity:.5;cursor:not-allowed}.profile-facts{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-top:12px}.profile-facts>p{display:flex;min-width:0;flex-direction:column;gap:5px;margin:0;padding:9px;border-radius:6px;background:#0e1318}.profile-facts small{color:#64717d;font-size:7px}.profile-facts b{overflow:hidden;color:#c7d0d8;font-size:9px;text-overflow:ellipsis;white-space:nowrap}.profile-facts p.wide{grid-column:span 2}.profile-facts p.wide b{display:flex;align-items:center;gap:5px;overflow:visible;white-space:normal}.profile-facts p.wide b svg{width:12px;color:#a99dff}.profile-facts .tags span{display:flex;flex-wrap:wrap;gap:4px}.profile-facts .tags i{padding:3px 5px;border-radius:4px;color:#9ce1cf;background:rgba(50,213,176,.08);font:normal 7px Inter}.profile-facts .note b{color:#99a6b2;font-weight:400;line-height:1.5}.profile-edit-form{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:9px;margin-top:12px}.profile-edit-form label{display:flex;flex-direction:column;gap:5px;color:#7c8995;font-size:8px}.profile-edit-form label.wide{grid-column:1/-1}.profile-edit-form input,.profile-edit-form textarea,.papi-field-list input{width:100%;border:1px solid rgba(255,255,255,.08);border-radius:6px;outline:0;color:#d6dde4;background:#0d1217;font-size:9px}.profile-edit-form input{height:32px;padding:0 9px}.profile-edit-form textarea{padding:8px 9px;resize:vertical;line-height:1.5}.profile-edit-form input:focus,.profile-edit-form textarea:focus,.papi-field-list input:focus{border-color:rgba(50,213,176,.34)}.inventory-layout{display:grid;grid-template-columns:minmax(0,1fr) minmax(210px,.38fr);gap:12px;margin-top:12px}.inventory-layout:has(.container-preview:only-child){grid-template-columns:1fr}.inventory-grid,.nested-grid{display:grid;grid-template-columns:repeat(9,minmax(0,1fr));gap:4px}.inventory-slot,.nested-slot{position:relative;aspect-ratio:1;min-width:0;display:flex;align-items:center;justify-content:center;padding:2px;border:1px solid rgba(255,255,255,.09);border-radius:4px;color:#93a1ad;background:#10161d}.inventory-slot:disabled,.nested-slot:disabled{cursor:default}.inventory-slot:not(:disabled):hover,.inventory-slot:not(:disabled):focus,.inventory-slot.active{border-color:rgba(50,213,176,.55);outline:0;background:rgba(50,213,176,.08)}.inventory-slot.shulker,.nested-slot.shulker{color:#bc8ee9;border-color:rgba(185,128,231,.35);background:rgba(185,128,231,.08)}.inventory-slot.bundle,.nested-slot.bundle{color:#ebbc7e;border-color:rgba(234,184,114,.34);background:rgba(234,184,114,.08)}.inventory-slot.container,.nested-slot.container{color:#82b4e9;border-color:rgba(109,164,225,.34);background:rgba(109,164,225,.08)}.inventory-slot svg,.nested-slot svg{width:15px}.inventory-slot .item-name{position:absolute;right:3px;bottom:2px;left:3px;overflow:hidden;color:#d4dce2;font:6px Inter;text-align:center;text-overflow:ellipsis;white-space:nowrap}.inventory-slot>i,.nested-slot>i{position:absolute;right:2px;bottom:1px;color:#fff;font:700 8px Inter;text-shadow:0 1px 2px #000}.container-preview{min-width:0;padding:11px;border:1px solid rgba(156,140,255,.2);border-radius:7px;background:#0d1217}.container-preview>header{display:flex;align-items:flex-start;justify-content:space-between;gap:8px}.container-preview header>div{display:flex;min-width:0;flex-direction:column}.container-preview small{color:#9f93e8;font-size:7px;text-transform:uppercase}.container-preview b{overflow:hidden;margin-top:4px;color:#d5d9e9;font-size:9px;text-overflow:ellipsis;white-space:nowrap}.container-preview>p{margin:8px 0;color:#697582;font-size:7px}.nested-slot svg{width:11px}.papi-state{display:flex;align-items:center;justify-content:center;gap:7px;min-height:70px;margin-top:10px;color:#6c7885;font-size:8px}.papi-state svg{width:15px}.papi-values{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:7px;margin-top:10px}.papi-values article{display:grid;grid-template-columns:minmax(0,1fr) auto;align-items:center;gap:6px;padding:9px;border:1px solid rgba(50,213,176,.12);border-radius:6px;background:#0e1419}.papi-values article.unresolved{border-color:rgba(243,167,92,.18)}.papi-values article.unavailable{border-color:rgba(255,107,114,.16)}.papi-values p{display:flex;min-width:0;flex-direction:column;margin:0}.papi-values small{color:#82909c;font-size:7px;text-transform:none}.papi-values code{overflow:hidden;margin-top:3px;color:#7f73cc;font:7px 'Cascadia Code',monospace;text-overflow:ellipsis;white-space:nowrap}.papi-values b{overflow:hidden;color:#c7d9d5;font-size:9px;text-align:right;text-overflow:ellipsis;white-space:nowrap}.papi-values em{grid-column:2;color:#68cdb3;font:normal 7px Inter}.papi-values article.unresolved em{color:#e5aa70}.papi-values article.unavailable em{color:#e78f95}.manager-backdrop{z-index:80}.papi-manager-modal>main{min-height:0;overflow:auto;padding:18px 20px}.manager-hint{margin:0;color:#71808c;font-size:9px;line-height:1.6}.papi-field-list{display:grid;gap:7px;margin-top:14px}.papi-field-list article{display:grid;grid-template-columns:70px minmax(110px,.65fr) minmax(180px,1.3fr) 30px;align-items:end;gap:7px;padding:9px;border:1px solid rgba(255,255,255,.07);border-radius:6px;background:#0e1318}.papi-field-list label{display:flex;flex-direction:column;gap:4px;color:#75818d;font-size:7px}.papi-field-list label.field-enabled{height:31px;align-items:center;justify-content:flex-start;flex-direction:row;gap:5px;color:#8996a1}.papi-field-list input{height:31px;padding:0 8px}.papi-field-list .field-enabled input{width:auto;height:auto;accent-color:#32d5b0}.delete-field{height:31px;color:#e78c92}.add-field{height:30px;margin-top:10px;border-color:rgba(50,213,176,.18);color:#8fe2cd;background:rgba(50,213,176,.06)}.papi-manager-modal>footer{height:58px;display:flex;align-items:center;justify-content:flex-end;gap:7px;padding:0 20px;border-top:1px solid rgba(255,255,255,.07);background:#10151b}.spin{animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@media(max-width:1050px){.player-stats{grid-template-columns:repeat(2,minmax(0,1fr))}.content-grid{grid-template-columns:1fr}.content-grid>aside{grid-template-columns:repeat(2,minmax(0,1fr))}.inventory-layout{grid-template-columns:1fr}.container-preview{max-width:420px}.papi-values{grid-template-columns:1fr}}@media(max-width:700px){.community-scroll{padding:12px}.community-hero{align-items:flex-start;flex-direction:column}.hero-actions{width:100%;justify-content:flex-end}.player-card-header{align-items:flex-start;flex-direction:column}.player-tools{width:100%}.player-search{flex:1}.player-search input{width:100%}.content-grid>aside{grid-template-columns:1fr}.option-inputs,.profile-edit-form,.profile-facts{grid-template-columns:1fr}.profile-facts p.wide{grid-column:auto}.community-modal-backdrop{padding:8px}.player-modal,.papi-manager-modal{width:100%;max-height:calc(100vh - 16px)}.modal-header,.profile-body,.papi-manager-modal>main{padding-right:14px;padding-left:14px}.papi-field-list article{grid-template-columns:1fr 30px}.papi-field-list label.field-enabled{grid-column:1}.papi-field-list label:not(.field-enabled){grid-column:1}.papi-field-list .delete-field{grid-column:2;grid-row:1;align-self:center}.inventory-grid{gap:3px}.inventory-slot .item-name{display:none}}
.inventory-layout{grid-template-columns:1fr}.inventory-layout.has-preview{grid-template-columns:minmax(0,1fr) minmax(210px,.38fr)}.modal-header-actions{display:flex;align-items:center;gap:6px}@media(max-width:1050px){.inventory-layout.has-preview{grid-template-columns:1fr}}
</style>
