<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  Archive,
  BookOpen,
  Box,
  Check,
  ChevronRight,
  CircleAlert,
  Clipboard,
  CloudDownload,
  Code2,
  Copy,
  Download,
  ExternalLink,
  FileJson,
  Filter,
  Layers3,
  LoaderCircle,
  Package,
  Pencil,
  Plus,
  Puzzle,
  RefreshCw,
  Search,
  Server,
  ShieldAlert,
  Trash2,
  X,
} from 'lucide-vue-next'
import { API_BASE, apiRequest, apiUrl } from '../../lib/api'
import type {
  CatalogKind,
  CatalogProject,
  CatalogSummary,
  CatalogVersion,
  ResolveResponse,
  VersionStatus,
} from './types'

type Section = 'library' | 'manage' | 'docs'
type ModalMode = 'create' | 'edit'
type CodeLanguage = 'curl' | 'powershell' | 'javascript'
type DocSection = 'quickstart' | 'catalog' | 'resolve' | 'download' | 'errors'

const props = withDefaults(defineProps<{ initialCore?: string; initialMinecraft?: string }>(), {
  initialCore: 'Paper',
  initialMinecraft: '1.21.4',
})

const section = ref<Section>('library')
const kind = ref<CatalogKind>('core')
const projects = ref<CatalogProject[]>([])
const versions = ref<CatalogVersion[]>([])
const summary = ref<CatalogSummary | null>(null)
const selectedSlug = ref('')
const search = ref('')
const minecraftFilter = ref('')
const channelFilter = ref('')
const loadingProjects = ref(false)
const loadingVersions = ref(false)
const saving = ref(false)
const pageError = ref('')
const toast = ref('')
let toastTimer: number | undefined
let searchTimer: number | undefined
let projectRequestSeq = 0
let versionRequestSeq = 0

const selectedProject = computed(() => projects.value.find((item) => item.slug === selectedSlug.value) || null)
const resourceSegment = computed(() => (kind.value === 'core' ? 'cores' : 'plugins'))
const minecraftOptions = computed(() => {
  const values = new Set<string>()
  projects.value.forEach((project) => project.minecraft_versions?.forEach((version) => values.add(version)))
  versions.value.forEach((version) => version.minecraft_versions.forEach((item) => values.add(item)))
  ;['1.21.4', '1.21.1', '1.20.6', '1.20.4'].forEach((item) => values.add(item))
  return [...values].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
})
const visibleVersions = computed(() => versions.value.filter((version) => {
  const minecraftMatches = !minecraftFilter.value || version.minecraft_versions.includes(minecraftFilter.value)
  const channelMatches = !channelFilter.value || version.channel === channelFilter.value
  const statusMatches = section.value === 'manage' || version.status === 'published'
  return minecraftMatches && channelMatches && statusMatches
}))

const segmentFor = (value: CatalogKind) => value === 'core' ? 'cores' : 'plugins'

function queryString(values: Record<string, string>) {
  const query = new URLSearchParams()
  Object.entries(values).forEach(([key, value]) => { if (value) query.set(key, value) })
  const encoded = query.toString()
  return encoded ? `?${encoded}` : ''
}

function notify(message: string) {
  toast.value = message
  if (toastTimer) window.clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => { toast.value = '' }, 2600)
}

function friendlyError(error: unknown) {
  const raw = error instanceof Error ? error.message : String(error)
  try {
    const parsed = JSON.parse(raw)
    return parsed.message || parsed.error || raw
  } catch {
    return raw
  }
}

async function loadSummary() {
  try { summary.value = await apiRequest<CatalogSummary>('/api/catalog/summary') } catch { /* page list handles connectivity */ }
}

async function loadProjects(preferredSlug = selectedSlug.value) {
  const requestSeq = ++projectRequestSeq
  ++versionRequestSeq
  const segment = resourceSegment.value
  loadingProjects.value = true
  pageError.value = ''
  try {
    const query = queryString({ search: search.value.trim(), minecraft: minecraftFilter.value })
    const result = await apiRequest<CatalogProject[]>(`/api/catalog/${segment}${query}`)
    if (requestSeq !== projectRequestSeq || segment !== resourceSegment.value) return
    projects.value = result
    const nextSlug = projects.value.some((item) => item.slug === preferredSlug) ? preferredSlug : projects.value[0]?.slug || ''
    selectedSlug.value = nextSlug
    await loadVersions()
  } catch (error) {
    if (requestSeq !== projectRequestSeq) return
    pageError.value = friendlyError(error)
    projects.value = []
    versions.value = []
  } finally {
    if (requestSeq === projectRequestSeq) loadingProjects.value = false
  }
}

async function loadVersions() {
  const requestSeq = ++versionRequestSeq
  const slug = selectedSlug.value
  const segment = resourceSegment.value
  versions.value = []
  if (!slug) { loadingVersions.value = false; return }
  loadingVersions.value = true
  pageError.value = ''
  try {
    const result = await apiRequest<CatalogVersion[]>(`/api/catalog/${segment}/${encodeURIComponent(slug)}/versions`)
    if (requestSeq !== versionRequestSeq || slug !== selectedSlug.value || segment !== resourceSegment.value) return
    versions.value = result
  } catch (error) {
    if (requestSeq !== versionRequestSeq) return
    versions.value = []
    pageError.value = friendlyError(error)
  } finally {
    if (requestSeq === versionRequestSeq) loadingVersions.value = false
  }
}

