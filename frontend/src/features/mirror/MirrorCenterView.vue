<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import {
  Archive,
  BookOpen,
  Bot,
  Box,
  Boxes,
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
  FileCog,
  Filter,
  Layers3,
  ImageIcon,
  LoaderCircle,
  Package,
  Pencil,
  Plus,
  Puzzle,
  RefreshCw,
  Search,
  Server,
  Shirt,
  ShieldAlert,
  Trash2,
  UploadCloud,
  X,
} from 'lucide-vue-next'
import { RESOURCE_API_BASE, resourceApiRequest, resourceApiUrl } from '../../lib/resource-api'
import type {
  CatalogKind,
  CatalogProject,
  CatalogSummary,
  CatalogVersion,
  ResolveResponse,
  PluginCategory,
  ResourceUpload,
  VersionStatus,
} from './types'

type Section = 'library' | 'manage' | 'docs'
type ModalMode = 'create' | 'edit'
type CodeLanguage = 'curl' | 'powershell' | 'javascript'
type DocSection = 'quickstart' | 'catalog' | 'resolve' | 'download' | 'errors'

interface ResourceKindConfig {
  kind: CatalogKind
  segment: string
  label: string
  shortLabel: string
  description: string
  requiresMinecraft: boolean
  formatLabel: string
  formatPlaceholder: string
  defaultFormats: string
}

interface MslCoreSyncStatus {
  enabled: boolean
  base_url: string
  target_versions: string[]
  interval_seconds: number
  running: boolean
  last_started_at?: string
  last_finished_at?: string
  last_error?: string
  core_types: number
  matching_core_types: number
  projects_created: number
  projects_refreshed: number
  versions_upserted: number
  mirror_sources?: string[]
  placeholders_created?: number
  versions_resolved?: number
  sizes_backfilled?: number
  pending_versions?: number
  versions_removed: number
  skipped_manual_versions: number
  failures: string[]
}

const RESOURCE_KINDS: ResourceKindConfig[] = [
  { kind: 'core', segment: 'cores', label: '核心库', shortLabel: '核心', description: '按 Minecraft 与发行渠道选择服务端制品', requiresMinecraft: true, formatLabel: '支持平台', formatPlaceholder: 'Paper, Purpur', defaultFormats: '' },
  { kind: 'plugin', segment: 'plugins', label: '插件库', shortLabel: '插件', description: '检查兼容版本与加载平台后下载插件', requiresMinecraft: true, formatLabel: '加载器 / 核心', formatPlaceholder: 'Paper, Fabric, Velocity', defaultFormats: '' },
  { kind: 'skin', segment: 'skins', label: '皮肤库', shortLabel: '皮肤', description: '下载玩家皮肤、角色套装与配套预览图', requiresMinecraft: false, formatLabel: '资源格式', formatPlaceholder: 'png, zip', defaultFormats: 'png' },
  { kind: 'bbmodel', segment: 'bbmodels', label: 'BBModel 模型库', shortLabel: '模型', description: '管理 Blockbench BBModel 模型与配套纹理', requiresMinecraft: false, formatLabel: '资源格式', formatPlaceholder: 'bbmodel, zip', defaultFormats: 'bbmodel' },
  { kind: 'ui_texture', segment: 'ui-textures', label: 'UI 贴图库', shortLabel: 'UI 贴图', description: '管理菜单、HUD、图标与材质包 UI 资源', requiresMinecraft: false, formatLabel: '资源格式', formatPlaceholder: 'png, zip, json', defaultFormats: 'png' },
  { kind: 'skill', segment: 'skills', label: 'Skill 库', shortLabel: 'Skill', description: '管理插件专属配置 Skill 与可安装能力包', requiresMinecraft: false, formatLabel: 'Skill 格式', formatPlaceholder: 'skill-bundle+json', defaultFormats: 'skill-bundle+json' },
  { kind: 'plugin_config', segment: 'plugin-configs', label: '插件配置库', shortLabel: '插件配置', description: '管理插件配置模板、字段参考与升级迁移规则', requiresMinecraft: false, formatLabel: '配置格式', formatPlaceholder: 'plugin-config+json, yaml', defaultFormats: 'plugin-config+json' },
]

const PLUGIN_CATEGORIES: { value: PluginCategory; label: string; short: string }[] = [
  { value: 'mainstream', label: '主流插件库', short: '主流' },
  { value: 'open_source', label: '开源插件库', short: '开源' },
  { value: 'standard', label: '普通插件库', short: '普通' },
  { value: 'paid', label: '付费插件库', short: '付费' },
]
const pluginCategoryLabel = (value: string) => PLUGIN_CATEGORIES.find((item) => item.value === value)?.short || '普通'

const resourceKind = (value: CatalogKind) => RESOURCE_KINDS.find((item) => item.kind === value) || RESOURCE_KINDS[0]

const props = withDefaults(defineProps<{ initialCore?: string; initialMinecraft?: string; adminMode?: boolean }>(), {
  initialCore: 'Paper',
  initialMinecraft: '1.21.4',
  adminMode: false,
})
const emit = defineEmits<{ catalogUpdated: [CatalogSummary] }>()

const section = ref<Section>(props.adminMode ? 'manage' : 'library')
const kind = ref<CatalogKind>('core')
const projects = ref<CatalogProject[]>([])
const versions = ref<CatalogVersion[]>([])
const summary = ref<CatalogSummary | null>(null)
const selectedSlug = ref('')
const search = ref('')
const minecraftFilter = ref('')
const channelFilter = ref('')
const pluginCategoryFilter = ref('')
const loadingProjects = ref(false)
const loadingVersions = ref(false)
const saving = ref(false)
const mslSyncing = ref(false)
const mslStatus = ref<MslCoreSyncStatus | null>(null)
const pageError = ref('')
const toast = ref('')
let toastTimer: number | undefined
let searchTimer: number | undefined
let projectRequestSeq = 0
let versionRequestSeq = 0

