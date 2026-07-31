<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import {
  Activity,
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronUp,
  CircleStop,
  Clock3,
  Download,
  FileText,
  LoaderCircle,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Square,
} from 'lucide-vue-next'
import {
  TASK_STATUS_LABELS,
  taskCanApprove,
  taskCanCancel,
  taskCanRollback,
  type TaskInfo,
  type TaskRisk,
} from '../features/automation/types'
import { apiRequest, apiUrl } from '../lib/api'

const props = defineProps<{
  serverId: string
  tasks: TaskInfo[]
  focusedTaskId?: string
}>()

const emit = defineEmits<{
  taskUpdated: [task: TaskInfo]
  refreshRequested: []
}>()

const creatingKind = ref('')
const busyTaskIds = ref<string[]>([])
const expandedTaskIds = ref<string[]>([])
const notice = ref('')
const noticeLevel = ref<'info' | 'error'>('info')

const visibleTasks = computed(() => props.tasks.filter((task) => task.server_id === props.serverId || task.server_id === 'all'))
const awaitingApproval = computed(() => visibleTasks.value.filter((task) => task.status === 'awaiting_approval').length)
const active = computed(() => visibleTasks.value.filter((task) => ['queued', 'running', 'cancelling'].includes(task.status)).length)
const failed = computed(() => visibleTasks.value.filter((task) => ['failed', 'interrupted', 'rollback_failed'].includes(task.status)).length)

function isBusy(taskId: string) {
  return busyTaskIds.value.includes(taskId)
}

function setTaskBusy(taskId: string, busy: boolean) {
  busyTaskIds.value = busy
    ? [...new Set([...busyTaskIds.value, taskId])]
    : busyTaskIds.value.filter((id) => id !== taskId)
}