async function refreshAll() {
  await Promise.all([loadSummary(), loadProjects()])
  notify(pageError.value ? '目录刷新失败，请检查后端连接' : '目录数据已刷新')
}

async function switchKind(next: CatalogKind) {
  if (kind.value === next) return
  kind.value = next
  selectedSlug.value = ''
  search.value = ''
  minecraftFilter.value = ''
  channelFilter.value = ''
  await loadProjects()
}

async function selectProject(slug: string) {
  if (selectedSlug.value === slug) return
  selectedSlug.value = slug
  channelFilter.value = ''
  await loadVersions()
}

function scheduleSearch() {
  if (searchTimer) window.clearTimeout(searchTimer)
  searchTimer = window.setTimeout(() => loadProjects(''), 260)
}

async function setSection(next: Section) {
  section.value = next
  if (next === 'docs') await loadDocProjects()
}

async function copyText(value: string, success = '已复制到剪贴板') {
  let copied = false
  try {
    if (navigator.clipboard?.writeText) {
      await Promise.race([
        navigator.clipboard.writeText(value),
        new Promise((_, reject) => window.setTimeout(() => reject(new Error('clipboard timeout')), 600)),
      ])
      copied = true
    }
  } catch { /* fall back to the selection-based copy path below */ }
  if (!copied) {
    const input = document.createElement('textarea')
    input.value = value
    input.setAttribute('readonly', '')
    input.style.position = 'fixed'
    input.style.opacity = '0'
    document.body.appendChild(input)
    input.select()
    copied = document.execCommand('copy')
    input.remove()
  }
  notify(copied ? success : '复制失败，请手动选择文本')
}

function formatSize(bytes: number) {
  if (!bytes) return '—'
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
  return `${(bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`
}

function formatCount(value: number) {
  return new Intl.NumberFormat('zh-CN', { notation: value > 9999 ? 'compact' : 'standard', maximumFractionDigits: 1 }).format(value || 0)
}

function formatDate(value: string) {
  if (!value) return '—'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString('zh-CN', { month: 'short', day: 'numeric', year: 'numeric' })
}

function projectDownloadPath(project = selectedProject.value) {
  if (!project) return ''
  const supported = [...(project.minecraft_versions || [])].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
  const minecraft = minecraftFilter.value || (supported.includes(props.initialMinecraft) ? props.initialMinecraft : supported[0]) || props.initialMinecraft
  return `/api/v1/resolve?kind=${kind.value}&project=${encodeURIComponent(project.slug)}&minecraft=${encodeURIComponent(minecraft)}&channel=stable`
}

function versionDownloadPath(version: CatalogVersion) {
  return `/api/v1/download/${kind.value}/${encodeURIComponent(version.project)}/${encodeURIComponent(version.version)}`
}

function downloadVersion(version: CatalogVersion) {
  if (version.status !== 'published') { notify('该版本尚未发布，不能通过公开 API 下载'); return }
  window.open(apiUrl(versionDownloadPath(version)), '_blank', 'noopener,noreferrer')
}

interface ProjectForm {
  slug: string
  name: string
  summary: string
  description: string
  author: string
  homepage: string
  repository: string
  tags: string
  color: string
  featured: boolean
}

const emptyProjectForm = (): ProjectForm => ({
  slug: '', name: '', summary: '', description: '', author: '', homepage: '', repository: '', tags: '', color: '#32d5b0', featured: false,
})
const projectModal = ref(false)
const projectModalMode = ref<ModalMode>('create')
const editingProjectKind = ref<CatalogKind>('core')
const editingProjectSlug = ref('')
const projectForm = ref<ProjectForm>(emptyProjectForm())
const projectFormError = ref('')

function openProjectModal(mode: ModalMode) {
  if (searchTimer) { window.clearTimeout(searchTimer); searchTimer = undefined }
  ++projectRequestSeq
  ++versionRequestSeq
  loadingProjects.value = false
  loadingVersions.value = false
  projectModalMode.value = mode
  editingProjectKind.value = kind.value
  editingProjectSlug.value = selectedProject.value?.slug || ''
  projectFormError.value = ''
  const project = selectedProject.value
  projectForm.value = mode === 'edit' && project ? {
    slug: project.slug,
    name: project.name,
    summary: project.summary,
    description: project.description,
    author: project.author,
    homepage: project.homepage,
    repository: project.repository,
    tags: project.tags.join(', '),
    color: project.color || '#32d5b0',
    featured: project.featured,
  } : emptyProjectForm()
  projectModal.value = true
}

async function saveProject() {
  const form = projectForm.value
  if (![form.slug, form.name, form.summary, form.description, form.author, form.homepage, form.repository].every((value) => value.trim())) {
    projectFormError.value = '请填写标识、名称、摘要、详细说明、维护者、项目主页和代码仓库。'
    return
  }
  saving.value = true
  projectFormError.value = ''
  const body = JSON.stringify({
    ...form,
    slug: form.slug.trim().toLowerCase(),
    name: form.name.trim(),
    summary: form.summary.trim(),
    tags: form.tags.split(',').map((item) => item.trim()).filter(Boolean),
  })
  try {
    const segment = segmentFor(editingProjectKind.value)
    const path = projectModalMode.value === 'create'
      ? `/api/catalog/${segment}`
      : `/api/catalog/${segment}/${encodeURIComponent(editingProjectSlug.value)}`
    const saved = await apiRequest<CatalogProject>(path, { method: projectModalMode.value === 'create' ? 'POST' : 'PUT', body })
    projectModal.value = false
    kind.value = editingProjectKind.value
    await Promise.all([loadSummary(), loadProjects(saved.slug)])
    notify(projectModalMode.value === 'create' ? '资源项目已创建' : '项目资料已保存')
  } catch (error) {
    projectFormError.value = friendlyError(error)
  } finally {
    saving.value = false
  }
}