const selectedProject = computed(() => projects.value.find((item) => item.slug === selectedSlug.value) || null)
const kindConfig = computed(() => resourceKind(kind.value))
const resourceSegment = computed(() => kindConfig.value.segment)
const minecraftOptions = computed(() => {
  const values = new Set<string>()
  projects.value.forEach((project) => project.minecraft_versions?.forEach((version) => values.add(version)))
  versions.value.forEach((version) => version.minecraft_versions.forEach((item) => values.add(item)))
  if (kindConfig.value.requiresMinecraft) {
    ;['1.21.4', '1.21.1', '1.20.6', '1.20.4'].forEach((item) => values.add(item))
  }
  return [...values].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
})
const visibleVersions = computed(() => versions.value.filter((version) => {
  const minecraftMatches = !minecraftFilter.value || version.minecraft_versions.includes(minecraftFilter.value)
  const channelMatches = !channelFilter.value || version.channel === channelFilter.value
  const statusMatches = section.value === 'manage' || version.status === 'published'
  return minecraftMatches && channelMatches && statusMatches
}))

const segmentFor = (value: CatalogKind) => resourceKind(value).segment

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
  try {
    summary.value = await resourceApiRequest<CatalogSummary>('/api/catalog/summary')
    emit('catalogUpdated', summary.value)
  } catch { /* page list handles connectivity */ }
}