function showNotice(message: string, level: 'info' | 'error' = 'info') {
  notice.value = message
  noticeLevel.value = level
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

async function create(kind: 'diagnostic' | 'server_start' | 'server_stop', title: string, risk: TaskRisk) {
  creatingKind.value = kind
  showNotice('')
  try {
    const task = await apiRequest<TaskInfo>('/api/automation/tasks', {
      method: 'POST',
      body: JSON.stringify({ server_id: props.serverId, title, kind, risk }),
    })
    emit('taskUpdated', task)
    expandedTaskIds.value = [...new Set([task.id, ...expandedTaskIds.value])]
    showNotice(task.status === 'awaiting_approval' ? '执行计划已保存，等待你批准。' : '执行计划已进入队列。')
    emit('refreshRequested')
  } catch (error) {
    showNotice(errorMessage(error), 'error')
  } finally {
    creatingKind.value = ''
  }
}

async function act(task: TaskInfo, action: 'approve' | 'cancel' | 'rollback') {
  if (action === 'rollback' && !window.confirm('回滚会创建一条新的补偿任务，将服务器恢复到原任务执行前的运行状态。继续吗？')) return
  setTaskBusy(task.id, true)
  showNotice('')
  try {
    const updated = await apiRequest<TaskInfo>(`/api/tasks/${task.id}/${action}`, { method: 'POST' })
    emit('taskUpdated', updated)
    if (action === 'rollback') expandedTaskIds.value = [...new Set([updated.id, ...expandedTaskIds.value])]
    showNotice(action === 'approve' ? '任务已批准并进入执行队列。' : action === 'cancel' ? '取消请求已提交。' : '补偿任务已创建。')
    emit('refreshRequested')
  } catch (error) {
    showNotice(errorMessage(error), 'error')
    emit('refreshRequested')
  } finally {
    setTaskBusy(task.id, false)
  }
}

function toggleDetails(taskId: string) {
  expandedTaskIds.value = expandedTaskIds.value.includes(taskId)
    ? expandedTaskIds.value.filter((id) => id !== taskId)
    : [...expandedTaskIds.value, taskId]
}

function isExpanded(taskId: string) {
  return expandedTaskIds.value.includes(taskId)
}

function focusTask(taskId?: string) {
  if (!taskId) return
  expandedTaskIds.value = [...new Set([taskId, ...expandedTaskIds.value])]
  nextTick(() => document.getElementById(`task-${taskId}`)?.scrollIntoView({ block: 'center', behavior: 'smooth' }))
}

function riskLabel(risk: string) {
  return risk === 'high' ? '高风险' : risk === 'medium' ? '中风险' : '低风险'
}

function kindLabel(kind: string) {
  const labels: Record<string, string> = {
    diagnostic: '日志诊断',
    server_provision: '首次初始化',
    bootstrap: '旧版初始化',
    server_start: '启动服务器',
    server_stop: '停止服务器',
    rollback_server_state: '状态补偿',
    download: '核心下载',
    core_download: '核心下载',
  }
  return labels[kind] ?? kind
}

function rollbackLabel(status: string) {
  const labels: Record<string, string> = {
    prepared: '补偿信息已准备',
    available: '可回滚',
    scheduled: '补偿任务已创建',
    planned: '准备恢复',
    completed: '补偿已完成',
    failed: '补偿失败',
  }
  return labels[status] ?? status
}

function formatDate(value?: string | null) {
  if (!value) return '—'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('zh-CN', { hour12: false })
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

watch(() => props.focusedTaskId, focusTask, { immediate: true })
onMounted(() => focusTask(props.focusedTaskId))
</script>

<template>
  <div class="module-scroll">
    <section class="module-hero">
      <div><span><Activity/></span><p><small>TASK EXECUTOR</small><b>真实任务执行器</b><em>计划、批准、执行、日志、产物与补偿状态统一记录。</em></p></div>
      <i><ShieldCheck/>状态来自后端持久化记录</i>
    </section>

    <div class="stats">
      <article><span class="mint"><Play/></span><p><small>执行队列</small><b>{{ active }}</b><em>排队、执行或取消中</em></p></article>
      <article><span class="amber"><AlertTriangle/></span><p><small>等待批准</small><b>{{ awaitingApproval }}</b><em>批准前不会执行</em></p></article>
      <article><span class="red"><CircleStop/></span><p><small>异常任务</small><b>{{ failed }}</b><em>失败、中断或回滚失败</em></p></article>
    </div>

    <section class="quick-card">
      <header>
        <div><small>已接入执行器</small><b>创建服务器任务</b></div>
        <span v-if="notice" :class="noticeLevel">{{ notice }}</span>
      </header>
      <div class="quick-grid">
        <button :disabled="Boolean(creatingKind)" @click="create('diagnostic', '分析最近服务器日志', 'low')">
          <LoaderCircle v-if="creatingKind === 'diagnostic'" class="spin"/><FileText v-else/>
          <span><b>日志诊断</b><small>只读分析最近日志并生成报告</small></span><em class="low">低风险</em>
        </button>
        <button :disabled="Boolean(creatingKind)" @click="create('server_start', '启动服务器并等待就绪', 'medium')">
          <LoaderCircle v-if="creatingKind === 'server_start'" class="spin"/><Play v-else/>
          <span><b>启动服务器</b><small>等待真实就绪标记后完成</small></span><em>中风险</em>
        </button>
        <button :disabled="Boolean(creatingKind)" @click="create('server_stop', '安全停止服务器并确认退出', 'high')">
          <LoaderCircle v-if="creatingKind === 'server_stop'" class="spin"/><Square v-else/>
          <span><b>安全停服</b><small>等待服务器进程实际退出</small></span><em class="high">高风险</em>
        </button>
      </div>
    </section>

    <section class="task-card">
      <header>
        <div><small>执行记录</small><b>任务、事件与产物</b></div>
        <button class="refresh" title="刷新任务状态" @click="emit('refreshRequested')"><RefreshCw/>刷新</button>
      </header>
      <div v-if="!visibleTasks.length" class="empty-state"><Clock3/><b>暂无执行任务</b><small>从上方创建诊断、启动或停止任务。</small></div>
      <article
        v-for="task in visibleTasks"
        :id="`task-${task.id}`"
        :key="task.id"
        class="task-row"
        :class="[task.status, { focused: task.id === focusedTaskId }]"
      >
        <div class="task-summary" @click="toggleDetails(task.id)">
          <span class="task-icon" :class="task.status">
            <Check v-if="task.status === 'completed'"/>
            <LoaderCircle v-else-if="['running', 'cancelling'].includes(task.status)" class="spin"/>
            <AlertTriangle v-else-if="['failed', 'interrupted', 'rollback_failed'].includes(task.status)"/>
            <CircleStop v-else-if="task.status === 'cancelled'"/>
            <Clock3 v-else/>
          </span>
          <div class="task-main">
            <div><b>{{ task.title }}</b><i :class="task.risk">{{ riskLabel(task.risk) }}</i></div>
            <small>{{ kindLabel(task.kind) }} · {{ formatDate(task.created_at) }}</small>
            <div class="bar"><i :class="task.status" :style="{ width: `${task.progress}%` }"/></div>
          </div>
          <em class="task-status" :class="task.status">{{ TASK_STATUS_LABELS[task.status] ?? task.status }}</em>
          <div class="actions" @click.stop>
            <button v-if="taskCanApprove(task)" class="approve" :disabled="isBusy(task.id)" @click="act(task, 'approve')"><Check/>批准</button>
            <button v-if="taskCanCancel(task)" :disabled="isBusy(task.id)" @click="act(task, 'cancel')"><CircleStop/>取消</button>
            <button v-if="taskCanRollback(task)" class="rollback" :disabled="isBusy(task.id)" @click="act(task, 'rollback')"><RotateCcw/>回滚</button>
            <button class="details" :title="isExpanded(task.id) ? '收起详情' : '展开详情'" @click="toggleDetails(task.id)"><ChevronUp v-if="isExpanded(task.id)"/><ChevronDown v-else/></button>
          </div>
        </div>

        <div v-if="isExpanded(task.id)" class="task-details">
          <div class="result-grid">
            <p><small>创建</small><b>{{ formatDate(task.created_at) }}</b></p>
            <p><small>开始</small><b>{{ formatDate(task.started_at) }}</b></p>
            <p><small>结束</small><b>{{ formatDate(task.finished_at) }}</b></p>
            <p><small>批准者</small><b>{{ task.approved_by || '—' }}</b></p>
          </div>
          <p v-if="task.summary" class="task-result"><Check/>{{ task.summary }}</p>
          <p v-if="task.error" class="task-error"><AlertTriangle/>{{ task.error }}</p>
          <p v-if="task.rollback" class="rollback-state"><RotateCcw/>{{ rollbackLabel(task.rollback.status) }}<span v-if="task.rollback.summary"> · {{ task.rollback.summary }}</span></p>

          <section v-if="task.events?.length" class="event-list">
            <header><b>执行事件</b><small>最近 {{ Math.min(task.events.length, 30) }} 条</small></header>
            <p v-for="(event, index) in task.events.slice(-30).reverse()" :key="`${event.at}-${index}`" :class="event.level">
              <time>{{ formatDate(event.at) }}</time><i/> <span>{{ event.message }}</span>
            </p>
          </section>

          <section v-if="task.artifacts?.length" class="artifact-list">
            <header><b>任务产物</b><small>{{ task.artifacts.length }} 个文件</small></header>
            <a v-for="artifact in task.artifacts" :key="artifact.id" :href="apiUrl(`/api/tasks/${task.id}/artifacts/${artifact.id}`)" download>
              <FileText/><span><b>{{ artifact.name }}</b><small>{{ artifact.kind }} · {{ formatBytes(artifact.size) }}</small></span><Download/>
            </a>
          </section>
        </div>
      </article>
    </section>
  </div>
</template>

<style scoped>
.module-scroll{flex:1;overflow:auto;padding:18px;color:#e8edf2}.module-hero,.quick-card,.task-card{border:1px solid rgba(255,255,255,.075);border-radius:11px;background:#11161c}.module-hero{display:flex;align-items:center;justify-content:space-between;padding:18px;background:linear-gradient(120deg,rgba(50,213,176,.07),#11161c 48%)}.module-hero>div{display:flex;align-items:center;gap:12px}.module-hero>div>span{width:43px;height:43px;display:grid;place-items:center;border-radius:10px;color:#32d5b0;background:rgba(50,213,176,.1)}.module-hero svg{width:20px}.module-hero p{display:flex;flex-direction:column;margin:0}.module-hero small,.quick-card small,.task-card small{color:#66727f;font-size:8px}.module-hero b{margin-top:4px;font-size:15px}.module-hero em{margin-top:5px;color:#6c7885;font:normal 8px Inter}.module-hero>i{display:flex;align-items:center;gap:6px;color:#7bd8c2;font:normal 8px Inter}.module-hero>i svg{width:14px}.stats{display:grid;grid-template-columns:repeat(3,1fr);gap:9px;margin-top:10px}.stats article{display:flex;align-items:center;gap:10px;padding:13px;border:1px solid rgba(255,255,255,.075);border-radius:9px;background:#12171d}.stats article>span{width:32px;height:32px;display:grid;place-items:center;border-radius:8px}.stats svg{width:16px}.mint{color:#32d5b0;background:rgba(50,213,176,.09)}.amber{color:#f3a75c;background:rgba(243,167,92,.09)}.red{color:#ff858b;background:rgba(255,107,114,.08)}.stats p{display:flex;flex-direction:column;margin:0}.stats small{color:#697582;font-size:8px}.stats b{margin-top:2px;font-size:14px}.stats em{color:#56616e;font:normal 7px Inter}.quick-card,.task-card{margin-top:10px;padding:16px}.quick-card>header,.task-card>header{display:flex;align-items:center;justify-content:space-between;margin-bottom:13px}.quick-card header div,.task-card header div{display:flex;flex-direction:column}.quick-card header b,.task-card header b{margin-top:4px;font-size:12px}.quick-card header>span{max-width:55%;color:#7ddbc5;font-size:8px;text-align:right}.quick-card header>span.error{color:#ff878c}.quick-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:8px}.quick-grid button{display:flex;align-items:center;gap:9px;padding:11px;border:1px solid rgba(255,255,255,.07);border-radius:8px;color:#8793a0;background:#0e1318;text-align:left}.quick-grid button:hover:not(:disabled){border-color:rgba(50,213,176,.2);background:rgba(50,213,176,.035)}.quick-grid button:disabled,.actions button:disabled{cursor:not-allowed;opacity:.5}.quick-grid button>svg{width:17px;color:#32d5b0}.quick-grid button>span{display:flex;min-width:0;flex:1;flex-direction:column}.quick-grid b{color:#cbd2da;font-size:9px}.quick-grid small{margin-top:4px;font-size:7px}.quick-grid em{padding:3px 5px;border-radius:5px;color:#e5aa70;background:rgba(243,167,92,.08);font:normal 7px Inter}.quick-grid em.low{color:#6dd2b9;background:rgba(50,213,176,.08)}.quick-grid em.high{color:#ff858b;background:rgba(255,107,114,.08)}.refresh{display:flex;align-items:center;gap:5px;border:0;color:#74808d;background:transparent;font-size:8px}.refresh:hover{color:#b9c3cd}.refresh svg{width:12px}.task-card{padding-bottom:8px}.task-row{border-top:1px solid rgba(255,255,255,.055);transition:.2s}.task-row.focused{margin:0 -8px;padding:0 8px;border-radius:8px;background:rgba(50,213,176,.04);box-shadow:inset 2px 0 #32d5b0}.task-summary{display:flex;align-items:center;gap:10px;padding:11px 0;cursor:pointer}.task-icon{width:28px;height:28px;display:grid;place-items:center;flex:0 0 auto;border-radius:7px;color:#8b96a3;background:#1b222a}.task-icon.running,.task-icon.cancelling{color:#32d5b0;background:rgba(50,213,176,.09)}.task-icon.completed{color:#74d487;background:rgba(83,190,100,.09)}.task-icon.failed,.task-icon.interrupted,.task-icon.rollback_failed{color:#ff858b;background:rgba(255,107,114,.08)}.task-icon svg{width:14px}.task-main{min-width:0;flex:1}.task-main>div:first-child{display:flex;align-items:center;gap:7px}.task-main b{font-size:9px}.task-main>small{display:block;margin-top:4px;text-transform:none}.task-main>div>i{padding:2px 4px;border-radius:4px;color:#72d6be;background:rgba(50,213,176,.08);font:normal 6px Inter}.task-main>div>i.medium{color:#ebb078;background:rgba(243,167,92,.08)}.task-main>div>i.high{color:#ff878c;background:rgba(255,107,114,.08)}.bar{height:3px;margin-top:7px;border-radius:4px;background:#252c34;overflow:hidden}.bar i{display:block;height:100%;border-radius:inherit;background:#32d5b0;transition:width .25s}.bar i.failed,.bar i.interrupted,.bar i.rollback_failed{background:#e96870}.bar i.cancelled{background:#64707c}.task-status{min-width:68px;color:#7f8b97;font:normal 8px Inter;text-align:center}.task-status.failed,.task-status.interrupted,.task-status.rollback_failed{color:#ff858b}.task-status.completed{color:#73d0a9}.actions{display:flex;gap:5px}.actions button{height:25px;display:flex;align-items:center;gap:4px;padding:0 7px;border:1px solid rgba(255,255,255,.08);border-radius:6px;color:#8793a0;background:#171d24;font-size:7px}.actions button.approve{border-color:rgba(50,213,176,.17);color:#79dac3;background:rgba(50,213,176,.06)}.actions button.rollback{border-color:rgba(156,140,255,.18);color:#ac9fff;background:rgba(156,140,255,.06)}.actions button.details{width:25px;padding:0;justify-content:center;border-color:transparent;background:transparent}.actions svg{width:11px}.task-details{margin:0 0 12px 38px;padding:12px;border:1px solid rgba(255,255,255,.055);border-radius:8px;background:#0d1217}.result-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:7px}.result-grid p{display:flex;flex-direction:column;margin:0;padding:8px;border-radius:6px;background:#12181e}.result-grid small{color:#5f6b77}.result-grid b{margin-top:4px;color:#adb7c1;font-size:7px;font-weight:500}.task-result,.task-error,.rollback-state{display:flex;align-items:flex-start;gap:7px;margin:8px 0 0;padding:9px;border-radius:6px;color:#7ed7c1;background:rgba(50,213,176,.055);font-size:8px;line-height:1.6}.task-error{color:#ff9a9f;background:rgba(255,107,114,.06)}.rollback-state{color:#afa3ff;background:rgba(156,140,255,.06)}.task-result svg,.task-error svg,.rollback-state svg{width:13px;flex:0 0 auto}.event-list,.artifact-list{margin-top:11px}.event-list>header,.artifact-list>header{display:flex;justify-content:space-between;margin-bottom:6px}.event-list header b,.artifact-list header b{font-size:8px}.event-list>p{display:grid;grid-template-columns:124px 5px 1fr;align-items:start;gap:7px;margin:0;padding:5px 0;color:#7d8995;font-size:7px;border-top:1px solid rgba(255,255,255,.035)}.event-list time{color:#596572}.event-list p>i{width:5px;height:5px;margin-top:3px;border-radius:50%;background:#52606d}.event-list p.warn>i{background:#e7a65f}.event-list p.error{color:#e88b91}.event-list p.error>i{background:#e96870}.artifact-list>a{display:flex;align-items:center;gap:8px;margin-top:5px;padding:8px;border:1px solid rgba(255,255,255,.05);border-radius:6px;color:#84909c;background:#12181e;text-decoration:none}.artifact-list>a:hover{border-color:rgba(50,213,176,.16)}.artifact-list>a>svg{width:14px}.artifact-list>a>svg:last-child{margin-left:auto}.artifact-list>a>span{display:flex;flex:1;flex-direction:column}.artifact-list>a b{color:#b9c2cc;font-size:8px}.artifact-list>a small{margin-top:3px;font-size:7px}.empty-state{display:flex;min-height:130px;align-items:center;justify-content:center;flex-direction:column;color:#596572}.empty-state svg{width:24px}.empty-state b{margin-top:9px;color:#84909c;font-size:10px}.empty-state small{margin-top:5px;text-transform:none}@media(max-width:900px){.stats,.quick-grid{grid-template-columns:1fr}.task-status{display:none}.task-details{margin-left:0}.result-grid{grid-template-columns:repeat(2,1fr)}}
</style>