interface VersionForm {
  version: string
  channel: string
  minecraftVersions: string
  loaders: string
  javaVersion: string
  filename: string
  sizeMb: string
  sha256: string
  downloadUrl: string
  releaseNotes: string
  releasedAt: string
  status: VersionStatus
}

function dateTimeLocal(value?: string) {
  const date = value ? new Date(value) : new Date()
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return local.toISOString().slice(0, 16)
}

const emptyVersionForm = (): VersionForm => ({
  version: '', channel: 'stable', minecraftVersions: props.initialMinecraft, loaders: '', javaVersion: '21', filename: '', sizeMb: '', sha256: '', downloadUrl: '', releaseNotes: '', releasedAt: dateTimeLocal(), status: 'draft',
})
const versionModal = ref(false)
const versionModalMode = ref<ModalMode>('create')
const editingVersionKind = ref<CatalogKind>('core')
const editingVersionId = ref('')
const editingVersionProject = ref('')
const versionForm = ref<VersionForm>(emptyVersionForm())
const versionFormError = ref('')

function openVersionModal(mode: ModalMode, version?: CatalogVersion) {
  if (searchTimer) { window.clearTimeout(searchTimer); searchTimer = undefined }
  ++projectRequestSeq
  ++versionRequestSeq
  loadingProjects.value = false
  loadingVersions.value = false
  versionModalMode.value = mode
  editingVersionKind.value = kind.value
  versionFormError.value = ''
  editingVersionId.value = version?.version || ''
  editingVersionProject.value = version?.project || selectedSlug.value
  versionForm.value = mode === 'edit' && version ? {
    version: version.version,
    channel: version.channel,
    minecraftVersions: version.minecraft_versions.join(', '),
    loaders: version.loaders.join(', '),
    javaVersion: version.java_version ? String(version.java_version) : '',
    filename: version.filename,
    sizeMb: version.size ? (version.size / 1024 / 1024).toFixed(2).replace(/\.00$/, '') : '',
    sha256: version.sha256,
    downloadUrl: version.download_url,
    releaseNotes: version.release_notes,
    releasedAt: dateTimeLocal(version.released_at),
    status: version.status,
  } : emptyVersionForm()
  versionModal.value = true
}

async function saveVersion() {
  const form = versionForm.value
  if (!form.version.trim() || !form.minecraftVersions.trim() || !form.loaders.trim() || !form.filename.trim() || !form.downloadUrl.trim() || !form.releaseNotes.trim() || !form.releasedAt) {
    versionFormError.value = '请完整填写版本号、兼容版本、平台、文件名、下载地址、发布时间和更新说明。'
    return
  }
  const releasedAt = new Date(form.releasedAt)
  if (Number.isNaN(releasedAt.getTime())) { versionFormError.value = '发布时间无效。'; return }
  const sha256 = form.sha256.trim().toLowerCase()
  const size = Math.max(0, Math.round((Number(form.sizeMb) || 0) * 1024 * 1024))
  if (sha256 && !/^[a-f0-9]{64}$/.test(sha256)) { versionFormError.value = 'SHA-256 必须是 64 位十六进制字符。'; return }
  if (form.status === 'published' && (!size || !sha256)) { versionFormError.value = '发布版本必须填写非零文件大小和完整 SHA-256。'; return }
  const parseList = (value: string) => value.split(',').map((item) => item.trim()).filter(Boolean)
  const body = JSON.stringify({
    version: form.version.trim(),
    channel: form.channel,
    minecraft_versions: parseList(form.minecraftVersions),
    loaders: parseList(form.loaders),
    java_version: form.javaVersion ? Number(form.javaVersion) : null,
    filename: form.filename.trim(),
    size,
    sha256,
    download_url: form.downloadUrl.trim(),
    release_notes: form.releaseNotes.trim(),
    released_at: releasedAt.toISOString(),
    status: form.status,
  })
  saving.value = true
  versionFormError.value = ''
  try {
    const base = `/api/catalog/${segmentFor(editingVersionKind.value)}/${encodeURIComponent(editingVersionProject.value)}/versions`
    const path = versionModalMode.value === 'create' ? base : `${base}/${encodeURIComponent(editingVersionId.value)}`
    await apiRequest<CatalogVersion>(path, { method: versionModalMode.value === 'create' ? 'POST' : 'PUT', body })
    versionModal.value = false
    kind.value = editingVersionKind.value
    await Promise.all([loadSummary(), loadProjects(editingVersionProject.value)])
    notify(versionModalMode.value === 'create' ? '版本记录已创建' : '版本记录已更新')
  } catch (error) {
    versionFormError.value = friendlyError(error)
  } finally {
    saving.value = false
  }
}

const confirmOpen = ref(false)
const confirmTitle = ref('')
const confirmBody = ref('')
const confirmAction = ref<null | (() => Promise<void>)>(null)

