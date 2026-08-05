<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import {
  AlertTriangle,
  Check,
  FileCode2,
  LoaderCircle,
  Play,
  RefreshCw,
  SquareTerminal,
  Trash2,
  Wrench,
} from 'lucide-vue-next'
import { ApiError, apiRequest } from '../../lib/api'

type BuildTool = 'maven' | 'gradle'
type BuildAction = 'clean' | 'compile' | 'package' | 'build'

interface BuilderInfo {
  tool: BuildTool
  label: string
  source?: string | null
  available: boolean
  descriptor?: unknown
  wrapper?: unknown
}

interface BuildDiscoveryResponse {
  builders?: unknown
  detected_tool?: unknown
}

interface BuildResult {
  tool?: BuildTool
  action?: BuildAction
  command?: string
  success: boolean
  exit_code?: number | null
  duration_ms?: number | null
  output?: unknown
  output_truncated?: boolean
}

const props = defineProps<{
  serverId: string
}>()

const emit = defineEmits<{
  completed: [result: BuildResult]
}>()

const ACTIONS: Array<{ key: BuildAction; label: string; maven: string; gradle: string }> = [
  { key: 'clean', label: '清理', maven: '删除 target 目录', gradle: '删除 build 目录' },
  { key: 'compile', label: '编译', maven: '编译主源码', gradle: '编译 Java/Kotlin 源码' },
  { key: 'package', label: '打包', maven: '生成插件 JAR', gradle: '生成插件 JAR' },
  { key: 'build', label: '完整构建', maven: '执行完整打包流程', gradle: '执行完整构建检查' },
]

const builders = ref<BuilderInfo[]>([])
const detectedTool = ref<BuildTool | null>(null)
const selectedTool = ref<BuildTool>('maven')
const selectedAction = ref<BuildAction>('build')
const discoveryLoading = ref(false)
const discoveryError = ref('')
const runError = ref('')
const running = ref(false)
const result = ref<BuildResult | null>(null)
const toolManuallySelected = ref(false)
const discoveryEpoch = ref(0)
const runEpoch = ref(0)
let discoveryController: AbortController | null = null
let runController: AbortController | null = null

const availableBuilders = computed(() => builders.value.filter(builder => builder.available))
const selectedBuilder = computed(() => builders.value.find(builder => builder.tool === selectedTool.value) ?? null)
const selectedActionMeta = computed(() => ACTIONS.find(action => action.key === selectedAction.value) ?? ACTIONS[3])
const canRun = computed(() => Boolean(props.serverId.trim() && selectedBuilder.value?.available && !running.value))
const output = computed(() => outputText(result.value?.output))
const resultTool = computed(() => result.value?.tool ? toolLabel(result.value.tool) : '')
const resultAction = computed(() => result.value?.action ? actionLabel(result.value.action) : '')

function isBuildTool(value: unknown): value is BuildTool {
  return value === 'maven' || value === 'gradle'
}

function parseBuildTool(value: unknown): BuildTool | null {
  if (typeof value !== 'string') return null
  const normalized = value.trim().toLowerCase()
  return isBuildTool(normalized) ? normalized : null
}

function normalizeBuilders(value: unknown): BuilderInfo[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<BuildTool>()
  const normalized: BuilderInfo[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object') continue
    const raw = item as Record<string, unknown>
    const tool = parseBuildTool(raw.tool)
    if (!tool || seen.has(tool)) continue
    seen.add(tool)
    normalized.push({
      tool,
      label: typeof raw.label === 'string' && raw.label.trim() ? raw.label : toolLabel(tool),
      source: typeof raw.source === 'string' ? raw.source : null,
      available: raw.available === true,
      descriptor: raw.descriptor,
      wrapper: raw.wrapper,
    })
  }
  return normalized
}

function toolLabel(tool: BuildTool) {
  return tool === 'maven' ? 'Maven' : 'Gradle'
}

function actionLabel(action: BuildAction) {
  return ACTIONS.find(item => item.key === action)?.label ?? action
}

function actionIcon(action: BuildAction) {
  return action === 'clean' ? Trash2 : action === 'build' ? Wrench : FileCode2
}