async function loadProjects(preferredSlug = selectedSlug.value) {
  const requestSeq = ++projectRequestSeq
  ++versionRequestSeq
  const segment = resourceSegment.value
  loadingProjects.value = true
  pageError.value = ''
  try {
    const query = queryString({ search: search.value.trim(), minecraft: minecraftFilter.value, plugin_category: kind.value === 'plugin' ? pluginCategoryFilter.value : '' })
    const result = await resourceApiRequest<CatalogProject[]>(`/api/catalog/${segment}${query}`)
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
    const result = await resourceApiRequest<CatalogVersion[]>(`/api/catalog/${segment}/${encodeURIComponent(slug)}/versions`)
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
  await Promise.all([loadSummary(), loadProjects(), props.adminMode ? loadMslStatus() : Promise.resolve()])
  notify(pageError.value ? '目录刷新失败，请检查资源中心连接' : '远程资源目录已刷新')
}

async function loadMslStatus() {
  if (!props.adminMode) return
  try {
    mslStatus.value = await resourceApiRequest<MslCoreSyncStatus>('/api/catalog/admin/msl-core-status')
  } catch (error) {
    console.warn('无法读取 MSL 核心同步状态', error)
  }
}

async function syncMslCores() {
  if (mslSyncing.value || mslStatus.value?.running) return
  mslSyncing.value = true
  try {
    mslStatus.value = await resourceApiRequest<MslCoreSyncStatus>('/api/catalog/admin/sync-msl-cores', { method: 'POST' })
    await Promise.all([loadSummary(), loadProjects(selectedSlug.value)])
    notify(`核心镜像检查完成：补齐 ${mslStatus.value.versions_resolved ?? mslStatus.value.versions_upserted} 个版本，新增 ${mslStatus.value.placeholders_created ?? 0} 个占位`)
  } catch (error) {
    notify(`核心镜像检查失败：${friendlyError(error)}`)
    await loadMslStatus()
  } finally {
    mslSyncing.value = false
  }
}

async function switchKind(next: CatalogKind) {
  if (kind.value === next) return
  kind.value = next
  selectedSlug.value = ''
  search.value = ''
  minecraftFilter.value = ''
  channelFilter.value = ''
  pluginCategoryFilter.value = ''
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
  const minecraftQuery = kindConfig.value.requiresMinecraft ? `&minecraft=${encodeURIComponent(minecraft)}` : ''
  return `/api/v1/resolve?kind=${kind.value}&project=${encodeURIComponent(project.slug)}${minecraftQuery}&channel=stable`
}

function versionDownloadPath(version: CatalogVersion) {
  return `/api/v1/download/${kind.value}/${encodeURIComponent(version.project)}/${encodeURIComponent(version.version)}`
}

function downloadVersion(version: CatalogVersion) {
  if (version.status !== 'published') { notify('该版本尚未发布，不能通过公开 API 下载'); return }
  window.open(resourceApiUrl(versionDownloadPath(version)), '_blank', 'noopener,noreferrer')
}

interface ProjectForm {
  slug: string
  name: string
  summary: string
  description: string
  author: string
  homepage: string
  repository: string
  previewUrl: string
  license: string
  pluginCategory: PluginCategory
  targetPlugin: string
  tags: string
  color: string
  featured: boolean
}

const emptyProjectForm = (): ProjectForm => ({
  slug: '', name: '', summary: '', description: '', author: '', homepage: '', repository: '', previewUrl: '', license: '', pluginCategory: 'standard', targetPlugin: '', tags: '', color: '#32d5b0', featured: false,
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
    previewUrl: project.preview_url || '',
    license: project.license || '',
    pluginCategory: (project.plugin_category || 'standard') as PluginCategory,
    targetPlugin: project.target_plugin || '',
    tags: project.tags.join(', '),
    color: project.color || '#32d5b0',
    featured: project.featured,
  } : emptyProjectForm()
  projectModal.value = true
}

async function saveProject() {
  const form = projectForm.value
  if (![form.slug, form.name, form.summary, form.description, form.author, form.homepage].every((value) => value.trim())) {
    projectFormError.value = '请填写标识、名称、摘要、详细说明、维护者和项目主页。'
    return
  }
  if ((editingProjectKind.value === 'skill' || editingProjectKind.value === 'plugin_config') && !form.targetPlugin.trim()) {
    projectFormError.value = 'Skill 和插件配置项目必须绑定目标插件 slug。'
    return
  }
  saving.value = true
  projectFormError.value = ''
  const body = JSON.stringify({
    ...form,
    slug: form.slug.trim().toLowerCase(),
    name: form.name.trim(),
    summary: form.summary.trim(),
    preview_url: form.previewUrl.trim(),
    license: form.license.trim(),
    plugin_category: editingProjectKind.value === 'plugin' ? form.pluginCategory : '',
    target_plugin: form.targetPlugin.trim().toLowerCase(),
    tags: form.tags.split(',').map((item) => item.trim()).filter(Boolean),
  })
  try {
    const creating = projectModalMode.value === 'create'
    const segment = segmentFor(editingProjectKind.value)
    const path = creating
      ? `/api/catalog/${segment}`
      : `/api/catalog/${segment}/${encodeURIComponent(editingProjectSlug.value)}`
    const saved = await resourceApiRequest<CatalogProject>(path, { method: creating ? 'POST' : 'PUT', body })
    projectModal.value = false
    kind.value = editingProjectKind.value
    await Promise.all([loadSummary(), loadProjects(saved.slug)])
    if (creating) {
      notify('项目已创建，请继续上传第一个版本')
      await nextTick()
      openVersionModal('create')
    } else {
      notify('项目资料已保存')
    }
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
  formats: string
  javaVersion: string
  filename: string
  sizeMb: string
  sha256: string
  downloadUrl: string
  releaseNotes: string
  content: string
  releasedAt: string
  status: VersionStatus
}

function dateTimeLocal(value?: string) {
  const date = value ? new Date(value) : new Date()
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return local.toISOString().slice(0, 16)
}

const emptyVersionForm = (targetKind: CatalogKind = kind.value): VersionForm => {
  const config = resourceKind(targetKind)
  return {
    version: '',
    channel: 'stable',
    minecraftVersions: config.requiresMinecraft ? props.initialMinecraft : '',
    loaders: '',
    formats: config.defaultFormats,
    javaVersion: targetKind === 'core' ? '21' : '',
    filename: '',
    sizeMb: '',
    sha256: '',
    downloadUrl: '',
    releaseNotes: '通过资源管理页上传发布',
    content: '',
    releasedAt: dateTimeLocal(),
    status: 'published',
  }
}
const versionModal = ref(false)
const versionModalMode = ref<ModalMode>('create')
const editingVersionKind = ref<CatalogKind>('core')
const editingVersionId = ref('')
const editingVersionProject = ref('')
const versionForm = ref<VersionForm>(emptyVersionForm())
const versionFormError = ref('')
const artifactFile = ref<File | null>(null)
const artifactDragActive = ref(false)

function acceptArtifact(file?: File | null) {
  artifactFile.value = file || null
  artifactDragActive.value = false
  if (!artifactFile.value) return
  versionForm.value.filename = artifactFile.value.name
  versionForm.value.sizeMb = (artifactFile.value.size / 1024 / 1024).toFixed(2)
  versionForm.value.sha256 = ''
  versionForm.value.downloadUrl = ''
}

function selectArtifact(event: Event) {
  const input = event.target as HTMLInputElement
  acceptArtifact(input.files?.[0])
}

function dropArtifact(event: DragEvent) {
  acceptArtifact(event.dataTransfer?.files?.[0])
}

function openVersionModal(mode: ModalMode, version?: CatalogVersion) {
  if (searchTimer) { window.clearTimeout(searchTimer); searchTimer = undefined }
  ++projectRequestSeq
  ++versionRequestSeq
  loadingProjects.value = false
  loadingVersions.value = false
  versionModalMode.value = mode
  editingVersionKind.value = kind.value
  versionFormError.value = ''
  artifactFile.value = null
  editingVersionId.value = version?.version || ''
  editingVersionProject.value = version?.project || selectedSlug.value
  versionForm.value = mode === 'edit' && version ? {
    version: version.version,
    channel: version.channel,
    minecraftVersions: version.minecraft_versions.join(', '),
    loaders: version.loaders.join(', '),
    formats: (version.formats || []).join(', '),
    javaVersion: version.java_version ? String(version.java_version) : '',
    filename: version.filename,
    sizeMb: version.size ? (version.size / 1024 / 1024).toFixed(2).replace(/\.00$/, '') : '',
    sha256: version.sha256,
    downloadUrl: version.download_url,
    releaseNotes: version.release_notes,
    content: version.content || '',
    releasedAt: dateTimeLocal(version.released_at),
    status: version.status,
  } : emptyVersionForm(kind.value)
  versionModal.value = true
}

async function saveVersion() {
  const form = versionForm.value
  const selectedFile = artifactFile.value
  const config = resourceKind(editingVersionKind.value)
  const compatibilityMissing = config.requiresMinecraft
    ? !form.minecraftVersions.trim() || !form.loaders.trim()
    : !form.formats.trim()
  const inlineKind = editingVersionKind.value === 'skill' || editingVersionKind.value === 'plugin_config'
  if (!form.version.trim() || compatibilityMissing || !form.filename.trim() || (!inlineKind && !form.downloadUrl.trim() && !selectedFile) || (inlineKind && !form.content.trim() && !form.downloadUrl.trim() && !selectedFile) || !form.releaseNotes.trim() || !form.releasedAt) {
    versionFormError.value = config.requiresMinecraft
      ? '请完整填写版本号、兼容版本、平台、文件名、下载地址、发布时间和更新说明。'
      : '请完整填写版本号、资源格式、文件名、下载地址、发布时间和更新说明。'
    return
  }
  const releasedAt = new Date(form.releasedAt)
  if (Number.isNaN(releasedAt.getTime())) { versionFormError.value = '发布时间无效。'; return }
  const sha256 = form.sha256.trim().toLowerCase()
  const inlineBytes = inlineKind ? new TextEncoder().encode(form.content).byteLength : 0
  const size = inlineBytes || Math.max(0, Math.round((Number(form.sizeMb) || 0) * 1024 * 1024))
  if (sha256 && !/^[a-f0-9]{64}$/.test(sha256)) { versionFormError.value = 'SHA-256 必须是 64 位十六进制字符。'; return }
  if (form.status === 'published' && !inlineKind && !selectedFile && (!size || !sha256)) { versionFormError.value = '发布版本必须填写非零文件大小和完整 SHA-256，或直接选择文件上传。'; return }
  const parseList = (value: string) => value.split(',').map((item) => item.trim()).filter(Boolean)
  saving.value = true
  versionFormError.value = ''
  try {
    let uploaded: ResourceUpload | null = null
    if (selectedFile) {
      const query = new URLSearchParams({
        kind: editingVersionKind.value,
        project: editingVersionProject.value,
        version: form.version.trim(),
        filename: form.filename.trim(),
      })
      uploaded = await resourceApiRequest<ResourceUpload>(`/api/catalog/admin/upload?${query}`, {
        method: 'POST',
        headers: { 'Content-Type': selectedFile.type || 'application/octet-stream' },
        body: selectedFile,
      })
      form.downloadUrl = uploaded.download_url
      form.sha256 = uploaded.sha256
      form.sizeMb = (uploaded.size / 1024 / 1024).toFixed(2)
    }
    const body = JSON.stringify({
      version: form.version.trim(),
      channel: form.channel,
      minecraft_versions: parseList(form.minecraftVersions),
      loaders: parseList(form.loaders),
      formats: parseList(form.formats),
      java_version: form.javaVersion ? Number(form.javaVersion) : null,
      filename: form.filename.trim(),
      size: uploaded?.size || size,
      sha256: uploaded?.sha256 || sha256,
      download_url: uploaded?.download_url || form.downloadUrl.trim(),
      content: inlineKind && !uploaded ? form.content : '',
      release_notes: form.releaseNotes.trim(),
      released_at: releasedAt.toISOString(),
      status: form.status,
    })
    const base = `/api/catalog/${segmentFor(editingVersionKind.value)}/${encodeURIComponent(editingVersionProject.value)}/versions`
    const path = versionModalMode.value === 'create' ? base : `${base}/${encodeURIComponent(editingVersionId.value)}`
    await resourceApiRequest<CatalogVersion>(path, { method: versionModalMode.value === 'create' ? 'POST' : 'PUT', body })
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
    await resourceApiRequest<void>(`/api/catalog/${segmentFor(targetKind)}/${encodeURIComponent(targetSlug)}`, { method: 'DELETE' })
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
    await resourceApiRequest<void>(path, { method: 'DELETE' })
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
const emptyDocProjects = () => Object.fromEntries(
  RESOURCE_KINDS.map((item) => [item.kind, []]),
) as unknown as Record<CatalogKind, CatalogProject[]>
const docProjects = ref<Record<CatalogKind, CatalogProject[]>>(emptyDocProjects())
const debugKind = ref<CatalogKind>('core')
const debugProject = ref('paper')
const debugMinecraft = ref(props.initialMinecraft)
const debugChannel = ref('stable')
const debugLoading = ref(false)
const debugResult = ref<ResolveResponse | null>(null)
const debugError = ref('')

async function loadDocProjects() {
  try {
    const entries = await Promise.all(RESOURCE_KINDS.map(async (item) => [
      item.kind,
      await resourceApiRequest<CatalogProject[]>(`/api/catalog/${item.segment}`),
    ] as const))
    docProjects.value = Object.fromEntries(entries) as Record<CatalogKind, CatalogProject[]>
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

const resolverPath = computed(() => {
  const fallbackProject = debugKind.value === 'core' ? 'paper' : debugKind.value === 'plugin' ? 'luckperms' : ''
  const minecraft = resourceKind(debugKind.value).requiresMinecraft
    ? `&minecraft=${encodeURIComponent(debugMinecraft.value)}`
    : ''
  return `/api/v1/resolve?kind=${debugKind.value}&project=${encodeURIComponent(debugProject.value || fallbackProject)}${minecraft}&channel=${encodeURIComponent(debugChannel.value)}`
})
const codeSnippet = computed(() => {
  const url = `${RESOURCE_API_BASE}${resolverPath.value}`
  if (codeLanguage.value === 'powershell') return `$result = Invoke-RestMethod -Uri "${url}"\nInvoke-WebRequest -Uri ("${RESOURCE_API_BASE}" + $result.download_path) -OutFile $result.version.filename`
  if (codeLanguage.value === 'javascript') return `const resolved = await fetch('${url}').then(r => {\n  if (!r.ok) throw new Error('resolve failed')\n  return r.json()\n})\n\nconst downloadUrl = '${RESOURCE_API_BASE}' + resolved.download_path`
  return `curl --fail --location \\\n  "${url}"\n\n# 使用响应中的 download_path 下载，并按 sha256 校验文件`
})

async function runResolver() {
  debugLoading.value = true
  debugResult.value = null
  debugError.value = ''
  try {
    debugResult.value = await resourceApiRequest<ResolveResponse>(resolverPath.value)
  } catch (error) {
    debugError.value = friendlyError(error)
  } finally {
    debugLoading.value = false
  }
}

onMounted(async () => {
  await Promise.all([
    loadSummary(),
    loadProjects(props.initialCore.trim().toLowerCase() || 'paper'),
    props.adminMode ? loadMslStatus() : Promise.resolve(),
  ])
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
        <div><h1>{{ props.adminMode ? '资源管理' : '资源中心' }}</h1><p>{{ props.adminMode ? '远程上传、发布与维护资源站制品' : '浏览在线核心、插件与开发资源' }}</p></div>
      </div>
      <nav aria-label="资源中心导航">
        <button role="tab" :aria-selected="section === 'library'" :class="{ active: section === 'library' }" @click="setSection('library')"><Layers3 />资源库</button>
        <button v-if="props.adminMode" role="tab" :aria-selected="section === 'manage'" :class="{ active: section === 'manage' }" @click="setSection('manage')"><Package />版本管理</button>
        <button role="tab" :aria-selected="section === 'docs'" :class="{ active: section === 'docs' }" @click="setSection('docs')"><BookOpen />API 文档</button>
      </nav>
      <button class="mirror-icon-button" aria-label="刷新目录" title="刷新目录" @click="refreshAll"><RefreshCw :class="{ spin: loadingProjects }" /></button>
    </header>

    <div v-if="section !== 'docs'" class="mirror-catalog">
      <aside class="catalog-rail">
        <div class="catalog-controls">
          <div class="catalog-summary">
            <span><b>{{ summary?.core_projects ?? '—' }}</b><small>核心</small></span>
            <span><b>{{ summary?.plugin_projects ?? '—' }}</b><small>插件</small></span>
            <span><b>{{ summary?.skin_projects ?? '—' }}</b><small>皮肤</small></span>
            <span><b>{{ summary?.bbmodel_projects ?? '—' }}</b><small>模型</small></span>
            <span><b>{{ summary?.ui_texture_projects ?? '—' }}</b><small>UI</small></span>
            <span><b>{{ summary?.skill_projects ?? '—' }}</b><small>Skill</small></span>
            <span><b>{{ summary?.plugin_config_projects ?? '—' }}</b><small>配置</small></span>
          </div>
          <div class="catalog-kind" role="tablist" aria-label="资源类型">
            <button role="tab" :aria-selected="kind === 'core'" :class="{ active: kind === 'core' }" @click="switchKind('core')"><Server />核心库</button>
            <button role="tab" :aria-selected="kind === 'plugin'" :class="{ active: kind === 'plugin' }" @click="switchKind('plugin')"><Puzzle />插件库</button>
            <button role="tab" :aria-selected="kind === 'skin'" :class="{ active: kind === 'skin' }" @click="switchKind('skin')"><Shirt />皮肤库</button>
            <button role="tab" :aria-selected="kind === 'bbmodel'" :class="{ active: kind === 'bbmodel' }" @click="switchKind('bbmodel')"><Boxes />BBModel</button>
            <button role="tab" :aria-selected="kind === 'ui_texture'" :class="{ active: kind === 'ui_texture' }" @click="switchKind('ui_texture')"><ImageIcon />UI 贴图</button>
            <button role="tab" :aria-selected="kind === 'skill'" :class="{ active: kind === 'skill' }" @click="switchKind('skill')"><Bot />Skill 库</button>
            <button role="tab" :aria-selected="kind === 'plugin_config'" :class="{ active: kind === 'plugin_config' }" @click="switchKind('plugin_config')"><FileCog />插件配置</button>
          </div>
          <label class="catalog-search"><Search /><input v-model="search" aria-label="搜索资源" placeholder="搜索名称、作者或标签" @input="scheduleSearch"/></label>
          <label v-if="kindConfig.requiresMinecraft" class="catalog-filter"><Filter /><select v-model="minecraftFilter" @change="loadProjects('')"><option value="">全部 MC 版本</option><option v-for="item in minecraftOptions" :key="item" :value="item">Minecraft {{ item }}</option></select></label>
          <label v-if="kind === 'plugin'" class="catalog-filter"><Filter /><select v-model="pluginCategoryFilter" @change="loadProjects('')"><option value="">全部插件库（AI 优先顺序）</option><option v-for="item in PLUGIN_CATEGORIES" :key="item.value" :value="item.value">{{ item.label }}</option></select></label>
          <button v-if="section === 'manage'" class="catalog-create" @click="openProjectModal('create')"><Plus />新建{{ kindConfig.shortLabel }}项目</button>
          <div v-if="section === 'manage' && props.adminMode" class="local-admin-note"><ShieldAlert/><span><b>资源源站管理模式</b><small>写操作已携带当前会话的管理凭证。</small></span></div>
        </div>
        <div class="catalog-list" :class="{ loading: loadingProjects }">
          <button v-for="project in projects" :key="project.slug" :class="{ active: selectedSlug === project.slug }" @click="selectProject(project.slug)">
            <span class="project-mark" :class="{ preview: project.preview_url }" :style="{ '--project-color': project.color || '#32d5b0' }"><img v-if="project.preview_url" :src="project.preview_url" alt="" loading="lazy"/><Box v-else-if="kind === 'core'"/><Puzzle v-else-if="kind === 'plugin'"/><Shirt v-else-if="kind === 'skin'"/><Boxes v-else-if="kind === 'bbmodel'"/><Bot v-else-if="kind === 'skill'"/><FileCog v-else-if="kind === 'plugin_config'"/><ImageIcon v-else/></span>
            <span><b>{{ project.name }}</b><small><template v-if="kind === 'plugin'">{{ pluginCategoryLabel(project.plugin_category) }} · </template><template v-if="project.target_plugin">{{ project.target_plugin }} · </template>{{ project.published_versions ? project.latest_version : '暂无已发布版本' }} · {{ project.author }}</small></span>
            <ChevronRight />
          </button>
          <div v-if="loadingProjects" class="rail-state" role="status" aria-live="polite"><LoaderCircle class="spin"/>正在读取目录</div>
          <div v-else-if="!projects.length" class="rail-state"><Search/>没有匹配的资源</div>
        </div>
        <div v-if="section === 'manage' && !props.adminMode" class="local-admin-note"><ShieldAlert/><span><b>资源源站管理模式</b><small>CRUD 接口需要在远程反向代理层增加鉴权。</small></span></div>
      </aside>

      <main class="catalog-detail">
        <section v-if="props.adminMode && kind === 'core' && mslStatus" class="msl-sync-card">
          <div class="msl-sync-card__identity">
            <span><CloudDownload/></span>
            <div>
              <small>自动多镜像</small>
              <h2>核心库占位与补齐</h2>
              <p>每 2 小时先登记可用版本，再由 MSL、FastMirror、Polars 依次补齐；已有构建保持不变。</p>
            </div>
          </div>
          <div class="msl-sync-card__versions">
            <small>目标版本</small>
            <span><code v-for="item in mslStatus.target_versions" :key="item">{{ item }}</code></span>
          </div>
          <div class="msl-sync-card__status">
            <small v-if="mslStatus.last_finished_at">上次完成 {{ formatDate(mslStatus.last_finished_at) }}</small>
            <small v-else>尚未完成首次同步</small>
            <b v-if="mslStatus.last_error" class="sync-error">{{ mslStatus.last_error }}</b>
            <b v-if="mslStatus.last_finished_at">{{ mslStatus.matching_core_types }} 种核心 · 本轮补齐 {{ mslStatus.versions_resolved ?? mslStatus.versions_upserted }} 个 · 待补齐 {{ mslStatus.pending_versions ?? '—' }} 个</b>
            <b v-else>后台将在启动后自动检查缺失版本</b>
            <div class="mirror-source-links">
              <a href="https://www.mslmc.cn" target="_blank" rel="noreferrer">MSL <ExternalLink/></a>
              <a href="https://www.fastmirror.net/#/download/" target="_blank" rel="noreferrer">FastMirror <ExternalLink/></a>
              <a href="https://mirror.polars.cc/#/minecraft/core" target="_blank" rel="noreferrer">Polars <ExternalLink/></a>
            </div>
          </div>
          <button class="primary msl-sync-button" :disabled="mslSyncing || mslStatus.running || !mslStatus.enabled" @click="syncMslCores">
            <LoaderCircle v-if="mslSyncing || mslStatus.running" class="spin"/>
            <RefreshCw v-else/>
            {{ mslSyncing || mslStatus.running ? '正在检查缺失版本…' : '立即补齐缺失版本' }}
          </button>
        </section>
        <div v-if="pageError" class="mirror-error" role="alert"><CircleAlert/><span><b>无法读取远程资源目录</b><small>{{ pageError }}</small></span><button @click="refreshAll">重试</button></div>
        <template v-else-if="selectedProject">
          <header class="project-hero">
            <div class="project-identity">
              <span class="project-mark large" :class="{ preview: selectedProject.preview_url }" :style="{ '--project-color': selectedProject.color || '#32d5b0' }"><img v-if="selectedProject.preview_url" :src="selectedProject.preview_url" :alt="`${selectedProject.name} 预览`"/><Box v-else-if="kind === 'core'"/><Puzzle v-else-if="kind === 'plugin'"/><Shirt v-else-if="kind === 'skin'"/><Boxes v-else-if="kind === 'bbmodel'"/><Bot v-else-if="kind === 'skill'"/><FileCog v-else-if="kind === 'plugin_config'"/><ImageIcon v-else/></span>
              <div>
                <div><h2>{{ selectedProject.name }}</h2><code>{{ selectedProject.slug }}</code></div>
                <p>{{ selectedProject.summary }}</p>
                <footer><span>由 {{ selectedProject.author }} 维护</span><i/> <span>{{ selectedProject.published_versions }} 个已发布版本</span><i/> <span>{{ formatCount(selectedProject.downloads) }} 次下载</span></footer>
              </div>
            </div>
            <div class="project-actions" v-if="section === 'library'">
              <a v-if="selectedProject.homepage" :href="selectedProject.homepage" target="_blank" rel="noreferrer"><ExternalLink/>项目主页</a>
              <button class="primary" @click="copyText(RESOURCE_API_BASE + projectDownloadPath())"><Copy/>复制解析 API</button>
            </div>
            <div class="project-actions" v-else>
              <button class="primary" @click="openVersionModal('create')"><UploadCloud/>上传新版本</button>
              <button @click="openProjectModal('edit')"><Pencil/>编辑资料</button>
              <button class="danger" @click="askDeleteProject"><Trash2/>删除项目</button>
            </div>
          </header>

          <section class="project-description">
            <p>{{ selectedProject.description }}</p>
            <div><span v-if="kind === 'plugin'">{{ pluginCategoryLabel(selectedProject.plugin_category) }}插件库</span><span v-if="selectedProject.target_plugin">专属：{{ selectedProject.target_plugin }}</span><span v-if="selectedProject.license">{{ selectedProject.license }}</span><span v-for="tag in selectedProject.tags" :key="tag">{{ tag }}</span></div>
          </section>

          <section class="versions-section">
            <header>
              <div><h3>可用版本</h3><p>{{ kindConfig.description }}</p></div>
              <span class="version-controls">
                <select v-model="channelFilter"><option value="">全部渠道</option><option value="stable">Stable</option><option value="beta">Beta</option><option value="snapshot">Snapshot</option></select>
                <button v-if="section === 'manage'" class="primary" @click="openVersionModal('create')"><Plus/>新建版本</button>
              </span>
            </header>

            <div class="version-table-wrap">
              <table class="version-table">
                <thead><tr><th>版本 / 渠道</th><th>{{ kindConfig.requiresMinecraft ? 'Minecraft' : '格式' }}</th><th>{{ kind === 'core' ? 'Java' : kind === 'plugin' ? '平台' : '许可证' }}</th><th>{{ props.adminMode ? '制品 / 文件体积' : '制品' }}</th><th>发布时间</th><th>下载</th><th aria-label="操作"></th></tr></thead>
                <tbody>
                  <tr v-for="version in visibleVersions" :key="version.id" :class="{ muted: version.status !== 'published' }">
                    <td><b>{{ version.version }}</b><span><em class="channel" :class="version.channel">{{ version.channel }}</em><em class="status" :class="version.status">{{ version.status === 'published' ? '已发布' : version.status === 'draft' ? '草稿' : '已撤回' }}</em></span></td>
                    <td><span class="compat-list"><code v-for="item in (kindConfig.requiresMinecraft ? version.minecraft_versions : version.formats || []).slice(0, 2)" :key="item">{{ item }}</code><small v-if="(kindConfig.requiresMinecraft ? version.minecraft_versions : version.formats || []).length > 2">+{{ (kindConfig.requiresMinecraft ? version.minecraft_versions : version.formats || []).length - 2 }}</small></span></td>
                    <td><span v-if="kind === 'core'">{{ version.java_version ? `Java ${version.java_version}` : '—' }}</span><span v-else-if="kind === 'plugin'">{{ version.loaders.join(' / ') || 'Bukkit' }}</span><span v-else>{{ selectedProject.license || '未标注' }}</span></td>
                    <td>
                      <b class="artifact-name">{{ version.filename }}</b>
                      <small v-if="props.adminMode" class="artifact-meta">
                        <span class="artifact-size" :class="{ unknown: version.size <= 0 }">{{ version.size > 0 ? `体积 ${formatSize(version.size)}` : '体积待获取' }}</span>
                        <span class="artifact-sha">SHA {{ version.sha256 ? version.sha256.slice(0, 8) : '未提供' }}</span>
                      </small>
                      <small v-else>{{ formatSize(version.size) }} · SHA {{ version.sha256 ? version.sha256.slice(0, 8) : '未提供' }}</small>
                    </td>
                    <td><span>{{ formatDate(version.released_at) }}</span></td>
                    <td><span>{{ formatCount(version.downloads) }}</span></td>
                    <td>
                      <span v-if="section === 'manage'" class="row-actions"><button title="编辑版本" aria-label="编辑版本" @click="openVersionModal('edit', version)"><Pencil/></button><button class="danger" title="删除版本" aria-label="删除版本" @click="askDeleteVersion(version)"><Trash2/></button></span>
                      <span v-else class="row-actions"><button title="复制下载地址" aria-label="复制下载地址" @click="copyText(RESOURCE_API_BASE + versionDownloadPath(version))"><Copy/></button><button class="download" title="下载" aria-label="下载" :disabled="version.status !== 'published'" @click="downloadVersion(version)"><Download/></button></span>
                    </td>
                  </tr>
                </tbody>
              </table>
              <div v-if="loadingVersions" class="table-state" role="status" aria-live="polite"><LoaderCircle class="spin"/>正在读取版本</div>
              <div v-else-if="!visibleVersions.length" class="table-state"><Package/>当前筛选条件下没有版本</div>
            </div>
          </section>
        </template>
        <div v-else-if="!loadingProjects" class="catalog-empty"><Archive/><h2>选择一个资源项目</h2><p>从左侧资源库中选择项目，查看版本、格式与下载接口。</p></div>
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
        <a :href="resourceApiUrl('/api/openapi.json')" target="_blank" rel="noreferrer"><FileJson/>OpenAPI JSON<ExternalLink/></a>
      </aside>

      <main class="docs-content">
        <section v-if="docSection === 'quickstart'" class="doc-page">
          <header><h2>快速开始</h2><p>客户端通过独立资源 API 查询目录、解析版本，再从高带宽源站下载制品。</p></header>
          <div class="base-url"><span><small>RESOURCE API BASE URL</small><code>{{ RESOURCE_API_BASE }}/api/v1</code></span><button @click="copyText(RESOURCE_API_BASE + '/api/v1')"><Copy/>复制</button></div>
          <article><h3>解析并下载</h3><p>解析接口按资源类型、项目、渠道及可选 Minecraft 版本选择最新制品。响应包含文件名、格式、SHA-256 与稳定的 <code>download_path</code>。</p></article>
          <div class="code-sample">
            <header><span><button v-for="lang in (['curl','powershell','javascript'] as CodeLanguage[])" :key="lang" :class="{ active: codeLanguage === lang }" @click="codeLanguage=lang">{{ lang === 'powershell' ? 'PowerShell' : lang === 'javascript' ? 'JavaScript' : 'cURL' }}</button></span><button @click="copyText(codeSnippet)"><Clipboard/>复制代码</button></header>
            <pre><code>{{ codeSnippet }}</code></pre>
          </div>
          <article class="doc-callout"><Check/><span><b>支持独立部署</b><p>前端通过 <code>VITE_RESOURCE_API_BASE</code> 指向资源服务器；源站使用 <code>SCULK_ALLOWED_ORIGINS</code> 精确允许主站来源，并在反向代理层保护写接口。</p></span></article>
        </section>

        <section v-else-if="docSection === 'catalog'" class="doc-page">
          <header><h2>目录查询</h2><p>统一列出制品、Skill 与插件配置，可按关键词、兼容版本和插件分类筛选。</p></header>
          <article class="endpoint"><h3><em>GET</em><code>/api/catalog/{resource}</code></h3><p><code>{resource}</code> 可取 <code>cores</code>、<code>plugins</code>、<code>skins</code>、<code>bbmodels</code>、<code>ui-textures</code>、<code>skills</code> 或 <code>plugin-configs</code>。</p><table><tbody><tr><th>search</th><td>可选</td><td>匹配名称、作者、摘要或标签</td></tr><tr><th>minecraft</th><td>可选</td><td>只返回包含兼容版本的核心或插件</td></tr><tr><th>plugin_category</th><td>可选</td><td>主流、开源、普通或付费插件库</td></tr><tr><th>target_plugin</th><td>可选</td><td>筛选插件专属 Skill/配置</td></tr></tbody></table></article>
          <article class="endpoint"><h3><em>GET</em><code>/api/v1/plugins/search</code></h3><p>AI 插件发现专用接口，固定按照主流 → 开源 → 普通 → 付费排序。</p></article>
          <article class="endpoint"><h3><em>GET</em><code>/api/catalog/{resource}/{slug}/versions</code></h3><p>获取项目的完整版本历史；素材版本通过 <code>formats</code> 返回文件格式。</p></article>
        </section>

        <section v-else-if="docSection === 'resolve'" class="doc-page docs-split">
          <div>
            <header><h2>版本解析</h2><p>让客户端不必硬编码构建号、素材版本或源站文件地址。</p></header>
            <article class="endpoint"><h3><em>GET</em><code>/api/v1/resolve</code></h3><table><tbody><tr><th>kind</th><td>必填</td><td><code>core</code>、<code>plugin</code>、<code>skin</code>、<code>bbmodel</code>、<code>ui_texture</code>、<code>skill</code> 或 <code>plugin_config</code></td></tr><tr><th>project</th><td>必填</td><td>项目 slug，例如 <code>paper</code></td></tr><tr><th>minecraft</th><td>条件必填</td><td>核心与插件必填，其他资源可省略</td></tr><tr><th>channel</th><td>可选</td><td>默认 <code>stable</code></td></tr></tbody></table></article>
            <article><h3>响应策略</h3><p>只选择 <code>published</code> 状态与渠道匹配的记录；核心和插件额外匹配 Minecraft 版本，并按发布时间返回最新版本。</p></article>
          </div>
          <aside class="api-console">
            <header><span><i/>在线调试</span><small>连接资源 API</small></header>
            <div class="console-form"><label>资源类型<select :value="debugKind" @change="switchDebugKind(($event.target as HTMLSelectElement).value as CatalogKind)"><option v-for="item in RESOURCE_KINDS" :key="item.kind" :value="item.kind">{{ item.label }}</option></select></label><label>项目<select v-model="debugProject"><option v-for="project in docProjects[debugKind]" :key="project.slug" :value="project.slug">{{ project.name }}</option></select></label><label v-if="resourceKind(debugKind).requiresMinecraft">Minecraft<input v-model="debugMinecraft"/></label><label>渠道<select v-model="debugChannel"><option value="stable">stable</option><option value="beta">beta</option><option value="snapshot">snapshot</option></select></label><button :disabled="debugLoading || !debugProject" @click="runResolver"><LoaderCircle v-if="debugLoading" class="spin"/><Code2 v-else/>发送请求</button></div>
            <pre v-if="debugResult"><code>{{ JSON.stringify(debugResult, null, 2) }}</code></pre>
            <div v-else-if="debugError" class="console-error"><CircleAlert/>{{ debugError }}</div>
            <div v-else class="console-empty"><Code2/>请求结果会显示在这里</div>
          </aside>
        </section>

        <section v-else-if="docSection === 'download'" class="doc-page">
          <header><h2>文件下载</h2><p>通过稳定路径访问制品；外部对象返回 HTTP 307，内联 Skill/配置 bundle 直接返回文件内容。</p></header>
          <article class="endpoint"><h3><em>GET</em><code>/api/v1/download/{kind}/{project}/{version}</code></h3><p>例如：<code>/api/v1/download/core/paper/1.21.4-232</code>。客户端必须跟随重定向。</p></article>
          <article><h3>完整性校验</h3><p>先从解析响应读取 <code>version.sha256</code>，下载完成后再校验文件。若目录未提供校验和，开服器应提示风险，而不是静默执行 JAR。</p></article>
          <article class="doc-callout"><ShieldAlert/><span><b>支持源站对象直传</b><p>管理页上传文件后由服务端计算 SHA-256，并保存到 <code>/objects</code>；下载接口统计次数后跳转到 Caddy 静态对象服务，支持 Range、ETag 与断点续传。</p></span></article>
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
        <header><div><small>{{ projectModalMode === 'create' ? 'NEW PROJECT' : 'PROJECT SETTINGS' }}</small><h2 id="project-dialog-title">{{ projectModalMode === 'create' ? `新建${kindConfig.shortLabel}项目` : `编辑 ${selectedProject?.name}` }}</h2></div><button type="button" aria-label="关闭" @click="projectModal=false"><X/></button></header>
        <main class="mirror-form-grid">
          <label><span>项目标识 *</span><input v-model="projectForm.slug" required :disabled="projectModalMode === 'edit'" placeholder="paper" pattern="[a-z0-9][a-z0-9-]*"/></label>
          <label><span>显示名称 *</span><input v-model="projectForm.name" required placeholder="Paper"/></label>
          <label class="wide"><span>一句话摘要 *</span><input v-model="projectForm.summary" required placeholder="面向插件服务器的高性能 Minecraft 核心"/></label>
          <label class="wide"><span>详细说明 *</span><textarea v-model="projectForm.description" required rows="3"/></label>
          <label><span>维护者 *</span><input v-model="projectForm.author" required/></label>
          <label><span>主题色</span><div class="color-input"><input v-model="projectForm.color" type="color"/><input v-model="projectForm.color"/></div></label>
          <label><span>项目主页 *</span><input v-model="projectForm.homepage" required type="url" placeholder="https://..."/></label>
          <label><span>代码仓库（可选）</span><input v-model="projectForm.repository" type="url" placeholder="https://..."/></label>
          <label><span>预览图 URL</span><input v-model="projectForm.previewUrl" type="url" placeholder="https://resources.example.com/previews/..."/></label>
          <label><span>许可证</span><input v-model="projectForm.license" placeholder="CC BY 4.0 / 自定义授权"/></label>
          <label v-if="kind === 'plugin'"><span>插件库分类 *</span><select v-model="projectForm.pluginCategory"><option v-for="item in PLUGIN_CATEGORIES" :key="item.value" :value="item.value">{{ item.label }}</option></select></label>
          <label v-if="kind === 'skill' || kind === 'plugin_config'"><span>目标插件 slug *</span><input v-model="projectForm.targetPlugin" required placeholder="luckperms" pattern="[a-z0-9][a-z0-9-]*"/></label>
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
          <label v-if="kindConfig.requiresMinecraft"><span>Minecraft 版本 *（逗号分隔）</span><input v-model="versionForm.minecraftVersions" required placeholder="1.21.4, 1.21.3"/></label>
          <label v-if="kindConfig.requiresMinecraft"><span>{{ kindConfig.formatLabel }} *（逗号分隔）</span><input v-model="versionForm.loaders" required :placeholder="kindConfig.formatPlaceholder"/></label>
          <label v-else><span>{{ kindConfig.formatLabel }} *（逗号分隔）</span><input v-model="versionForm.formats" required :placeholder="kindConfig.formatPlaceholder"/></label>
          <label v-if="kind === 'core'"><span>Java 版本</span><input v-model="versionForm.javaVersion" type="number" min="8" max="99"/></label>
          <label><span>状态</span><select v-model="versionForm.status"><option value="draft">草稿</option><option value="published">已发布</option><option value="yanked">已撤回</option></select></label>
          <div class="wide artifact-upload" :class="{ dragging: artifactDragActive, selected: artifactFile }" @dragenter.prevent="artifactDragActive=true" @dragover.prevent="artifactDragActive=true" @dragleave.prevent="artifactDragActive=false" @drop.prevent="dropArtifact">
            <input id="artifact-file" class="artifact-file-input" type="file" @change="selectArtifact"/>
            <label for="artifact-file">
              <UploadCloud/>
              <span v-if="artifactFile"><b>{{ artifactFile.name }}</b><small>{{ formatSize(artifactFile.size) }} · 上传后自动计算 SHA-256</small></span>
              <span v-else><b>拖放文件到这里，或点击选择</b><small>支持 JAR、ZIP、PNG、BBModel 及其他资源文件</small></span>
            </label>
            <button v-if="artifactFile" type="button" @click="acceptArtifact(null)"><X/>移除</button>
          </div>
          <label><span>文件名 *</span><input v-model="versionForm.filename" required placeholder="paper-1.21.4-232.jar"/></label>
          <label><span>文件大小（MB）{{ versionForm.status === 'published' && kind !== 'skill' && kind !== 'plugin_config' && !artifactFile ? ' *' : '' }}</span><input v-model="versionForm.sizeMb" :required="versionForm.status === 'published' && kind !== 'skill' && kind !== 'plugin_config' && !artifactFile" type="number" min="0" step="0.01"/></label>
          <label class="wide"><span>上游下载地址{{ kind === 'skill' || kind === 'plugin_config' ? '（与内联内容或上传文件三选一）' : artifactFile ? '（已选择本机文件）' : ' *' }}</span><input v-model="versionForm.downloadUrl" :required="kind !== 'skill' && kind !== 'plugin_config' && !artifactFile" type="url" placeholder="https://.../artifact.jar"/></label>
          <label v-if="kind === 'skill' || kind === 'plugin_config'" class="wide"><span>内联资源内容（Skill bundle / 配置 JSON）</span><textarea v-model="versionForm.content" rows="8" class="mono" placeholder="总站自动生成时会在此保存可安装 bundle"/></label>
          <label class="wide"><span>SHA-256（上传文件或内联内容由服务端计算）{{ versionForm.status === 'published' && kind !== 'skill' && kind !== 'plugin_config' && !artifactFile ? ' *' : '' }}</span><input v-model="versionForm.sha256" :required="versionForm.status === 'published' && kind !== 'skill' && kind !== 'plugin_config' && !artifactFile" class="mono" minlength="64" maxlength="64"/></label>
          <label><span>发布时间 *</span><input v-model="versionForm.releasedAt" required type="datetime-local"/></label>
          <label class="wide"><span>更新说明 *</span><textarea v-model="versionForm.releaseNotes" required rows="3"/></label>
          <p v-if="versionFormError" class="form-error"><CircleAlert/>{{ versionFormError }}</p>
        </main>
        <footer><button type="button" @click="versionModal=false">取消</button><button class="primary" :disabled="saving"><LoaderCircle v-if="saving" class="spin"/><Check v-else/>{{ saving && artifactFile ? '正在上传并计算 SHA-256…' : artifactFile ? '上传并发布版本' : '保存版本' }}</button></footer>
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