function askDeleteProject() {
  if (!selectedProject.value) return
  const targetKind = kind.value
  const targetSlug = selectedProject.value.slug
  const targetName = selectedProject.value.name
  confirmTitle.value = `删除 ${targetName}？`
  confirmBody.value = '项目及其全部版本记录会被永久删除；已复制的下载地址也会立即失效。'
  confirmAction.value = async () => {
    await apiRequest<void>(`/api/catalog/${segmentFor(targetKind)}/${encodeURIComponent(targetSlug)}`, { method: 'DELETE' })
    kind.value = targetKind
    selectedSlug.value = ''
    await Promise.all([loadSummary(), loadProjects('')])
    notify('项目已删除')
  }
  confirmOpen.value = true
}

function askDeleteVersion(version: CatalogVersion) {
  const targetKind = kind.value
  const targetProject = version.project
  const targetVersion = version.version
  confirmTitle.value = `删除版本 ${targetVersion}？`
  confirmBody.value = '此操作只删除目录元数据，不会删除上游文件。现有稳定下载路径将无法再解析此版本。'
  confirmAction.value = async () => {
    const path = `/api/catalog/${segmentFor(targetKind)}/${encodeURIComponent(targetProject)}/versions/${encodeURIComponent(targetVersion)}`
    await apiRequest<void>(path, { method: 'DELETE' })
    kind.value = targetKind
    await Promise.all([loadSummary(), loadProjects(targetProject)])
    notify('版本记录已删除')
  }
  confirmOpen.value = true
}

async function executeConfirm() {
  if (!confirmAction.value) return
  saving.value = true
  try {
    await confirmAction.value()
    confirmOpen.value = false
  } catch (error) {
    notify(friendlyError(error))
  } finally {
    saving.value = false
  }
}

const docSection = ref<DocSection>('quickstart')
const codeLanguage = ref<CodeLanguage>('curl')
const docProjects = ref<Record<CatalogKind, CatalogProject[]>>({ core: [], plugin: [] })
const debugKind = ref<CatalogKind>('core')
const debugProject = ref('paper')
const debugMinecraft = ref(props.initialMinecraft)
const debugChannel = ref('stable')
const debugLoading = ref(false)
const debugResult = ref<ResolveResponse | null>(null)
const debugError = ref('')

async function loadDocProjects() {
  try {
    const [cores, plugins] = await Promise.all([
      apiRequest<CatalogProject[]>('/api/catalog/cores'),
      apiRequest<CatalogProject[]>('/api/catalog/plugins'),
    ])
    docProjects.value = { core: cores, plugin: plugins }
    if (!docProjects.value[debugKind.value].some((item) => item.slug === debugProject.value)) {
      debugProject.value = docProjects.value[debugKind.value][0]?.slug || ''
    }
  } catch { /* docs remain readable while API is offline */ }
}

function switchDebugKind(next: CatalogKind) {
  debugKind.value = next
  debugProject.value = docProjects.value[next][0]?.slug || ''
  debugResult.value = null
  debugError.value = ''
}

const resolverPath = computed(() => `/api/v1/resolve?kind=${debugKind.value}&project=${encodeURIComponent(debugProject.value || (debugKind.value === 'core' ? 'paper' : 'luckperms'))}&minecraft=${encodeURIComponent(debugMinecraft.value)}&channel=${encodeURIComponent(debugChannel.value)}`)
const codeSnippet = computed(() => {
  const url = `${API_BASE}${resolverPath.value}`
  if (codeLanguage.value === 'powershell') return `$result = Invoke-RestMethod -Uri "${url}"\nInvoke-WebRequest -Uri ("${API_BASE}" + $result.download_path) -OutFile $result.version.filename`
  if (codeLanguage.value === 'javascript') return `const resolved = await fetch('${url}').then(r => {\n  if (!r.ok) throw new Error('resolve failed')\n  return r.json()\n})\n\nconst downloadUrl = '${API_BASE}' + resolved.download_path`
  return `curl --fail --location \\\n  "${url}"\n\n# 使用响应中的 download_path 下载，并按 sha256 校验文件`
})

async function runResolver() {
  debugLoading.value = true
  debugResult.value = null
  debugError.value = ''
  try {
    debugResult.value = await apiRequest<ResolveResponse>(resolverPath.value)
  } catch (error) {
    debugError.value = friendlyError(error)
  } finally {
    debugLoading.value = false
  }
}

onMounted(async () => {
  await Promise.all([loadSummary(), loadProjects(props.initialCore.trim().toLowerCase() || 'paper')])
})

onUnmounted(() => {
  if (toastTimer) window.clearTimeout(toastTimer)
  if (searchTimer) window.clearTimeout(searchTimer)
  ++projectRequestSeq
  ++versionRequestSeq
})
</script>