function outputText(value: unknown) {
  if (typeof value === 'string') return value
  if (Array.isArray(value)) return value.map(item => String(item)).join('\n')
  if (value === null || value === undefined) return ''
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function artifactText(value: unknown) {
  if (typeof value === 'string') return value
  if (typeof value === 'boolean') return value ? '已检测' : '未检测'
  if (value && typeof value === 'object') {
    const raw = value as Record<string, unknown>
    for (const key of ['path', 'name', 'command', 'version']) {
      if (typeof raw[key] === 'string' && raw[key]) return raw[key] as string
    }
    if (typeof raw.exists === 'boolean') return raw.exists ? '已检测' : '未检测'
  }
  return '未提供'
}

function formatDuration(value: number | null | undefined) {
  if (value === null || value === undefined || !Number.isFinite(value)) return ''
  if (value < 1000) return `${Math.max(0, Math.round(value))} ms`
  return `${(value / 1000).toFixed(1)} s`
}

function errorText(error: unknown) {
  if (error instanceof ApiError) {
    try {
      const parsed = JSON.parse(error.message) as Record<string, unknown>
      for (const key of ['message', 'detail', 'error']) {
        if (typeof parsed[key] === 'string' && parsed[key].trim()) return parsed[key] as string
      }
    } catch {
      // The API may return a plain-text error body.
    }
    return error.message || `请求失败（HTTP ${error.status}）`
  }
  if (error instanceof DOMException && error.name === 'AbortError') return ''
  return error instanceof Error ? error.message : String(error)
}

function clearForWorkspace() {
  builders.value = []
  detectedTool.value = null
  selectedTool.value = 'maven'
  toolManuallySelected.value = false
  result.value = null
  discoveryError.value = ''
  runError.value = ''
}

function cancelRun() {
  runEpoch.value += 1
  runController?.abort()
  runController = null
  running.value = false
}

function resetForWorkspace() {
  discoveryEpoch.value += 1
  discoveryController?.abort()
  discoveryController = null
  discoveryLoading.value = false
  cancelRun()
  clearForWorkspace()
}

function chooseDefaultTool(nextBuilders: BuilderInfo[], detected: BuildTool | null) {
  const previous = toolManuallySelected.value
    ? nextBuilders.find(builder => builder.tool === selectedTool.value && builder.available)
    : null
  const preferred = detected ? nextBuilders.find(builder => builder.tool === detected && builder.available) : null
  const first = nextBuilders.find(builder => builder.available)
  selectedTool.value = (previous ?? preferred ?? first)?.tool ?? detected ?? nextBuilders[0]?.tool ?? 'maven'
}

async function loadBuilders() {
  const id = props.serverId.trim()
  discoveryController?.abort()
  const epoch = discoveryEpoch.value + 1
  discoveryEpoch.value = epoch
  if (!id) {
    clearForWorkspace()
    return
  }
  discoveryLoading.value = true
  discoveryError.value = ''
  try {
    const controller = new AbortController()
    discoveryController = controller
    const payload = await apiRequest<BuildDiscoveryResponse>(`/api/servers/${encodeURIComponent(id)}/build`, { signal: controller.signal })
    if (epoch !== discoveryEpoch.value || id !== props.serverId.trim()) return
    builders.value = normalizeBuilders(payload.builders)
    detectedTool.value = parseBuildTool(payload.detected_tool)
    chooseDefaultTool(builders.value, detectedTool.value)
  } catch (error) {
    if (epoch === discoveryEpoch.value) discoveryError.value = errorText(error)
  } finally {
    if (epoch === discoveryEpoch.value) {
      discoveryLoading.value = false
      discoveryController = null
    }
  }
}

async function runBuild(action: BuildAction = selectedAction.value) {
  const id = props.serverId.trim()
  const tool = selectedBuilder.value?.tool
  if (!id || !tool || !selectedBuilder.value?.available || running.value) return
  selectedAction.value = action
  runError.value = ''
  running.value = true
  result.value = null
  const epoch = runEpoch.value + 1
  runEpoch.value = epoch
  const controller = new AbortController()
  runController?.abort()
  runController = controller
  try {
    const payload = await apiRequest<BuildResult>(`/api/servers/${encodeURIComponent(id)}/build`, {
      method: 'POST',
      body: JSON.stringify({ tool, action }),
      signal: controller.signal,
    })
    if (epoch !== runEpoch.value || id !== props.serverId.trim()) return
    result.value = payload
    if (!payload.success) runError.value = '构建失败，请查看下方输出。'
    emit('completed', payload)
  } catch (error) {
    if (epoch === runEpoch.value) runError.value = errorText(error)
  } finally {
    if (epoch === runEpoch.value) {
      running.value = false
      runController = null
    }
  }
}

function selectTool(tool: BuildTool) {
  const builder = builders.value.find(item => item.tool === tool)
  if (!builder || !builder.available || running.value) return
  selectedTool.value = tool
  toolManuallySelected.value = true
  runError.value = ''
}

watch(() => props.serverId, () => {
  resetForWorkspace()
  void loadBuilders()
}, { immediate: true })
onUnmounted(() => {
  discoveryEpoch.value += 1
  runEpoch.value += 1
  discoveryController?.abort()
  runController?.abort()
})
</script>

<template>
  <section class="project-build-manager" aria-labelledby="project-build-title">
    <header class="build-manager-header">
      <div>
        <span class="build-manager-kicker">PROJECT BUILD</span>
        <h2 id="project-build-title">Maven / Gradle 构建</h2>
        <p>在当前插件项目目录中检测并执行构建任务。</p>
      </div>
      <button class="build-icon-button" type="button" title="刷新构建器检测" :disabled="discoveryLoading || running || !serverId" @click="loadBuilders">
        <LoaderCircle v-if="discoveryLoading" class="spin" />
        <RefreshCw v-else />
      </button>
    </header>

    <div v-if="discoveryError" class="build-alert error" role="alert">
      <AlertTriangle />
      <span>{{ discoveryError }}</span>
      <button type="button" title="重试构建器检测" @click="loadBuilders"><RefreshCw /></button>
    </div>
    <div v-else-if="discoveryLoading" class="build-state"><LoaderCircle class="spin" /><span>正在检测 Maven 与 Gradle…</span></div>
    <div v-else-if="!serverId" class="build-state"><SquareTerminal /><span>请先选择一个项目工作区。</span></div>
    <div v-else-if="!builders.length || !availableBuilders.length" class="build-state empty">
      <Wrench />
      <b>尚未检测到可用构建器</b>
      <span>请在项目中加入 `pom.xml`、`build.gradle` 或对应 Wrapper 后刷新。</span>
    </div>
    <template v-else>
      <section class="builder-section" aria-label="构建器选择">
        <div class="builder-tabs" role="tablist" aria-label="选择构建器">
          <button
            v-for="builder in builders"
            :key="builder.tool"
            class="builder-tab"
            :class="{ active: selectedTool === builder.tool, unavailable: !builder.available }"
            type="button"
            role="tab"
            :aria-selected="selectedTool === builder.tool"
            :disabled="!builder.available || running"
            @click="selectTool(builder.tool)"
          >
            <span class="builder-mark">{{ builder.tool === 'maven' ? 'M' : 'G' }}</span>
            <span><b>{{ builder.label }}</b><small>{{ builder.available ? (detectedTool === builder.tool ? '已检测 · 推荐' : '可用') : '不可用' }}</small></span>
            <Check v-if="selectedTool === builder.tool && builder.available" />
          </button>
        </div>
        <div v-if="selectedBuilder" class="builder-meta">
          <span v-if="selectedBuilder.source"><small>执行器</small><code>{{ selectedBuilder.source }}</code></span>
          <span><small>描述文件</small><code>{{ artifactText(selectedBuilder.descriptor) }}</code></span>
          <span><small>Wrapper</small><code>{{ artifactText(selectedBuilder.wrapper) }}</code></span>
        </div>
      </section>

      <section class="action-section" aria-label="构建动作">
        <header><div><b>构建动作</b><small>{{ selectedActionMeta[selectedTool] }}</small></div><span v-if="detectedTool === selectedTool" class="detected-badge">自动选择</span></header>
        <div class="action-grid">
          <button
            v-for="action in ACTIONS"
            :key="action.key"
            class="action-button"
            :class="{ selected: selectedAction === action.key }"
            type="button"
            :disabled="!canRun"
            @click="runBuild(action.key)"
          >
            <component :is="actionIcon(action.key)" />
            <span><b>{{ action.label }}</b><small>{{ action[selectedTool] }}</small></span>
            <LoaderCircle v-if="running && selectedAction === action.key" class="spin action-state" />
            <Play v-else class="action-state" />
          </button>
        </div>
      </section>

      <div v-if="running" class="build-status running" role="status"><LoaderCircle class="spin" /><span>正在执行 {{ toolLabel(selectedTool) }} {{ actionLabel(selectedAction) }}…</span></div>
      <div v-if="runError" class="build-alert error" role="alert"><AlertTriangle /><span>{{ runError }}</span></div>
      <section v-if="result" class="result-section" :class="{ success: result.success, failure: !result.success }" aria-live="polite">
        <header>
          <span class="result-title"><Check v-if="result.success" /><AlertTriangle v-else /><b>{{ result.success ? '构建完成' : '构建失败' }}</b></span>
          <span class="result-meta"><template v-if="resultTool">{{ resultTool }} · </template><template v-if="resultAction">{{ resultAction }}</template><template v-if="formatDuration(result.duration_ms)"> · {{ formatDuration(result.duration_ms) }}</template><template v-if="result.exit_code !== undefined && result.exit_code !== null"> · 退出码 {{ result.exit_code }}</template></span>
        </header>
        <code v-if="result.command" class="build-command">{{ result.command }}</code>
        <pre v-if="output">{{ output }}</pre>
        <small v-if="result.output_truncated" class="truncation-note">输出过长，已截断显示。</small>
      </section>
    </template>
  </section>
</template>

<style scoped>
.project-build-manager{min-height:0;display:flex;flex:1;flex-direction:column;gap:12px;overflow:auto;padding:18px;background:#0d1319;color:#dfe6eb;scrollbar-width:thin}
.build-manager-header{display:flex;align-items:flex-start;justify-content:space-between;gap:14px;padding-bottom:13px;border-bottom:1px solid rgba(255,255,255,.075)}
.build-manager-kicker{display:block;color:#6b7885;font-size:9px;font-weight:700;letter-spacing:.12em}
.build-manager-header h2{margin:5px 0 0;font-size:17px;line-height:1.25}
.build-manager-header p{margin:6px 0 0;color:#75818d;font-size:10px;line-height:1.55}
.build-icon-button{width:31px;height:31px;display:grid;place-items:center;flex:none;padding:0;border:1px solid rgba(255,255,255,.09);border-radius:7px;color:#8b98a4;background:rgba(255,255,255,.025)}
.build-icon-button:hover:not(:disabled){border-color:rgba(50,213,176,.26);color:#9ee6d4;background:rgba(50,213,176,.07)}
.build-icon-button:disabled{opacity:.52;cursor:wait}
.build-icon-button svg{width:15px}
.builder-section,.action-section,.result-section{padding:13px;border:1px solid rgba(255,255,255,.075);border-radius:9px;background:#111820}
.builder-tabs{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}
.builder-tab{min-width:0;display:flex;align-items:center;gap:8px;padding:9px;border:1px solid rgba(255,255,255,.08);border-radius:7px;color:#9ca8b3;background:#0e141a;text-align:left}
.builder-tab:hover:not(:disabled){border-color:rgba(50,213,176,.25);background:rgba(50,213,176,.045)}
.builder-tab.active{border-color:rgba(50,213,176,.34);color:#d8f6ed;background:rgba(50,213,176,.085);box-shadow:inset 2px 0 #32d5b0}
.builder-tab.unavailable{opacity:.5}
.builder-tab:disabled{cursor:not-allowed}
.builder-tab>span:nth-child(2){display:flex;min-width:0;flex:1;flex-direction:column;gap:3px}
.builder-tab b{overflow:hidden;font-size:10px;text-overflow:ellipsis;white-space:nowrap}
.builder-tab small{color:#6c7985;font-size:8px}
.builder-tab.active small{color:#7ac9b6}
.builder-tab>svg{width:13px;flex:none;color:#32d5b0}
.builder-mark{width:25px;height:25px;display:grid;place-items:center;flex:none;border:1px solid rgba(156,140,255,.2);border-radius:6px;color:#c3baff;background:rgba(156,140,255,.08);font:700 12px Inter,sans-serif}
.builder-tab.active .builder-mark{border-color:rgba(50,213,176,.26);color:#8fe1cd;background:rgba(50,213,176,.08)}
.builder-meta{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:7px;margin-top:10px}
.builder-meta>span{min-width:0;display:flex;flex-direction:column;gap:4px;padding:7px 8px;border:1px solid rgba(255,255,255,.06);border-radius:6px;background:#0d1318}
.builder-meta small{color:#64717e;font-size:8px}
.builder-meta code{overflow:hidden;color:#a7b6c0;font:9px 'Cascadia Code',Consolas,monospace;text-overflow:ellipsis;white-space:nowrap}
.action-section>header{display:flex;align-items:flex-start;justify-content:space-between;gap:8px;margin-bottom:9px}
.action-section>header>div{display:flex;flex-direction:column;gap:4px}
.action-section header b{font-size:11px}
.action-section header small{color:#697783;font-size:8px}
.detected-badge{padding:3px 6px;border:1px solid rgba(50,213,176,.18);border-radius:5px;color:#81d9c3;background:rgba(50,213,176,.07);font-size:8px}
.action-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:7px}
.action-button{min-width:0;display:flex;align-items:center;gap:7px;padding:9px;border:1px solid rgba(255,255,255,.075);border-radius:7px;color:#aab5be;background:#0e141a;text-align:left}
.action-button:hover:not(:disabled),.action-button.selected{border-color:rgba(50,213,176,.25);background:rgba(50,213,176,.06)}
.action-button:disabled{opacity:.48;cursor:not-allowed}
.action-button>svg:first-child{width:15px;flex:none;color:#80d5c0}
.action-button>span{display:flex;min-width:0;flex:1;flex-direction:column;gap:3px}
.action-button b{font-size:10px}
.action-button small{overflow:hidden;color:#687681;font-size:8px;text-overflow:ellipsis;white-space:nowrap}
.action-state{width:13px!important;flex:none;color:#64717e!important}
.action-button.selected .action-state{color:#32d5b0!important}
.build-state{display:flex;align-items:center;justify-content:center;gap:8px;min-height:150px;padding:24px;border:1px dashed rgba(255,255,255,.1);border-radius:9px;color:#788591;font-size:10px;text-align:center}
.build-state svg{width:18px;flex:none}
.build-state.empty{flex-direction:column;gap:7px}
.build-state.empty>svg{width:28px;color:#72808d}
.build-state.empty b{color:#b7c2cb;font-size:12px}
.build-state.empty span{max-width:350px;color:#6c7985;font-size:9px;line-height:1.6}
.build-alert{display:flex;align-items:center;gap:8px;padding:9px 10px;border:1px solid rgba(226,92,101,.22);border-radius:7px;color:#e18a8f;background:rgba(226,92,101,.055);font-size:9px;line-height:1.5}
.build-alert>svg{width:15px;flex:none}
.build-alert>span{min-width:0;flex:1;overflow-wrap:anywhere}
.build-alert button{width:24px;height:24px;display:grid;place-items:center;flex:none;padding:0;border:0;border-radius:5px;color:inherit;background:transparent}
.build-alert button:hover{background:rgba(255,255,255,.07)}
.build-alert button svg{width:13px}
.build-status{display:flex;align-items:center;gap:8px;padding:8px 10px;border:1px solid rgba(50,213,176,.18);border-radius:7px;color:#8ddcca;background:rgba(50,213,176,.055);font-size:9px}
.build-status svg{width:14px}
.result-section{border-color:rgba(50,213,176,.2);background:rgba(50,213,176,.035)}
.result-section.failure{border-color:rgba(226,92,101,.22);background:rgba(226,92,101,.035)}
.result-section>header{display:flex;align-items:center;justify-content:space-between;gap:8px}
.result-title{display:flex;align-items:center;gap:6px;color:#8fe1ce}
.failure .result-title{color:#e38c92}
.result-title svg{width:14px}
.result-title b{font-size:10px}
.result-meta{color:#687681;font-size:8px;text-align:right}
.build-command{display:block;overflow:auto;margin-top:9px;padding:7px 8px;border-radius:5px;color:#9db7af;background:#0b1116;font:9px 'Cascadia Code',Consolas,monospace;white-space:pre}
.result-section pre{max-height:330px;overflow:auto;margin:8px 0 0;padding:9px;border:1px solid rgba(255,255,255,.06);border-radius:6px;color:#b1c0c8;background:#0b1116;font:9px/1.65 'Cascadia Code',Consolas,monospace;white-space:pre-wrap;overflow-wrap:anywhere}
.truncation-note{display:block;margin-top:6px;color:#c6a36c;font-size:8px}
.spin{animation:project-build-spin .85s linear infinite}
@keyframes project-build-spin{to{transform:rotate(360deg)}}
@media(max-width:620px){.project-build-manager{padding:14px}.builder-meta{grid-template-columns:1fr}.action-grid{grid-template-columns:1fr}.result-section>header{align-items:flex-start;flex-direction:column}.result-meta{text-align:left}}
</style>