<template>
  <section class="mirror-center">
    <header class="mirror-center__header">
      <div class="mirror-center__title">
        <span><Archive /></span>
        <div><h1>镜像资源中心</h1><p>统一管理服务端核心、插件制品与开服器下载接口</p></div>
      </div>
      <nav aria-label="镜像资源中心导航">
        <button role="tab" :aria-selected="section === 'library'" :class="{ active: section === 'library' }" @click="setSection('library')"><Layers3 />资源库</button>
        <button role="tab" :aria-selected="section === 'manage'" :class="{ active: section === 'manage' }" @click="setSection('manage')"><Package />版本管理</button>
        <button role="tab" :aria-selected="section === 'docs'" :class="{ active: section === 'docs' }" @click="setSection('docs')"><BookOpen />API 文档</button>
      </nav>
      <button class="mirror-icon-button" aria-label="刷新目录" title="刷新目录" @click="refreshAll"><RefreshCw :class="{ spin: loadingProjects }" /></button>
    </header>

    <div v-if="section !== 'docs'" class="mirror-catalog">
      <aside class="catalog-rail">
        <div class="catalog-summary">
          <span><b>{{ summary?.core_projects ?? '—' }}</b><small>核心</small></span>
          <span><b>{{ summary?.plugin_projects ?? '—' }}</b><small>插件</small></span>
          <span><b>{{ summary?.versions ?? '—' }}</b><small>版本</small></span>
          <span><b>{{ formatCount(summary?.downloads || 0) }}</b><small>下载</small></span>
        </div>
        <div class="catalog-kind" role="tablist" aria-label="资源类型">
          <button role="tab" :aria-selected="kind === 'core'" :class="{ active: kind === 'core' }" @click="switchKind('core')"><Server />核心库</button>
          <button role="tab" :aria-selected="kind === 'plugin'" :class="{ active: kind === 'plugin' }" @click="switchKind('plugin')"><Puzzle />插件库</button>
        </div>
        <label class="catalog-search"><Search /><input v-model="search" aria-label="搜索资源" placeholder="搜索名称、作者或标签" @input="scheduleSearch"/></label>
        <label class="catalog-filter"><Filter /><select v-model="minecraftFilter" @change="loadProjects('')"><option value="">全部 MC 版本</option><option v-for="item in minecraftOptions" :key="item" :value="item">Minecraft {{ item }}</option></select></label>
        <button v-if="section === 'manage'" class="catalog-create" @click="openProjectModal('create')"><Plus />新建{{ kind === 'core' ? '核心' : '插件' }}项目</button>
        <div class="catalog-list" :class="{ loading: loadingProjects }">
          <button v-for="project in projects" :key="project.slug" :class="{ active: selectedSlug === project.slug }" @click="selectProject(project.slug)">
            <span class="project-mark" :style="{ '--project-color': project.color || '#32d5b0' }"><Box v-if="kind === 'core'"/><Puzzle v-else/></span>
            <span><b>{{ project.name }}</b><small>{{ project.published_versions ? project.latest_version : '暂无已发布版本' }} · {{ project.author }}</small></span>
            <ChevronRight />
          </button>
          <div v-if="loadingProjects" class="rail-state" role="status" aria-live="polite"><LoaderCircle class="spin"/>正在读取目录</div>
          <div v-else-if="!projects.length" class="rail-state"><Search/>没有匹配的资源</div>
        </div>
        <div v-if="section === 'manage'" class="local-admin-note"><ShieldAlert/><span><b>本机管理模式</b><small>当前没有登录与 RBAC，请勿直接暴露公网。</small></span></div>
      </aside>

      <main class="catalog-detail">
        <div v-if="pageError" class="mirror-error" role="alert"><CircleAlert/><span><b>无法读取镜像目录</b><small>{{ pageError }}</small></span><button @click="refreshAll">重试</button></div>
        <template v-else-if="selectedProject">
          <header class="project-hero">
            <div class="project-identity">
              <span class="project-mark large" :style="{ '--project-color': selectedProject.color || '#32d5b0' }"><Box v-if="kind === 'core'"/><Puzzle v-else/></span>
              <div>
                <div><h2>{{ selectedProject.name }}</h2><code>{{ selectedProject.slug }}</code></div>
                <p>{{ selectedProject.summary }}</p>
                <footer><span>由 {{ selectedProject.author }} 维护</span><i/> <span>{{ selectedProject.published_versions }} 个已发布版本</span><i/> <span>{{ formatCount(selectedProject.downloads) }} 次下载</span></footer>
              </div>
            </div>
            <div class="project-actions" v-if="section === 'library'">
              <a v-if="selectedProject.homepage" :href="selectedProject.homepage" target="_blank" rel="noreferrer"><ExternalLink/>项目主页</a>
              <button class="primary" @click="copyText(API_BASE + projectDownloadPath())"><Copy/>复制解析 API</button>
            </div>
            <div class="project-actions" v-else>
              <button @click="openProjectModal('edit')"><Pencil/>编辑资料</button>
              <button class="danger" @click="askDeleteProject"><Trash2/>删除项目</button>
            </div>
          </header>

          <section class="project-description">
            <p>{{ selectedProject.description }}</p>
            <div><span v-for="tag in selectedProject.tags" :key="tag">{{ tag }}</span></div>
          </section>

          <section class="versions-section">
            <header>
              <div><h3>可用版本</h3><p>{{ kind === 'core' ? '按 Minecraft 与发行渠道选择服务端制品' : '检查兼容版本与加载平台后下载插件' }}</p></div>
              <span class="version-controls">
                <select v-model="channelFilter"><option value="">全部渠道</option><option value="stable">Stable</option><option value="beta">Beta</option><option value="snapshot">Snapshot</option></select>
                <button v-if="section === 'manage'" class="primary" @click="openVersionModal('create')"><Plus/>新建版本</button>
              </span>
            </header>

            <div class="version-table-wrap">
              <table class="version-table">
                <thead><tr><th>版本 / 渠道</th><th>Minecraft</th><th>{{ kind === 'core' ? 'Java' : '平台' }}</th><th>制品</th><th>发布时间</th><th>下载</th><th aria-label="操作"></th></tr></thead>
                <tbody>
                  <tr v-for="version in visibleVersions" :key="version.id" :class="{ muted: version.status !== 'published' }">
                    <td><b>{{ version.version }}</b><span><em class="channel" :class="version.channel">{{ version.channel }}</em><em class="status" :class="version.status">{{ version.status === 'published' ? '已发布' : version.status === 'draft' ? '草稿' : '已撤回' }}</em></span></td>
                    <td><span class="compat-list"><code v-for="item in version.minecraft_versions.slice(0, 2)" :key="item">{{ item }}</code><small v-if="version.minecraft_versions.length > 2">+{{ version.minecraft_versions.length - 2 }}</small></span></td>
                    <td><span v-if="kind === 'core'">{{ version.java_version ? `Java ${version.java_version}` : '—' }}</span><span v-else>{{ version.loaders.join(' / ') || 'Bukkit' }}</span></td>
                    <td><b class="artifact-name">{{ version.filename }}</b><small>{{ formatSize(version.size) }} · SHA {{ version.sha256 ? version.sha256.slice(0, 8) : '未提供' }}</small></td>
                    <td><span>{{ formatDate(version.released_at) }}</span></td>
                    <td><span>{{ formatCount(version.downloads) }}</span></td>
                    <td>
                      <span v-if="section === 'manage'" class="row-actions"><button title="编辑版本" aria-label="编辑版本" @click="openVersionModal('edit', version)"><Pencil/></button><button class="danger" title="删除版本" aria-label="删除版本" @click="askDeleteVersion(version)"><Trash2/></button></span>
                      <span v-else class="row-actions"><button title="复制下载地址" aria-label="复制下载地址" @click="copyText(API_BASE + versionDownloadPath(version))"><Copy/></button><button class="download" title="下载" aria-label="下载" :disabled="version.status !== 'published'" @click="downloadVersion(version)"><Download/></button></span>
                    </td>
                  </tr>
                </tbody>
              </table>
              <div v-if="loadingVersions" class="table-state" role="status" aria-live="polite"><LoaderCircle class="spin"/>正在读取版本</div>
              <div v-else-if="!visibleVersions.length" class="table-state"><Package/>当前筛选条件下没有版本</div>
            </div>
          </section>
        </template>
        <div v-else-if="!loadingProjects" class="catalog-empty"><Archive/><h2>选择一个资源项目</h2><p>从左侧核心库或插件库中选择项目，查看兼容版本与下载接口。</p></div>
      </main>
    </div>

    <div v-else class="api-docs">
      <aside class="docs-rail">
        <div><b>API 参考</b><small>v1 · JSON / HTTP</small></div>
        <nav>
          <button :class="{ active: docSection === 'quickstart' }" @click="docSection='quickstart'"><BookOpen/>快速开始</button>
          <button :class="{ active: docSection === 'catalog' }" @click="docSection='catalog'"><Layers3/>目录查询</button>
          <button :class="{ active: docSection === 'resolve' }" @click="docSection='resolve'"><Code2/>版本解析</button>
          <button :class="{ active: docSection === 'download' }" @click="docSection='download'"><CloudDownload/>文件下载</button>
          <button :class="{ active: docSection === 'errors' }" @click="docSection='errors'"><CircleAlert/>错误处理</button>
        </nav>
        <a :href="apiUrl('/api/openapi.json')" target="_blank" rel="noreferrer"><FileJson/>OpenAPI JSON<ExternalLink/></a>
      </aside>

      <main class="docs-content">
        <section v-if="docSection === 'quickstart'" class="doc-page">
          <header><h2>快速开始</h2><p>开服器只需两步：解析出兼容的已发布版本，再访问稳定下载路径。</p></header>
          <div class="base-url"><span><small>BASE URL</small><code>{{ API_BASE }}/api/v1</code></span><button @click="copyText(API_BASE + '/api/v1')"><Copy/>复制</button></div>
          <article><h3>解析并下载</h3><p>解析接口按项目、Minecraft 版本和渠道选择最新兼容制品。响应会包含文件名、SHA-256 与不会随上游地址变化的 <code>download_path</code>。</p></article>
          <div class="code-sample">
            <header><span><button v-for="lang in (['curl','powershell','javascript'] as CodeLanguage[])" :key="lang" :class="{ active: codeLanguage === lang }" @click="codeLanguage=lang">{{ lang === 'powershell' ? 'PowerShell' : lang === 'javascript' ? 'JavaScript' : 'cURL' }}</button></span><button @click="copyText(codeSnippet)"><Clipboard/>复制代码</button></header>
            <pre><code>{{ codeSnippet }}</code></pre>
          </div>
          <article class="doc-callout"><Check/><span><b>为开服器准备</b><p>公开查询不需要认证；管理 CRUD 仅用于本机工作台。生产部署前应在反向代理层为管理接口增加鉴权。</p></span></article>
        </section>

        <section v-else-if="docSection === 'catalog'" class="doc-page">
          <header><h2>目录查询</h2><p>列出核心、插件及其版本，可按关键词和 Minecraft 版本筛选。</p></header>
          <article class="endpoint"><h3><em>GET</em><code>/api/catalog/cores</code></h3><p>查询所有服务端核心。插件列表使用 <code>/api/catalog/plugins</code>。</p><table><tbody><tr><th>search</th><td>可选</td><td>匹配名称、作者、摘要或标签</td></tr><tr><th>minecraft</th><td>可选</td><td>只返回包含兼容版本的项目</td></tr></tbody></table></article>
          <article class="endpoint"><h3><em>GET</em><code>/api/catalog/cores/{slug}/versions</code></h3><p>获取项目的完整版本历史；将 <code>cores</code> 替换为 <code>plugins</code> 即可查询插件。</p></article>
        </section>

        <section v-else-if="docSection === 'resolve'" class="doc-page docs-split">
          <div>
            <header><h2>版本解析</h2><p>让开服器不必硬编码构建号或上游地址。</p></header>
            <article class="endpoint"><h3><em>GET</em><code>/api/v1/resolve</code></h3><table><tbody><tr><th>kind</th><td>必填</td><td><code>core</code> 或 <code>plugin</code></td></tr><tr><th>project</th><td>必填</td><td>项目 slug，例如 <code>paper</code></td></tr><tr><th>minecraft</th><td>必填</td><td>Minecraft 版本</td></tr><tr><th>channel</th><td>可选</td><td>默认 <code>stable</code></td></tr></tbody></table></article>
            <article><h3>响应策略</h3><p>只选择 <code>published</code> 状态、Minecraft 版本与渠道均匹配的记录，并按发布时间返回最新版本。没有匹配项时返回 404。</p></article>
          </div>
          <aside class="api-console">
            <header><span><i/>在线调试</span><small>连接本机 API</small></header>
            <div class="console-form"><label>资源类型<select :value="debugKind" @change="switchDebugKind(($event.target as HTMLSelectElement).value as CatalogKind)"><option value="core">服务端核心</option><option value="plugin">插件</option></select></label><label>项目<select v-model="debugProject"><option v-for="project in docProjects[debugKind]" :key="project.slug" :value="project.slug">{{ project.name }}</option></select></label><label>Minecraft<input v-model="debugMinecraft"/></label><label>渠道<select v-model="debugChannel"><option value="stable">stable</option><option value="beta">beta</option><option value="snapshot">snapshot</option></select></label><button :disabled="debugLoading || !debugProject" @click="runResolver"><LoaderCircle v-if="debugLoading" class="spin"/><Code2 v-else/>发送请求</button></div>
            <pre v-if="debugResult"><code>{{ JSON.stringify(debugResult, null, 2) }}</code></pre>
            <div v-else-if="debugError" class="console-error"><CircleAlert/>{{ debugError }}</div>
            <div v-else class="console-empty"><Code2/>请求结果会显示在这里</div>
          </aside>
        </section>

        <section v-else-if="docSection === 'download'" class="doc-page">
          <header><h2>文件下载</h2><p>通过稳定路径访问制品，服务会记录下载次数并以 HTTP 307 跳转到已配置的上游文件。</p></header>
          <article class="endpoint"><h3><em>GET</em><code>/api/v1/download/{kind}/{project}/{version}</code></h3><p>例如：<code>/api/v1/download/core/paper/1.21.4-232</code>。客户端必须跟随重定向。</p></article>
          <article><h3>完整性校验</h3><p>先从解析响应读取 <code>version.sha256</code>，下载完成后再校验文件。若目录未提供校验和，开服器应提示风险，而不是静默执行 JAR。</p></article>
          <article class="doc-callout warning"><ShieldAlert/><span><b>当前是重定向式镜像目录</b><p>MVP 不在本机托管二进制文件，因此暂不提供 Range、ETag 或断点续传；这些能力应在接入对象存储后补充。</p></span></article>
        </section>

        <section v-else class="doc-page">
          <header><h2>错误处理</h2><p>所有失败都使用标准 HTTP 状态码，并在响应正文提供可读错误信息。</p></header>
          <article class="endpoint error-codes"><table><tbody><tr><th>400</th><td>参数、URL、SHA-256 或项目数据不合法</td></tr><tr><th>404</th><td>项目、版本或兼容制品不存在</td></tr><tr><th>409</th><td>slug 或版本号重复，或资源仍有版本无法删除</td></tr><tr><th>500</th><td>状态持久化或服务器内部错误</td></tr></tbody></table></article>
          <article><h3>推荐重试策略</h3><p>只对网络错误、307 上游失败和 5xx 采用指数退避；400、404 与 409 应直接向用户展示并停止自动重试。</p></article>
        </section>
      </main>
    </div>

    <div v-if="projectModal" class="mirror-modal-backdrop" @click.self="projectModal=false">
      <form class="mirror-modal" role="dialog" aria-modal="true" aria-labelledby="project-dialog-title" @submit.prevent="saveProject">
        <header><div><small>{{ projectModalMode === 'create' ? 'NEW PROJECT' : 'PROJECT SETTINGS' }}</small><h2 id="project-dialog-title">{{ projectModalMode === 'create' ? `新建${kind === 'core' ? '核心' : '插件'}项目` : `编辑 ${selectedProject?.name}` }}</h2></div><button type="button" aria-label="关闭" @click="projectModal=false"><X/></button></header>
        <main class="mirror-form-grid">
          <label><span>项目标识 *</span><input v-model="projectForm.slug" required :disabled="projectModalMode === 'edit'" placeholder="paper" pattern="[a-z0-9][a-z0-9-]*"/></label>
          <label><span>显示名称 *</span><input v-model="projectForm.name" required placeholder="Paper"/></label>
          <label class="wide"><span>一句话摘要 *</span><input v-model="projectForm.summary" required placeholder="面向插件服务器的高性能 Minecraft 核心"/></label>
          <label class="wide"><span>详细说明 *</span><textarea v-model="projectForm.description" required rows="3"/></label>
          <label><span>维护者 *</span><input v-model="projectForm.author" required/></label>
          <label><span>主题色</span><div class="color-input"><input v-model="projectForm.color" type="color"/><input v-model="projectForm.color"/></div></label>
          <label><span>项目主页 *</span><input v-model="projectForm.homepage" required type="url" placeholder="https://..."/></label>
          <label><span>代码仓库 *</span><input v-model="projectForm.repository" required type="url" placeholder="https://..."/></label>
          <label class="wide"><span>标签（逗号分隔）</span><input v-model="projectForm.tags" placeholder="性能, Bukkit, 推荐"/></label>
          <label class="wide checkbox"><input v-model="projectForm.featured" type="checkbox"/><span>在资源库中标记为推荐项目</span></label>
          <p v-if="projectFormError" class="form-error"><CircleAlert/>{{ projectFormError }}</p>
        </main>
        <footer><button type="button" @click="projectModal=false">取消</button><button class="primary" :disabled="saving"><LoaderCircle v-if="saving" class="spin"/><Check v-else/>保存项目</button></footer>
      </form>
    </div>

    <div v-if="versionModal" class="mirror-modal-backdrop" @click.self="versionModal=false">
      <form class="mirror-modal version-modal" role="dialog" aria-modal="true" aria-labelledby="version-dialog-title" @submit.prevent="saveVersion">
        <header><div><small>{{ versionModalMode === 'create' ? 'NEW RELEASE' : 'RELEASE SETTINGS' }}</small><h2 id="version-dialog-title">{{ versionModalMode === 'create' ? `为 ${selectedProject?.name} 新建版本` : `编辑 ${versionForm.version}` }}</h2></div><button type="button" aria-label="关闭" @click="versionModal=false"><X/></button></header>
        <main class="mirror-form-grid">
          <label><span>版本号 *</span><input v-model="versionForm.version" required placeholder="1.21.4-232"/></label>
          <label><span>发行渠道</span><select v-model="versionForm.channel"><option value="stable">stable</option><option value="beta">beta</option><option value="snapshot">snapshot</option></select></label>
          <label><span>Minecraft 版本 *（逗号分隔）</span><input v-model="versionForm.minecraftVersions" required placeholder="1.21.4, 1.21.3"/></label>
          <label><span>{{ kind === 'core' ? '支持平台' : '加载器/核心' }} *（逗号分隔）</span><input v-model="versionForm.loaders" required placeholder="Paper, Purpur"/></label>
          <label><span>Java 版本</span><input v-model="versionForm.javaVersion" type="number" min="8" max="99"/></label>
          <label><span>状态</span><select v-model="versionForm.status"><option value="draft">草稿</option><option value="published">已发布</option><option value="yanked">已撤回</option></select></label>
          <label><span>文件名 *</span><input v-model="versionForm.filename" required placeholder="paper-1.21.4-232.jar"/></label>
          <label><span>文件大小（MB）{{ versionForm.status === 'published' ? ' *' : '' }}</span><input v-model="versionForm.sizeMb" :required="versionForm.status === 'published'" type="number" min="0" step="0.01"/></label>
          <label class="wide"><span>上游下载地址 *</span><input v-model="versionForm.downloadUrl" required type="url" placeholder="https://.../artifact.jar"/></label>
          <label class="wide"><span>SHA-256（发布时必填，64 位十六进制）{{ versionForm.status === 'published' ? ' *' : '' }}</span><input v-model="versionForm.sha256" :required="versionForm.status === 'published'" class="mono" minlength="64" maxlength="64"/></label>
          <label><span>发布时间 *</span><input v-model="versionForm.releasedAt" required type="datetime-local"/></label>
          <label class="wide"><span>更新说明 *</span><textarea v-model="versionForm.releaseNotes" required rows="3"/></label>
          <p v-if="versionFormError" class="form-error"><CircleAlert/>{{ versionFormError }}</p>
        </main>
        <footer><button type="button" @click="versionModal=false">取消</button><button class="primary" :disabled="saving"><LoaderCircle v-if="saving" class="spin"/><Check v-else/>保存版本</button></footer>
      </form>
    </div>

    <div v-if="confirmOpen" class="mirror-modal-backdrop" @click.self="confirmOpen=false">
      <section class="mirror-confirm" role="dialog" aria-modal="true" aria-labelledby="confirm-dialog-title">
        <span><Trash2/></span><h2 id="confirm-dialog-title">{{ confirmTitle }}</h2><p>{{ confirmBody }}</p><footer><button @click="confirmOpen=false">取消</button><button class="danger" :disabled="saving" @click="executeConfirm"><LoaderCircle v-if="saving" class="spin"/><Trash2 v-else/>确认删除</button></footer>
      </section>
    </div>

    <transition name="mirror-toast"><div v-if="toast" class="mirror-toast" role="status" aria-live="polite"><Check/>{{ toast }}</div></transition>
  </section>
</template>
