<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Activity, AlertTriangle, Archive, Bot, Box, BrainCircuit, ChevronDown, ChevronRight, ChevronUp, CircleStop, Copy, CornerDownRight, Cpu, Database, Download, FileCode2, FileUp, Files, Folder, FolderOpen, FolderTree, Gauge, GitBranch, LayoutDashboard, ListPlus, MapPin, MessageSquareText, Mic, MoreHorizontal, PanelLeftClose, Paperclip, Pencil, Play, PlugZap, Plus, RefreshCw, RotateCcw, RotateCw, Search, Send, Server, Settings, ShieldCheck, Sparkles, SquareTerminal, Trash2, Users, Vote, Wrench, X, Coffee, HardDrive, Check, LoaderCircle, SlidersHorizontal } from 'lucide-vue-next'
import AutomationView from './components/AutomationView.vue'
import CommunityView from './components/CommunityView.vue'
import ConversationTree from './components/ConversationTree.vue'
import ConversationMessageContent from './features/conversations/ConversationMessageContent.vue'
import IntegrationsView from './components/IntegrationsView.vue'
import MirrorCenterView from './features/mirror/MirrorCenterView.vue'
import SettingsView from './features/settings/SettingsView.vue'
import ProjectBuildManager from './features/project/ProjectBuildManager.vue'
import WorkspacePanelResizer from './components/WorkspacePanelResizer.vue'
import { API_BASE, ApiError, apiRequest } from './lib/api'
import { filterDeletedWorkspaceSnapshot } from './lib/dashboard-state'
import { postSse } from './lib/sse'
import { writeClipboard } from './lib/clipboard'
import { reasoningEffortFromScale, reasoningEffortToScale } from './lib/reasoning-scale'
import { loadUi, uiSettings } from './features/settings/store'
import { REASONING_EFFORTS, REVIEW_MODES } from './features/settings/types'
import type { AiAgent, AiSettingsView, ModelBinding, ReasoningEffort, ReviewMode, SpeechRecognitionSettings } from './features/settings/types'
import type { Conversation, ConversationAction, ConversationSummary } from './features/conversations/types'
import { cloudPrompts } from './features/cloud/workspace'
import { findActiveComposerToken, parseModelShortcut, replaceComposerToken } from './features/composer'
import type { ActiveComposerToken, ComposerTokenKind } from './features/composer'
import { MAX_SERVER_TEMPLATE_FILE_BYTES, parseServerTemplateManifest, type ServerTemplate } from './features/portable/server-manifest'
import { TASK_STATUS_LABELS, TERMINAL_TASK_STATUSES } from './features/automation/types'
import type { TaskInfo } from './features/automation/types'
import type { SkillItem } from './features/settings/types'

type Status = 'online' | 'stopped' | 'warning' | 'planning' | 'ready'
type ServerOperationState = 'idle' | 'provisioning' | 'starting' | 'stopping'
type WorkspaceKind = 'server' | 'project'
type Tab = 'overview' | 'files' | 'build' | 'terminal'
type Surface = 'control' | 'automation' | 'community' | 'integrations' | 'mirror' | 'settings'
interface SocialServiceSettings { enabled:boolean; qq_bot:boolean; bilibili_bot:boolean; douyin_bot:boolean; sync_interval_seconds:number; burst_interval_seconds:number; burst_recovery_seconds:number }
interface ServiceSettings { social:SocialServiceSettings; economy:boolean; player_support:boolean; game_operations:boolean; content_improvement:boolean }
interface ServerItem { id:string; name:string; kind?:WorkspaceKind; core:string; version:string; status:Status; operation_state?:ServerOperationState; core_ready?:boolean; last_error?:string|null; lifecycle_phase?:'create'|'build'|'operate'|'project'; players:string; memory:number; memory_gb?:number; cpu:number; port:number; task:string; location?:string; service_settings?:ServiceSettings }
interface ServerTelemetry { availability:'available'|'stale'|'unsupported'|'unavailable'; source:string; collected_at?:string|null; online?:number|null; max_players?:number|null; player_names?:string[]|null; tps_1m?:number|null; mspt_1m?:number|null; detail?:string|null }
interface Message { id:string; role:'assistant'|'user'; content:string; time:string; actions?:string[]; task_id?:string; task_status?:string; streaming?:boolean; fallback?:boolean; warning?:string; error?:boolean; interrupted?:boolean; retryContent?:string }
type ChatPhase = 'connecting'|'streaming'|'stopping'|'failed'|'interrupted'
interface ChatRun { conversationId:string; serverId:string; phase:ChatPhase; user:Message; reply:Message }
type QueuedChatMode = 'queue'|'steer'
interface QueuedChat { id:string; conversationId:string; serverId:string; content:string; mode:QueuedChatMode; createdAt:string; modelOverride:ModelBinding|null; agentOverride:string|null; reasoningEffort:ReasoningEffort|null }
interface EnsuredConversation { id:string; serverId:string; selected:boolean }
interface SystemInfo { java_installed:boolean; java_version?:string; java_major?:number; java_compatible:boolean; java_executable?:string; java_home?:string; os:string; arch:string; data_dir:string; data_dir_writable:boolean; data_dir_free_bytes?:number; total_memory_bytes?:number; recommended_java:number; java_install_supported:boolean; java_install_hint:string; cores:string[] }
interface MirrorInfo { id:string; name:string; base_url:string; enabled:boolean; priority:number; cores:string[]; region:string }
interface DownloadCandidate { mirror_id:string; mirror_name:string; url:string; priority:number; region:string; supported:boolean }
interface FileEntry { name:string;path:string;kind:'folder'|'file';size:number;modified?:number }
interface ComposerSuggestion { id:string;kind:ComposerTokenKind;label:string;detail:string;value:string;agentId?:string;filePath?:string;skill?:SkillItem }
interface CatalogCore { slug:string;name:string;minecraft_versions:string[] }
interface DownloadStatus { task_id:string; phase:string; source:string; received:number; total?:number|null; percent:number; message:string }
interface OpenDirectoryResponse { server:ServerItem; directory?:string; detected?:Record<string,unknown>|null; warnings?:string[]; files?:string[] }
interface SpeechRecognitionAlternativeLike { transcript:string }
interface SpeechRecognitionResultLike { isFinal:boolean; 0:SpeechRecognitionAlternativeLike }
interface SpeechRecognitionResultListLike { length:number; [index:number]:SpeechRecognitionResultLike }
interface SpeechRecognitionEventLike { resultIndex:number; results:SpeechRecognitionResultListLike }
interface BrowserSpeechRecognition {
  lang:string
  continuous:boolean
  interimResults:boolean
  onresult:((event:SpeechRecognitionEventLike)=>void)|null
  onerror:((event:{error:string})=>void)|null
  onend:(()=>void)|null
  start():void
  stop():void
  abort():void
}
interface BrowserSpeechRecognitionConstructor { new():BrowserSpeechRecognition }

const launchParams = new URLSearchParams(window.location.search)
const localPackage = import.meta.env.VITE_APP_MODE === 'local'
const cloudWorkspaceLaunch = !localPackage && launchParams.get('cloud') === 'workspace'
const WORKSPACE_MODE_KEY = 'sculk-workspace-mode-v1'
const savedWorkspaceMode = localStorage.getItem(WORKSPACE_MODE_KEY)
const workspaceMode = ref<WorkspaceKind>(savedWorkspaceMode === 'project' ? 'project' : 'server')
const servers = ref<ServerItem[]>([]), dashboardTelemetry = ref<Record<string,ServerTelemetry>>({}), selectedId = ref(''), collapsed = ref(false), tab = ref<Tab>('overview'), activeFile = ref('')
const surface = ref<Surface>(cloudWorkspaceLaunch ? 'settings' : 'control')
const input = ref(''), command = ref(''), busy = ref(false), notice = ref(''), scroller = ref<HTMLElement|null>(null), composerInput = ref<HTMLTextAreaElement|null>(null), fileEditor = ref<HTMLTextAreaElement|null>(null)
const speechState = ref<'idle'|'recording'|'transcribing'>('idle')
const speechCaptureMode = ref<'browser'|'model'|null>(null)
let browserSpeechRecognition:BrowserSpeechRecognition|null=null
let modelSpeechRecorder:MediaRecorder|null=null
let modelSpeechStream:MediaStream|null=null
let modelSpeechChunks:Blob[]=[]
let speechRecordingTimer:number|undefined
const showCreate = ref(false), createStep = ref(1), creating = ref(false), systemInfo = ref<SystemInfo|null>(null)
const showOpenDirectory = ref(false), openingDirectory = ref(false), openDirectoryPath = ref(''), openDirectoryName = ref(''), openDirectoryError = ref('')
const openDirectorySummary = ref<OpenDirectoryResponse|null>(null)
const dashboardState = ref<'loading'|'ready'|'error'>('loading'), dashboardError = ref('')
const systemState = ref<'idle'|'loading'|'ready'|'error'>('idle'), systemError = ref('')
const javaInstallState = ref<'idle'|'installing'|'success'|'error'>('idle'), javaInstallError = ref('')
const catalogCores = ref<CatalogCore[]>([])
const tasks = ref<TaskInfo[]>([])
const pendingConversationTaskRefresh = new Set<string>()
const focusedTaskId = ref('')
const mirrors = ref<MirrorInfo[]>([]), selectedMirrorIds = ref<string[]>([]), previewCandidates = ref<DownloadCandidate[]>([]), mirrorPanel = ref(false)
const createForm = ref({name:'',location:'local',core:'',version:'',memory_gb:8,port:25565,eula_accepted:false})
const projectForm = ref({name:'',location:'local'})
const importedTemplateTitle = ref('')
const manifestInput = ref<HTMLInputElement|null>(null)
const availableCores = computed(() => catalogCores.value.length?catalogCores.value.map(item=>item.name):(systemInfo.value?.cores??[]))
const createMinecraftVersions = computed(() => catalogCores.value.find(item=>item.name===createForm.value.core)?.minecraft_versions ?? [])
function defaultServiceSettings():ServiceSettings{return{social:{enabled:false,qq_bot:false,bilibili_bot:false,douyin_bot:false,sync_interval_seconds:240,burst_interval_seconds:10,burst_recovery_seconds:240},economy:false,player_support:false,game_operations:false,content_improvement:false}}
const emptyServer:ServerItem={id:'',name:'未选择工作区',kind:'server',core:'',version:'',status:'stopped',operation_state:'idle',core_ready:false,last_error:null,lifecycle_phase:'create',players:'- / -',memory:0,memory_gb:8,cpu:0,port:0,task:'请创建或选择工作区',location:'local',service_settings:defaultServiceSettings()}
const workspaceKind = (item:ServerItem):WorkspaceKind => item.kind === 'project' ? 'project' : 'server'
const visibleServers = computed(() => servers.value.filter(item => workspaceKind(item) === workspaceMode.value))
const server = computed(() => servers.value.find(item => item.id === selectedId.value) ?? visibleServers.value[0] ?? emptyServer)
const isProject = computed(() => !!selectedId.value && workspaceKind(server.value) === 'project')
const serverMemoryLimit = computed(() => server.value.memory_gb ?? 8)
const serverTelemetry = computed<ServerTelemetry>(() => dashboardTelemetry.value[server.value.id] ?? {
  availability:'unavailable',source:'managed_java_console',collected_at:null,online:null,max_players:null,player_names:null,tps_1m:null,mspt_1m:null,detail:'尚未取得服务器遥测'
})
function telemetryTime(value?:string|null){
  if(!value)return '尚未采样'
  const time=Date.parse(value);if(!Number.isFinite(time))return '采样时间未知'
  const seconds=Math.max(0,Math.floor((Date.now()-time)/1000));return seconds<60?`${seconds} 秒前`:`${Math.floor(seconds/60)} 分钟前`
}
const playerTelemetryValue = computed(() => serverTelemetry.value.online!==null&&serverTelemetry.value.online!==undefined&&serverTelemetry.value.max_players!==null&&serverTelemetry.value.max_players!==undefined ? `${serverTelemetry.value.online} / ${serverTelemetry.value.max_players}` : '不可用')
const playerTelemetryDetail = computed(() => serverTelemetry.value.availability==='available' ? `受管 Java 控制台 · ${telemetryTime(serverTelemetry.value.collected_at)}` : (serverTelemetry.value.detail ?? '在线玩家数据不可用'))
const tpsTelemetryValue = computed(() => serverTelemetry.value.tps_1m===null||serverTelemetry.value.tps_1m===undefined ? '不可用' : `${serverTelemetry.value.tps_1m.toFixed(2)} TPS`)
const tpsTelemetryDetail = computed(() => {
  if(serverTelemetry.value.tps_1m===null||serverTelemetry.value.tps_1m===undefined)return isPaperCore(server.value.core)?(serverTelemetry.value.detail ?? '尚未取得 Paper TPS 输出'):'当前核心不支持 TPS 遥测'
  const mspt=serverTelemetry.value.mspt_1m===null||serverTelemetry.value.mspt_1m===undefined?'':` · ${serverTelemetry.value.mspt_1m.toFixed(2)} MSPT`
  return `${serverTelemetry.value.source==='managed_java_console'?'Paper 控制台':'未知来源'}${mspt} · ${telemetryTime(serverTelemetry.value.collected_at)}`
})
function isPaperCore(core:string){return core.trim().toLowerCase()==='paper'}
function formatRuntimeMemory(value:number){
  if(value>=1024)return `${(value/1024).toFixed(1)} GiB`
  return `${value} MiB`
}
const bootstrapTask = computed(() => tasks.value
  .filter(task => task.server_id===selectedId.value&&['server_bootstrap','server_provision','bootstrap'].includes(task.kind))
  .reduce<TaskInfo|undefined>((latest,task)=>{
    if(!latest)return task
    const latestTime=Date.parse(latest.created_at),taskTime=Date.parse(task.created_at)
    return Number.isFinite(taskTime)&&(!Number.isFinite(latestTime)||taskTime>latestTime)?task:latest
  },undefined))
const activeProvisionStatuses = new Set(['awaiting_approval','queued','running','cancelling'])
const failedProvisionStatuses = new Set(['failed','cancelled','interrupted','rollback_failed'])
const provisionActive = computed(() => !!bootstrapTask.value&&activeProvisionStatuses.has(bootstrapTask.value.status))
const provisionFailed = computed(() => !!bootstrapTask.value&&failedProvisionStatuses.has(bootstrapTask.value.status))
const serverOperationState = computed<ServerOperationState>(() => server.value.operation_state??'idle')
const serverCoreReady = computed(() => server.value.core_ready===true)
const serverTransitioning = computed(() => serverOperationState.value!=='idle')
const showServiceSettings = ref(false)
const serviceSettingsDraft = ref<ServiceSettings|null>(null)
const serviceSettingsSaving = ref(false)
const lifecycleSaving = ref(false)
const lifecycleStages = [{key:'create',label:'创建'},{key:'build',label:'建设'},{key:'operate',label:'运营'}] as const
function lifecycleStageClass(key:string){
  const rank:Record<string,number>={create:0,build:1,operate:2}
  const current=rank[lifecyclePhase.value.key]??0,target=rank[key]??0
  return {active:current===target,done:current>target}
}
const enabledServiceCount = computed(() => {
  const settings = server.value.service_settings ?? defaultServiceSettings()
  return [settings.social.enabled,settings.economy,settings.player_support,settings.game_operations,settings.content_improvement].filter(Boolean).length
})
const lifecyclePhase = computed(() => {
  if(isProject.value)return {key:'project',label:'项目',detail:'通用 IDE 工作区，可直接编辑文件并使用对话'}
  if(server.value.status==='planning'||serverOperationState.value==='provisioning'||!serverCoreReady.value)return {key:'create',label:'创建',detail:bootstrapTask.value?`正在完成首次创建 · ${taskStatusLabel[bootstrapTask.value.status]??bootstrapTask.value.status}`:'等待服务端方案与运行环境'}
  if(server.value.lifecycle_phase==='operate')return {key:'operate',label:'运营',detail:enabledServiceCount.value?`${enabledServiceCount.value} 个运营模块已开启，可并行运行`:'正式运营中，可按需开启运营模块'}
  return {key:'build',label:'建设',detail:'核心已就绪，可安装插件并持续调整配置'}
})
async function setLifecyclePhase(phase:'build'|'operate'){
  const id=selectedId.value
  if(!id||lifecycleSaving.value||lifecyclePhase.value.key===phase)return
  lifecycleSaving.value=true
  try{
    const updated=await apiRequest<ServerItem>('/api/servers/'+id+'/lifecycle',{method:'PUT',body:JSON.stringify({phase})})
    upsertServer(updated);flash(phase==='operate'?'已进入运营阶段':'已返回建设阶段')
  }catch(error){flash('生命周期切换失败：'+String(error))}
  finally{lifecycleSaving.value=false}
}
function openServiceSettings(){
  if(!selectedId.value||isProject.value)return
  const source=server.value.service_settings??defaultServiceSettings()
  serviceSettingsDraft.value=JSON.parse(JSON.stringify(source)) as ServiceSettings
  showServiceSettings.value=true
}
function closeServiceSettings(){showServiceSettings.value=false;serviceSettingsDraft.value=null}
async function saveServiceSettings(){
  const id=selectedId.value,draft=serviceSettingsDraft.value
  if(!id||!draft||serviceSettingsSaving.value)return
  if(draft.social.enabled&&!draft.social.qq_bot&&!draft.social.bilibili_bot&&!draft.social.douyin_bot){flash('开启社交运营前，至少选择一个机器人渠道');return}
  serviceSettingsSaving.value=true
  try{
    const updated=await apiRequest<ServerItem>('/api/servers/'+id+'/services',{method:'PUT',body:JSON.stringify(draft)})
    upsertServer(updated);closeServiceSettings();flash('服务设置已保存')
  }catch(error){flash('服务设置保存失败：'+String(error))}
  finally{serviceSettingsSaving.value=false}
}
const conversationsByServer = ref<Record<string,ConversationSummary[]>>({})
const selectedConversationId = ref('')
const selectedConversationByServer = ref<Record<string,string>>({})
const conversationSelectionPending = ref(false)
const conversationCreationPending = ref(false)
let conversationSelectionEpoch=0,workspaceSelectionEpoch=0
const ensureConversationPromises=new Map<string,Promise<EnsuredConversation>>()
const selectedConversation = computed(()=>Object.values(conversationsByServer.value).flat().find(item=>item.id===selectedConversationId.value))
const messages = ref<Message[]>([])
const chatRuns = ref<Record<string,ChatRun>>({})
const chatControllers = new Map<string,AbortController>()
const CHAT_QUEUE_STORAGE_KEY='sculk-chat-queues-v1'
function readChatQueues(){
  try{
    const parsed=JSON.parse(localStorage.getItem(CHAT_QUEUE_STORAGE_KEY)??'{}') as unknown
    if(!parsed||typeof parsed!=='object'||Array.isArray(parsed))return{}
    const queues:Record<string,QueuedChat[]>={}
    for(const [conversationId,rawItems] of Object.entries(parsed).slice(0,100)){
      if(!Array.isArray(rawItems))continue
      const items=rawItems.slice(0,100).flatMap(raw=>{
        if(!raw||typeof raw!=='object')return[]
        const candidate=raw as Record<string,unknown>
        if(typeof candidate.id!=='string'||typeof candidate.serverId!=='string'||typeof candidate.content!=='string'||!candidate.content.trim()||candidate.content.length>64_000)return[]
        const model=candidate.modelOverride
        const modelOverride=model&&typeof model==='object'&&typeof (model as Record<string,unknown>).provider_id==='string'&&typeof (model as Record<string,unknown>).model_id==='string'?model as unknown as ModelBinding:null
        const effort=typeof candidate.reasoningEffort==='string'&&REASONING_EFFORTS.some(item=>item.key===candidate.reasoningEffort)?candidate.reasoningEffort as ReasoningEffort:null
        return[{id:candidate.id,conversationId,serverId:candidate.serverId,content:candidate.content,mode:candidate.mode==='steer'?'steer':'queue',createdAt:typeof candidate.createdAt==='string'?candidate.createdAt:new Date().toISOString(),modelOverride,agentOverride:typeof candidate.agentOverride==='string'?candidate.agentOverride:null,reasoningEffort:effort} satisfies QueuedChat]
      })
      if(items.length)queues[conversationId]=items
    }
    return queues
  }catch{return{}}
}
const chatQueues=ref<Record<string,QueuedChat[]>>(readChatQueues())
const pendingSteerIds=new Map<string,string>()
const editingQueuedId=ref(''),editingQueuedContent=ref('')
const activeConversationIds = computed(()=>Object.values(chatRuns.value).filter(run=>['connecting','streaming','stopping'].includes(run.phase)).map(run=>run.conversationId))
const currentChatRun = computed(()=>selectedConversationId.value?chatRuns.value[selectedConversationId.value]:undefined)
const chatBusy = computed(()=>!!currentChatRun.value&&['connecting','streaming','stopping'].includes(currentChatRun.value.phase))
const currentChatQueue = computed(()=>selectedConversationId.value?(chatQueues.value[selectedConversationId.value]??[]):[])
const currentQueuePaused = computed(()=>currentChatQueue.value.length>0&&!chatBusy.value)
const thinking = computed(()=>currentChatRun.value?.phase==='connecting')
const chatStatusLabel = computed(()=>currentChatRun.value?.phase==='connecting'?'正在连接':currentChatRun.value?.phase==='streaming'?'正在生成':currentChatRun.value?.phase==='stopping'?'正在停止':'')
const defaultSpeechSettings:SpeechRecognitionSettings={mode:'browser',language:'zh-CN',provider_id:null,model_id:'whisper-1'}
const speechSettings=computed(()=>aiSettings.value?.speech_recognition??defaultSpeechSettings)
const speechButtonTitle=computed(()=>speechState.value==='transcribing'?'正在转写录音':speechState.value==='recording'?'停止录音':speechSettings.value.mode==='browser'?'使用浏览器语音识别':'录音并使用 ASR 模型转写')
const showMessageSearch = ref(false), messageSearch = ref(''), activeSearchMatch = ref(0), messageSearchInput = ref<HTMLInputElement|null>(null)
const showContextMenu = ref(false)
const composerToken=ref<ActiveComposerToken|null>(null),composerSuggestionIndex=ref(0)
const composerSkills=ref<SkillItem[]>([]),composerSkillsLoaded=ref(false),composerFiles=ref<string[]>([]),composerFilesWorkspace=ref(''),composerSourceLoading=ref<ComposerTokenKind|null>(null)
const messageSearchMatches = computed(()=>{
  const query=messageSearch.value.trim().toLocaleLowerCase('zh-CN')
  return query?messages.value.filter(message=>message.content.toLocaleLowerCase('zh-CN').includes(query)).map(message=>message.id):[]
})
const DRAFT_STORAGE_KEY='sculk-conversation-drafts-v1'
function readDrafts(){try{return JSON.parse(localStorage.getItem(DRAFT_STORAGE_KEY)??'{}') as Record<string,string>}catch{return {}}}
const conversationDrafts=ref<Record<string,string>>(readDrafts())
let suppressedDraftWrites=0
const conversationsLoading = ref(false)
const conversationDialog = ref<{kind:'rename'|'group'|'delete';conversation:ConversationSummary}|null>(null)
const conversationDialogValue = ref('')
const deleteServerTarget = ref<ServerItem|null>(null)
const deleteServerFiles = ref(false)
const deleteServerStep = ref<1|2>(1)
const deleteServerConfirmation = ref('')
const deletedWorkspaceIds = new Set<string>()
const fileEntries=ref<FileEntry[]>([]),currentPath=ref(''),parentPath=ref<string|null>(null),fileContent=ref(''),fileReadonly=ref(false),showNewFolder=ref(false),newFolderName=ref(''),showNewFile=ref(false),newFileName=ref('')
const fileUploadInput=ref<HTMLInputElement|null>(null)
const fileContextMenu=ref<{entry:FileEntry|null;x:number;y:number}|null>(null)
const fileActionDialog=ref<{kind:'rename'|'delete';entry:FileEntry}|null>(null)
const fileActionValue=ref(''),fileActionBusy=ref(false)
const fileSelection=ref<{start:number;end:number}|null>(null)
let fileOperationEpoch=0
const hasFileSelection=computed(()=>!!fileSelection.value&&fileSelection.value.end>fileSelection.value.start)
const canTransferFiles=computed(()=>!!selectedId.value&&(isProject.value||workspaceKind(server.value)==='server'))
const selectedTasks = computed(() => tasks.value.filter(task=>task.server_id===selectedId.value))
const terminal = ref<string[]>([])
const GIB = 1024 ** 3
const MIN_DISK_BYTES = 2 * GIB
const javaReady = computed(() => systemState.value==='ready'&&!!systemInfo.value?.java_installed&&!!systemInfo.value.java_compatible)
const diskReady = computed(() => systemState.value==='ready'&&systemInfo.value?.data_dir_free_bytes!==undefined&&systemInfo.value.data_dir_free_bytes>=MIN_DISK_BYTES)
const environmentUnknown = computed(() => systemState.value!=='ready'||systemInfo.value?.data_dir_free_bytes===undefined||systemInfo.value?.total_memory_bytes===undefined)
const totalMemoryGb = computed(() => systemInfo.value?.total_memory_bytes===undefined?null:systemInfo.value.total_memory_bytes/GIB)
const reasonableMemoryMax = computed(() => totalMemoryGb.value===null?64:Math.min(64,Math.floor(totalMemoryGb.value*.8)))
const portIssue = computed(() => {
  const port=Number(createForm.value.port)
  return Number.isInteger(port)&&port>=1024&&port<=65535?'':'端口必须是 1024–65535 之间的整数'
})
const memoryIssue = computed(() => {
  const memory=Number(createForm.value.memory_gb)
  if(!Number.isInteger(memory)||memory<2)return '内存必须是至少 2 GB 的整数'
  if(memory>64)return '单个服务器最多分配 64 GB 内存'
  if(totalMemoryGb.value!==null&&reasonableMemoryMax.value<2)return `系统总内存仅 ${totalMemoryGb.value.toFixed(1)} GB，不足以安全分配最低 2 GB`
  if(totalMemoryGb.value!==null&&memory>reasonableMemoryMax.value)return `最多可分配 ${reasonableMemoryMax.value} GB（保留约 20% 系统内存）`
  return ''
})
const parameterIssue = computed(() => {
  if(!createForm.value.core)return '尚未获取可用核心'
  if(availableCores.value.length&&!availableCores.value.includes(createForm.value.core))return `本机资源目录不支持核心 ${createForm.value.core}`
  if(!createForm.value.version)return '尚未获取该核心的 Minecraft 版本'
  if(createMinecraftVersions.value.length&&!createMinecraftVersions.value.includes(createForm.value.version))return `${createForm.value.core} 当前不支持 Minecraft ${createForm.value.version}`
  if(servers.value.some(item=>item.port===Number(createForm.value.port)))return `端口 ${createForm.value.port} 已被现有服务器占用`
  return portIssue.value||memoryIssue.value
})
const environmentIssue = computed(() => {
  if(systemState.value!=='ready'||!systemInfo.value)return '尚未取得后端环境信息'
  if(!systemInfo.value.data_dir_writable)return '服务器数据目录不可写'
  if(!javaReady.value)return systemInfo.value.java_installed?'当前 Java 版本不兼容':'尚未安装 Java'
  if(systemInfo.value.data_dir_free_bytes!==undefined&&!diskReady.value)return '服务器数据目录可用空间不足 2 GB'
  return memoryIssue.value
})
const serverOperationLabel = computed(() => ({
  idle: '', provisioning: '正在初始化', starting: '正在启动', stopping: '正在停止',
} as Record<ServerOperationState,string>)[serverOperationState.value])
const serverStatusLabel = computed(() => serverOperationLabel.value||(server.value.status==='online'?'运行中':server.value.status==='warning'?'需关注':'已停止'))
const serverStartBlocker = computed(() => {
  if(!selectedId.value)return '请先选择服务器'
  if(serverOperationState.value==='provisioning')return '首次初始化尚未完成'
  if(serverOperationState.value==='starting')return '服务器正在启动'
  if(serverOperationState.value==='stopping')return '服务器正在停止'
  if(provisionActive.value)return '首次初始化任务仍在执行'
  if(provisionFailed.value)return serverCoreReady.value?'核心已安装，但首次初始化检查未闭环，请重试完成初始化':'首次初始化未完成，请重试或手动下载核心'
  if(!serverCoreReady.value)return '服务端核心尚未下载并校验'
  if(systemState.value!=='ready')return '尚未取得 Java 环境状态'
  if(!javaReady.value)return systemInfo.value?.java_installed?'当前 Java 版本不兼容':'尚未安装 Java'
  return ''
})
const canStartServer = computed(() => server.value.status!=='online'&&!serverStartBlocker.value)
const serverControlDisabled = computed(() => busy.value||serverTransitioning.value||(server.value.status!=='online'&&!canStartServer.value))
const terminalCommandReady = computed(() => server.value.status==='online'&&serverOperationState.value==='idle')
const terminalCommandPlaceholder = computed(() => {
  if(serverOperationState.value==='starting')return '服务器启动完成后即可执行命令'
  if(serverOperationState.value==='stopping')return '服务器正在停止，暂时不能执行命令'
  if(serverOperationState.value==='provisioning')return '首次初始化完成后即可使用控制台'
  if(server.value.status!=='online')return '服务器运行后可输入命令'
  return '输入服务器命令，例如 list'
})
const taskStatusLabel=TASK_STATUS_LABELS
function formatBytes(value?:number){if(value===undefined)return '未知';if(value>=GIB)return (value/GIB).toFixed(1)+' GB';return (value/1048576).toFixed(0)+' MB'}
const now = () => new Date().toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'})
const aiSettings = ref<AiSettingsView|null>(null)
const chatModelOverride = ref<ModelBinding|null>(null)
const chatAgentOverride = ref<string|null>(null) // null=跟随全局；'default'=强制内置；agent id=强制该 Agent
const chatReasoningEffort = ref<ReasoningEffort|null>(null)
const showAgentMenu = ref(false), showModelMenu = ref(false), showReviewMenu = ref(false)
const showQuickPrompts = ref(false)
const safeNotice = ref('')
let safeNoticeTimer:number|undefined
function flashSafeNotice(text:string){safeNotice.value=text;if(safeNoticeTimer)window.clearTimeout(safeNoticeTimer);safeNoticeTimer=window.setTimeout(()=>{safeNotice.value=''},4000)}
const reviewMode = computed<ReviewMode>(()=>aiSettings.value?.review_mode ?? 'approval')
const reviewModeLabel = computed(()=>REVIEW_MODES.find(mode=>mode.key===reviewMode.value)?.label ?? '请求批准')
const safeHint = computed(()=>({
  approval:'高风险操作会在执行前请求你的确认',
  auto:'AI 将自动批准中低风险任务，高风险仍需你确认',
  full:'完全访问已开启，所有任务将自动执行',
}[reviewMode.value]))
const modelMenuGroups = computed(()=>(aiSettings.value?.providers ?? []).filter(provider=>provider.enabled).map(provider=>({id:provider.id,name:provider.name,models:provider.models.filter(model=>model.enabled)})).filter(group=>group.models.length))
const agentMenuItems = computed(()=>(aiSettings.value?.agents ?? []).filter(agent=>agent.enabled))
const composerSuggestions=computed<ComposerSuggestion[]>(()=>{
  const token=composerToken.value;if(!token)return[]
  const query=token.query.toLocaleLowerCase('zh-CN')
  if(token.kind==='skill')return composerSkills.value.filter(skill=>skill.enabled&&(!query||skill.id.toLocaleLowerCase('zh-CN').includes(query)||skill.name.toLocaleLowerCase('zh-CN').includes(query))).slice(0,10).map(skill=>({id:'skill:'+skill.id,kind:'skill',label:skill.name,detail:skill.description,value:skill.id,skill}))
  if(token.kind==='file')return composerFiles.value.filter(path=>!query||path.toLocaleLowerCase('zh-CN').includes(query)).slice(0,12).map(path=>({id:'file:'+path,kind:'file',label:path.split('/').at(-1)??path,detail:path,value:path,filePath:path}))
  const builtIn:ComposerSuggestion={id:'agent:default',kind:'agent',label:'Sculk Agent',detail:'内置模型直连智能体',value:'Sculk-Agent',agentId:'default'}
  return [builtIn,...agentMenuItems.value.map(agent=>({id:'agent:'+agent.id,kind:'agent' as const,label:agent.name,detail:agentMenuDetail(agent),value:agent.name.replace(/\s+/g,'-'),agentId:agent.id}))].filter(item=>!query||item.label.toLocaleLowerCase('zh-CN').includes(query)||item.value.toLocaleLowerCase('zh-CN').includes(query)).slice(0,10)
})
const activeAgentId = computed(()=>{
  if(chatAgentOverride.value==='default')return null
  if(chatAgentOverride.value)return chatAgentOverride.value
  return aiSettings.value?.active_agent ?? null
})
const chatAgentLabel = computed(()=>{
  const agentId = activeAgentId.value
  if(agentId){
    const agent = aiSettings.value?.agents.find(item=>item.id===agentId)
    if(agent)return agent.name
  }
  return 'Sculk Agent'
})
const activeAgent = computed(()=>activeAgentId.value ? aiSettings.value?.agents.find(item=>item.id===activeAgentId.value) ?? null : null)
function fullAccessAgentPolicyMessage(agent:AiAgent|null){
  if(reviewMode.value!=='full'||!agent)return''
  if(agent.kind!=='codex'||(agent.transport??'acp')!=='cli')return '完全访问仅支持原生 Codex CLI；请选择 Codex CLI，或切回请求批准/替我审核模式'
  if((aiSettings.value?.codex_full_access_ready_agent_ids??[]).includes(agent.id))return''
  return aiSettings.value?.codex_full_access_available?'当前 Codex 命令未授权；请在设置中重新接入检测到的 Codex，或改为已授权绝对路径':'Codex 完整权限尚未配置；请在设置 → 常规完成本机启动授权'
}
const activeAgentPolicyMessage=computed(()=>fullAccessAgentPolicyMessage(activeAgent.value))
const reasoningSupported = computed(()=>!activeAgent.value || (activeAgent.value.transport ?? 'acp')==='cli')
const reasoningMenuItems = computed(()=>{
  if(!activeAgent.value)return REASONING_EFFORTS
  const detected=aiSettings.value?.detected_agents?.find(item=>item.kind===activeAgent.value?.kind)
  const values=detected?.capabilities.reasoning_effort.values ?? (activeAgent.value.kind==='codex'?['minimal','low','medium','high','xhigh']:activeAgent.value.kind==='claude-code'?['low','medium','high','xhigh','max']:[])
  return REASONING_EFFORTS.filter(item=>values.includes(item.key))
})
const reasoningLabel = computed(()=>chatReasoningEffort.value ? `思考：${REASONING_EFFORTS.find(item=>item.key===chatReasoningEffort.value)?.label ?? chatReasoningEffort.value}` : '思考：自动')
const reasoningShortLabel = computed(()=>chatReasoningEffort.value ? REASONING_EFFORTS.find(item=>item.key===chatReasoningEffort.value)?.label ?? chatReasoningEffort.value : '自动')
function reasoningItemsForAgent(agent:AiAgent|null){
  if(!agent)return REASONING_EFFORTS
  const detected=aiSettings.value?.detected_agents?.find(item=>item.kind===agent.kind)
  const values=detected?.capabilities.reasoning_effort.values ?? (agent.kind==='codex'?['minimal','low','medium','high','xhigh']:agent.kind==='claude-code'?['low','medium','high','xhigh','max']:[])
  return REASONING_EFFORTS.filter(item=>values.includes(item.key))
}
function sameModelBinding(left:ModelBinding|null,right:ModelBinding|null){return !left&&!right||!!left&&!!right&&left.provider_id===right.provider_id&&left.model_id===right.model_id}
function modelReasoningIndex(binding:ModelBinding|null){
  if(!sameModelBinding(chatModelOverride.value,binding))return 0
  return reasoningEffortToScale(chatReasoningEffort.value,reasoningMenuItems.value)
}
function modelReasoningLabel(binding:ModelBinding|null){
  if(!sameModelBinding(chatModelOverride.value,binding)||!chatReasoningEffort.value)return '自动'
  return REASONING_EFFORTS.find(item=>item.key===chatReasoningEffort.value)?.label??chatReasoningEffort.value
}
function agentReasoningIndex(agent:AiAgent){
  if(activeAgentId.value!==agent.id)return 0
  return reasoningEffortToScale(chatReasoningEffort.value,reasoningItemsForAgent(agent))
}
function agentReasoningLabel(agent:AiAgent){
  if(activeAgentId.value!==agent.id||!chatReasoningEffort.value)return '自动'
  return REASONING_EFFORTS.find(item=>item.key===chatReasoningEffort.value)?.label??chatReasoningEffort.value
}
function agentMenuDetail(agent:AiAgent){
  const transport=(agent.transport??'acp')==='cli'?'原生 CLI':'ACP 协议'
  if(agent.kind!=='codex'||transport!=='原生 CLI')return `${agent.kind} · ${transport}${reviewMode.value==='full'?' · 完全访问不支持':''}`
  const codexFullAccessAvailable=aiSettings.value?.codex_full_access_available===true
  const codexFullAccessReady=codexFullAccessAvailable&&(aiSettings.value?.codex_full_access_ready_agent_ids??[]).includes(agent.id)
  const access=reviewMode.value!=='full'?'只读保护':codexFullAccessReady?'完全访问权限':codexFullAccessAvailable?'当前命令未授权':'完全访问未配置'
  return `${agent.kind} · ${transport} · ${access}`
}
const chatModelLabel = computed(()=>{
  const override = chatModelOverride.value
  if(!override) return '自动模型'
  const provider = aiSettings.value?.providers.find(item=>item.id===override.provider_id)
  return (provider?.name ?? '未知提供商')+' / '+override.model_id
})
async function loadAiSettings(){try{aiSettings.value=await apiRequest<AiSettingsView>('/api/ai/settings')}catch{}}
async function persistConversationExecution(modelBinding:ModelBinding|null,agentOverride:string|null,reasoningEffort:ReasoningEffort|null){
  const ensured=await ensureConversation();if(!ensured.selected||!ensured.id)throw new Error('无法创建或选择对话任务')
  const conversationId=ensured.id
  const summary=await apiRequest<ConversationSummary>('/api/conversations/'+conversationId+'/execution',{method:'PUT',body:JSON.stringify({model_binding:modelBinding,agent_override:agentOverride,reasoning_effort:reasoningEffort})})
  const serverItems=conversationsByServer.value[summary.server_id]??[]
  conversationsByServer.value={...conversationsByServer.value,[summary.server_id]:serverItems.map(item=>item.id===summary.id?summary:item)}
  return summary
}
async function pickChatModel(binding:ModelBinding|null,effort:ReasoningEffort|null=null){
  const workspaceId=selectedId.value;showModelMenu.value=false
  try{const summary=await persistConversationExecution(binding,chatAgentOverride.value,effort);if(selectedId.value!==workspaceId||selectedConversationId.value!==summary.id)return;chatModelOverride.value=binding;chatReasoningEffort.value=effort;flash(binding?`当前对话将使用 ${chatModelLabel.value} · 思考${reasoningShortLabel.value}`:`当前对话已恢复自动模型 · 思考${reasoningShortLabel.value}`)}
  catch(error){flash('模型选择保存失败：'+String(error))}
}
function changeChatModelReasoning(binding:ModelBinding|null,event:Event){
  const index=Number((event.target as HTMLInputElement).value)
  void pickChatModel(binding,reasoningEffortFromScale(index,reasoningMenuItems.value))
}
async function pickChatAgent(agentId:string|null,requestedEffort:ReasoningEffort|null=chatReasoningEffort.value):Promise<boolean>{
  const workspaceId=selectedId.value
  const nextAgent=agentId===null?(aiSettings.value?.active_agent?'default':null):agentId
  showAgentMenu.value=false
  try{
    const chosen=agentId?aiSettings.value?.agents.find(item=>item.id===agentId):null
    const policyMessage=fullAccessAgentPolicyMessage(chosen??null)
    if(policyMessage){flashSafeNotice(policyMessage);return false}
    const values=chosen?reasoningItemsForAgent(chosen).map(item=>item.key):[]
    const nextEffort=chosen?(chosen.transport??'acp')==='cli'&&(!requestedEffort||values.includes(requestedEffort))?requestedEffort:null:requestedEffort
    const summary=await persistConversationExecution(chatModelOverride.value,nextAgent,nextEffort)
    if(selectedId.value!==workspaceId||selectedConversationId.value!==summary.id)return false
    chatAgentOverride.value=nextAgent
    chatReasoningEffort.value=nextEffort
    flash(agentId?`当前对话将交给 ${chatAgentLabel.value}`:'当前对话使用内置 Sculk Agent')
    return true
  }
  catch(error){flash('Agent 选择保存失败：'+String(error));return false}
}
function changeChatAgentReasoning(agent:AiAgent,event:Event){
  const index=Number((event.target as HTMLInputElement).value)
  void pickChatAgent(agent.id,reasoningEffortFromScale(index,reasoningItemsForAgent(agent)))
}
async function pickReviewMode(mode:ReviewMode){
  showReviewMenu.value=false
  if(mode===reviewMode.value)return
  try{aiSettings.value=await apiRequest<AiSettingsView>('/api/ai/review-mode',{method:'PUT',body:JSON.stringify({mode})});flashSafeNotice(safeHint.value??'')}
  catch(error){flash('切换失败：'+String(error))}
}
function closeMenus(event:MouseEvent){
  const target=event.target as HTMLElement
  const insideFileMenu=!!target.closest('.file-context-menu')
  if(!target.closest('.composer-menu-anchor')){showAgentMenu.value=false;showModelMenu.value=false;showReviewMenu.value=false;showContextMenu.value=false}
  if(!insideFileMenu)fileContextMenu.value=null
  if((showNewFile.value||showNewFolder.value)&&!target.closest('.new-entry-form')&&!target.closest('[data-new-entry-toggle]')&&!insideFileMenu)void submitNewEntry()
}
function scrollChat(smooth=true){scroller.value?.scrollTo({top:scroller.value.scrollHeight,behavior:smooth?'smooth':'auto'})}
let deltaScrollTimer:number|undefined
function throttledScroll(){if(deltaScrollTimer)return;deltaScrollTimer=window.setTimeout(()=>{deltaScrollTimer=undefined;scrollChat(false)},80)}
function draftKey(serverId=selectedId.value,conversationId=selectedConversationId.value){return conversationId?'conversation:'+conversationId:serverId?'server:'+serverId:''}
function saveCurrentDraft(){const key=draftKey();if(!key)return;const value=input.value;if(value)conversationDrafts.value={...conversationDrafts.value,[key]:value};else{const next={...conversationDrafts.value};delete next[key];conversationDrafts.value=next}}
function discardConversationDraft(conversationId:string){const next={...conversationDrafts.value};delete next[draftKey('',conversationId)];conversationDrafts.value=next}
function restoreUnsentDraft(key:string,content:string){
  if(!key)return
  const existing=((draftKey()===key?input.value:'')||conversationDrafts.value[key]||'').trim()
  const restored=existing?`${content}\n\n${existing}`:content
  conversationDrafts.value={...conversationDrafts.value,[key]:restored}
  if(draftKey()===key)setComposerInput(restored)
}
function setComposerInput(value:string){suppressedDraftWrites+=1;input.value=value;nextTick(()=>{suppressedDraftWrites=Math.max(0,suppressedDraftWrites-1)})}
function restoreCurrentDraft(){setComposerInput(conversationDrafts.value[draftKey()]??'')}
function appendSpeechTranscript(value:string){
  const transcript=value.trim();if(!transcript)return
  const current=input.value,separator=current&&!/\s$/u.test(current)?' ':''
  const next=current+separator+transcript
  setComposerInput(next)
  nextTick(()=>{composerInput.value?.focus();composerInput.value?.setSelectionRange(next.length,next.length);updateComposerAssist()})
}
function speechRecognitionConstructor():BrowserSpeechRecognitionConstructor|null{
  const speechWindow=window as typeof window&{SpeechRecognition?:BrowserSpeechRecognitionConstructor;webkitSpeechRecognition?:BrowserSpeechRecognitionConstructor}
  return speechWindow.SpeechRecognition??speechWindow.webkitSpeechRecognition??null
}
function speechErrorMessage(error:string){
  if(error==='not-allowed'||error==='service-not-allowed')return'麦克风或语音识别权限被拒绝，请在浏览器站点权限中允许后重试'
  if(error==='audio-capture')return'没有找到可用麦克风'
  if(error==='network')return'浏览器语音识别服务当前无法连接'
  if(error==='no-speech')return'没有识别到语音，请靠近麦克风后重试'
  return`语音识别失败：${error}`
}
function clearSpeechRecordingTimer(){if(speechRecordingTimer){window.clearTimeout(speechRecordingTimer);speechRecordingTimer=undefined}}
function releaseModelSpeechStream(){modelSpeechStream?.getTracks().forEach(track=>track.stop());modelSpeechStream=null}
function finishSpeechCapture(){clearSpeechRecordingTimer();speechState.value='idle';speechCaptureMode.value=null}
function startBrowserSpeechRecognition(){
  const Constructor=speechRecognitionConstructor()
  if(!Constructor){flash('当前浏览器不支持语音识别 API，请在设置中改用 ASR 模型模式');return}
  const recognition=new Constructor();browserSpeechRecognition=recognition
  if(speechSettings.value.language&&speechSettings.value.language!=='auto')recognition.lang=speechSettings.value.language
  recognition.continuous=true;recognition.interimResults=false
  recognition.onresult=event=>{let transcript='';for(let index=event.resultIndex;index<event.results.length;index+=1){const result=event.results[index];if(result?.isFinal)transcript+=`${result[0]?.transcript??''} `}appendSpeechTranscript(transcript)}
  recognition.onerror=event=>{flash(speechErrorMessage(event.error));browserSpeechRecognition=null;finishSpeechCapture()}
  recognition.onend=()=>{browserSpeechRecognition=null;finishSpeechCapture()}
  try{recognition.start();speechCaptureMode.value='browser';speechState.value='recording'}
  catch(error){browserSpeechRecognition=null;finishSpeechCapture();flash('无法启动浏览器语音识别：'+String(error))}
}
function preferredRecordingMimeType(){return['audio/webm;codecs=opus','audio/webm','audio/ogg;codecs=opus','audio/mp4'].find(type=>MediaRecorder.isTypeSupported(type))??''}
function recordingFileName(type:string){if(type.includes('ogg'))return'recording.ogg';if(type.includes('mp4'))return'recording.m4a';return'recording.webm'}
async function transcribeModelRecording(blob:Blob){
  if(blob.size===0){flash('没有录到音频，请重试');finishSpeechCapture();return}
  if(blob.size>25*1024*1024){flash('录音超过 25 MiB，请缩短录音后重试');finishSpeechCapture();return}
  speechState.value='transcribing'
  const form=new FormData();form.append('audio',blob,recordingFileName(blob.type))
  try{const result=await apiRequest<{text:string}>('/api/ai/transcriptions',{method:'POST',body:form});appendSpeechTranscript(result.text);flash('录音已转成对话草稿')}
  catch(error){flash('录音转写失败：'+String(error))}
  finally{finishSpeechCapture()}
}
async function startModelSpeechRecording(){
  if(!speechSettings.value.provider_id||!speechSettings.value.model_id.trim()){flash('请先在「设置 → 语音识别」配置 ASR 提供商和模型');return}
  if(!navigator.mediaDevices?.getUserMedia||typeof MediaRecorder==='undefined'){flash('当前浏览器无法录音，请检查浏览器版本和安全连接');return}
  try{
    const stream=await navigator.mediaDevices.getUserMedia({audio:{echoCancellation:true,noiseSuppression:true,autoGainControl:true}})
    const mimeType=preferredRecordingMimeType(),recorder=mimeType?new MediaRecorder(stream,{mimeType}):new MediaRecorder(stream)
    modelSpeechStream=stream;modelSpeechRecorder=recorder;modelSpeechChunks=[]
    recorder.ondataavailable=event=>{if(event.data.size)modelSpeechChunks.push(event.data)}
    recorder.onerror=()=>{recorder.onstop=null;releaseModelSpeechStream();modelSpeechRecorder=null;finishSpeechCapture();flash('录音设备发生错误')}
    recorder.onstop=()=>{const blob=new Blob(modelSpeechChunks,{type:recorder.mimeType||mimeType||'audio/webm'});modelSpeechChunks=[];modelSpeechRecorder=null;releaseModelSpeechStream();void transcribeModelRecording(blob)}
    recorder.start(1000);speechCaptureMode.value='model';speechState.value='recording'
    speechRecordingTimer=window.setTimeout(()=>{if(modelSpeechRecorder?.state==='recording'){flash('录音已达到 5 分钟上限，正在转写');modelSpeechRecorder.stop()}},5*60*1000)
  }catch(error){releaseModelSpeechStream();finishSpeechCapture();const denied=error instanceof DOMException&&['NotAllowedError','SecurityError'].includes(error.name);flash(denied?'麦克风权限被拒绝，请在浏览器站点权限中允许后重试':'无法启动录音：'+String(error))}
}
function stopSpeechCapture(){
  clearSpeechRecordingTimer()
  if(speechCaptureMode.value==='browser'&&browserSpeechRecognition){browserSpeechRecognition.stop();return}
  if(speechCaptureMode.value==='model'&&modelSpeechRecorder?.state==='recording'){modelSpeechRecorder.stop();return}
  finishSpeechCapture()
}
function toggleSpeechCapture(){
  if(speechState.value==='transcribing')return
  if(speechState.value==='recording'){stopSpeechCapture();return}
  if(speechSettings.value.mode==='model')void startModelSpeechRecording();else startBrowserSpeechRecognition()
}
function abortSpeechCapture(){clearSpeechRecordingTimer();browserSpeechRecognition?.abort();browserSpeechRecognition=null;const recorder=modelSpeechRecorder;if(recorder){recorder.onstop=null;recorder.onerror=null;if(recorder.state==='recording')recorder.stop()}modelSpeechRecorder=null;modelSpeechChunks=[];releaseModelSpeechStream();finishSpeechCapture()}
function resetConversationTransient(clearInput=true){
  composerToken.value=null;composerSuggestionIndex.value=0;editingQueuedId.value='';editingQueuedContent.value=''
  showMessageSearch.value=false;messageSearch.value='';activeSearchMatch.value=0
  if(clearInput)setComposerInput('')
}
function resizeComposer(){const element=composerInput.value;if(!element)return;element.style.height='auto';element.style.height=Math.min(Math.max(element.scrollHeight,42),160)+'px'}
async function loadComposerSkills(){
  if(composerSkillsLoaded.value||composerSourceLoading.value==='skill')return
  composerSourceLoading.value='skill'
  try{const data=await apiRequest<{skills:SkillItem[]}>('/api/integrations');composerSkills.value=data.skills??[];composerSkillsLoaded.value=true}
  catch(error){flashSafeNotice('技能列表读取失败：'+String(error))}
  finally{composerSourceLoading.value=null}
}
async function loadComposerFiles(){
  const workspaceId=selectedId.value;if(!workspaceId||composerSourceLoading.value==='file')return
  if(composerFilesWorkspace.value===workspaceId)return
  composerSourceLoading.value='file'
  const files:string[]=[];const pending:{path:string;depth:number}[]=[{path:'',depth:0}];const skipped=new Set(['.git','node_modules','target','.pnpm-store'])
  try{
    while(pending.length&&files.length<400){
      const current=pending.shift()!;const data=await apiRequest<{entries:FileEntry[]}>('/api/servers/'+workspaceId+'/files?path='+encodeURIComponent(current.path))
      for(const entry of data.entries??[]){
        if(entry.kind==='file'){if(entry.size<=2_000_000)files.push(entry.path)}
        else if(current.depth<7&&!skipped.has(entry.name))pending.push({path:entry.path,depth:current.depth+1})
        if(files.length>=400)break
      }
    }
    if(selectedId.value===workspaceId){composerFiles.value=files;composerFilesWorkspace.value=workspaceId}
  }catch(error){flashSafeNotice('文件索引读取失败：'+String(error))}
  finally{composerSourceLoading.value=null}
}
function updateComposerAssist(){
  const element=composerInput.value;composerToken.value=findActiveComposerToken(input.value,element?.selectionStart??input.value.length);composerSuggestionIndex.value=0
  if(composerToken.value?.kind==='skill')void loadComposerSkills()
  if(composerToken.value?.kind==='file')void loadComposerFiles()
}
function applyComposerReplacement(value:string,cursor:number){input.value=value;nextTick(()=>{composerInput.value?.focus();composerInput.value?.setSelectionRange(cursor,cursor);updateComposerAssist()})}
function utf8Length(value:string){return new TextEncoder().encode(value).length}
function fitUtf8(value:string,maxBytes:number){if(maxBytes<=0)return'';if(utf8Length(value)<=maxBytes)return value;let low=0,high=value.length;while(low<high){const middle=Math.ceil((low+high)/2);if(utf8Length(value.slice(0,middle))<=maxBytes)low=middle;else high=middle-1}let fitted=value.slice(0,low);if(fitted&&/[\uD800-\uDBFF]$/u.test(fitted))fitted=fitted.slice(0,-1);return fitted}
function fileLanguage(path:string){const extension=path.split('.').at(-1)?.toLowerCase()??'';return({ts:'typescript',tsx:'tsx',js:'javascript',jsx:'jsx',rs:'rust',json:'json',yml:'yaml',yaml:'yaml',toml:'toml',md:'markdown',py:'python',java:'java',kt:'kotlin',sh:'bash',ps1:'powershell',css:'css',html:'html',vue:'vue'} as Record<string,string>)[extension]??''}
function fileContext(path:string,content:string,availableBytes:number,label=path){
  const header=`[文件：${label}]\n\`\`\`${fileLanguage(path)}\n`,footer='\n```'
  const budget=Math.min(24_000,Math.max(0,availableBytes-utf8Length(header+footer+48)))
  const body=fitUtf8(content,budget),truncated=body.length<content.length?'\n…（文件内容已按对话上限截断）':''
  return header+body+truncated+footer
}
async function insertFileIntoComposer(path:string,token:ActiveComposerToken|null=composerToken.value,knownContent?:string,label=path){
  const workspaceId=selectedId.value,conversationId=selectedConversationId.value,originalValue=input.value
  if(!workspaceId)return
  try{
    const content=knownContent??(await apiRequest<{content:string}>('/api/servers/'+workspaceId+'/file?path='+encodeURIComponent(path))).content
    if(selectedId.value!==workspaceId||selectedConversationId.value!==conversationId||input.value!==originalValue){flashSafeNotice('对话草稿或工作区已变化，请重新添加文件');return}
    const start=token?.range.start??(composerInput.value?.selectionStart??originalValue.length),end=token?.range.end??start
    const prefix=originalValue.slice(0,start),suffix=originalValue.slice(end)
    const spacerBefore=prefix&&!/\s$/u.test(prefix)?'\n\n':'',spacerAfter=suffix&&!/^\s/u.test(suffix)?'\n\n':''
    const available=60_000-utf8Length(prefix+spacerBefore+spacerAfter+suffix)
    const context=fileContext(path,content,available,label)
    if(!context||utf8Length(prefix+spacerBefore+context+spacerAfter+suffix)>60_000){flashSafeNotice('当前输入内容已接近上限，无法再加入该文件');return}
    applyComposerReplacement(prefix+spacerBefore+context+spacerAfter+suffix,(prefix+spacerBefore+context+spacerAfter).length)
    flashSafeNotice(`${label} 已添加到对话草稿`)
  }catch(error){flash('文件无法加入对话：'+String(error))}
}
async function selectComposerSuggestion(suggestion:ComposerSuggestion){
  const token=composerToken.value;if(!token)return
  if(suggestion.kind==='file'&&suggestion.filePath){await insertFileIntoComposer(suggestion.filePath,token,suggestion.filePath===activeFile.value?fileContent.value:undefined);return}
  if(suggestion.kind==='agent'){
    const workspaceId=selectedId.value,originalValue=input.value
    const selected=await pickChatAgent(suggestion.agentId==='default'?null:(suggestion.agentId??null));if(!selected)return
    if(selectedId.value!==workspaceId||input.value!==originalValue){flashSafeNotice('对话或草稿已变化，智能体已切换，但未覆盖当前输入');return}
  }
  const value=suggestion.skill?`${suggestion.skill.id}（${suggestion.skill.name}：${suggestion.skill.description}）`:suggestion.value
  const replacement=replaceComposerToken(input.value,token,value,{appendSpace:true});composerToken.value=null;applyComposerReplacement(replacement.value,replacement.cursor)
}
function handleComposerKeydown(event:KeyboardEvent){
  if(composerToken.value&&composerSuggestions.value.length){
    if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();const direction=event.key==='ArrowDown'?1:-1;composerSuggestionIndex.value=(composerSuggestionIndex.value+direction+composerSuggestions.value.length)%composerSuggestions.value.length;return}
    if((event.key==='Enter'||event.key==='Tab')&&!event.shiftKey&&!event.isComposing){event.preventDefault();void selectComposerSuggestion(composerSuggestions.value[composerSuggestionIndex.value]!);return}
    if(event.key==='Escape'){event.preventDefault();composerToken.value=null;return}
  }
  if(event.key==='Enter'&&!event.shiftKey&&!event.isComposing){event.preventDefault();void send()}
}
function handleComposerKeyup(event:KeyboardEvent){if(['ArrowLeft','ArrowRight','Home','End'].includes(event.key))updateComposerAssist()}
function mergeRunMessages(conversationId:string,persisted:Message[]){
  const run=chatRuns.value[conversationId];if(!run)return persisted
  const merged=[...persisted]
  if(!merged.some(message=>message.id===run.user.id))merged.push({...run.user})
  if((run.reply.content||run.phase!=='connecting')&&!merged.some(message=>message.id===run.reply.id))merged.push({...run.reply})
  return merged
}
function syncVisibleReply(conversationId:string){
  if(selectedConversationId.value!==conversationId)return
  const run=chatRuns.value[conversationId];if(!run)return
  const visible=messages.value.find(message=>message.id===run.reply.id)
  if(visible)Object.assign(visible,run.reply)
  else messages.value.push({...run.reply})
}
async function copyMessage(content:string){
  try{await writeClipboard(content);flashSafeNotice('消息已复制')}
  catch{flash('复制失败，请手动选择文本')}
}
function toggleMessageSearch(){showMessageSearch.value=!showMessageSearch.value;if(!showMessageSearch.value){messageSearch.value='';return}nextTick(()=>messageSearchInput.value?.focus())}
function stepMessageSearch(direction:number){
  const matches=messageSearchMatches.value;if(!matches.length)return
  activeSearchMatch.value=(activeSearchMatch.value+direction+matches.length)%matches.length
  const target=Array.from(scroller.value?.querySelectorAll<HTMLElement>('[data-message-id]')??[]).find(element=>element.dataset.messageId===matches[activeSearchMatch.value])
  target?.scrollIntoView({behavior:'smooth',block:'center'})
}
function isSearchMatch(id:string){return messageSearchMatches.value.includes(id)}
function isActiveSearchMatch(id:string){return messageSearchMatches.value[activeSearchMatch.value]===id}
function taskForMessage(message:Message){return message.task_id?tasks.value.find(task=>task.id===message.task_id):undefined}
function taskRiskLabel(risk:string){return risk==='high'?'高风险':risk==='medium'?'中风险':'低风险'}
function openTaskCenter(taskId=''){focusedTaskId.value=taskId;surface.value='automation'}
function upsertServer(item:ServerItem){const index=servers.value.findIndex(server=>server.id===item.id);if(index>=0)servers.value[index]=item;else servers.value.push(item)}
function upsertTask(task:TaskInfo){const index=tasks.value.findIndex(item=>item.id===task.id);if(index>=0)tasks.value[index]=task;else tasks.value.unshift(task)}
function handleMessageAction(action:string,message:Message){
  if(message.task_id||['查看任务详情','审阅执行计划','在镜像服运行'].includes(action)){openTaskCenter();return}
  void send(action)
}
function toConversationSummary(conversation:Conversation):ConversationSummary{return {...conversation,message_count:conversation.messages.length}}
async function loadConversationSummaries(serverId:string,selectPreferred=false){
  if(!serverId)return
  conversationsLoading.value=true
  try{
    const items=await apiRequest<ConversationSummary[]>('/api/servers/'+serverId+'/conversations')
    conversationsByServer.value={...conversationsByServer.value,[serverId]:items}
    if(selectPreferred){
      const remembered=selectedConversationByServer.value[serverId]
      const preferred=items.find(item=>item.id===remembered&&!item.archived)??items.find(item=>!item.archived)??items[0]
      if(preferred)await selectConversation(preferred)
      else{selectedConversationId.value='';messages.value=[]}
    }
  }catch(error){flash('对话任务加载失败：'+String(error))}
  finally{conversationsLoading.value=false}
}
async function loadAllConversationSummaries(){await Promise.all(servers.value.map(item=>loadConversationSummaries(item.id,false)))}
async function selectConversation(summary:ConversationSummary,hydrated?:Conversation){
  const crossWorkspace=summary.server_id!==selectedId.value
  const epoch=++conversationSelectionEpoch
  if(crossWorkspace)workspaceSelectionEpoch+=1
  saveCurrentDraft()
  resetConversationTransient();selectedConversationId.value='';messages.value=[];chatModelOverride.value=null;chatAgentOverride.value=null;chatReasoningEffort.value=null
  if(crossWorkspace){
    conversationCreationPending.value=ensureConversationPromises.has(summary.server_id)
    selectedId.value=summary.server_id;surface.value='control'
    tab.value=workspaceKind(servers.value.find(item=>item.id===summary.server_id)??emptyServer)==='project'?'files':'overview'
  }
  conversationSelectionPending.value=true
  try{
    const conversation=hydrated??await apiRequest<Conversation>('/api/conversations/'+summary.id)
    if(epoch!==conversationSelectionEpoch||selectedId.value!==conversation.server_id)return
    selectedConversationId.value=conversation.id
    selectedConversationByServer.value={...selectedConversationByServer.value,[conversation.server_id]:conversation.id}
    chatModelOverride.value=conversation.model_binding??null
    chatAgentOverride.value=conversation.agent_override??null
    chatReasoningEffort.value=conversation.reasoning_effort??null
    messages.value=mergeRunMessages(conversation.id,conversation.messages.map(message=>({...message})))
    restoreCurrentDraft()
    if(conversation.unread){
      await apiRequest('/api/conversations/'+conversation.id,{method:'PUT',body:JSON.stringify({unread:false})})
      await loadConversationSummaries(conversation.server_id,false)
    }
    await nextTick();scrollChat(false)
  }catch(error){if(epoch===conversationSelectionEpoch)flash('对话打开失败：'+String(error))}
  finally{if(epoch===conversationSelectionEpoch)conversationSelectionPending.value=false}
}
async function createConversation(serverId=selectedId.value,title='新对话'){
  if(!serverId)return null
  const serverDraft=draftKey(serverId,'')
  const transferServerDraft=selectedId.value===serverId&&!selectedConversationId.value
  const startedWorkspaceEpoch=workspaceSelectionEpoch,startedConversationEpoch=conversationSelectionEpoch
  try{
    const conversation=await apiRequest<Conversation>('/api/servers/'+serverId+'/conversations',{method:'POST',body:JSON.stringify({title})})
    const items=conversationsByServer.value[serverId]??[]
    conversationsByServer.value={...conversationsByServer.value,[serverId]:[toConversationSummary(conversation),...items]}
    if(selectedId.value===serverId&&workspaceSelectionEpoch===startedWorkspaceEpoch&&conversationSelectionEpoch===startedConversationEpoch)await selectConversation(toConversationSummary(conversation),conversation)
    if(transferServerDraft){const transferableDraft=conversationDrafts.value[serverDraft]??'';const conversationDraft=draftKey('',conversation.id);const next={...conversationDrafts.value};delete next[serverDraft];if(transferableDraft)next[conversationDraft]=transferableDraft;conversationDrafts.value=next;if(selectedConversationId.value===conversation.id)setComposerInput(transferableDraft)}
    return conversation
  }catch(error){flash('新建对话失败：'+String(error));return null}
}
async function ensureConversation(serverId=selectedId.value):Promise<EnsuredConversation>{
  if(!serverId||serverId!==selectedId.value)return{id:'',serverId,selected:false}
  if(selectedConversationId.value)return{id:selectedConversationId.value,serverId,selected:true}
  const existing=ensureConversationPromises.get(serverId);if(existing)return existing
  if(conversationSelectionPending.value)return{id:'',serverId,selected:false}
  const pending=(async()=>{const conversation=await createConversation(serverId);if(!conversation)return{id:'',serverId,selected:false};return{id:conversation.id,serverId,selected:selectedId.value===serverId&&selectedConversationId.value===conversation.id}})()
  ensureConversationPromises.set(serverId,pending)
  if(selectedId.value===serverId)conversationCreationPending.value=true
  try{return await pending}finally{if(ensureConversationPromises.get(serverId)===pending)ensureConversationPromises.delete(serverId);if(selectedId.value===serverId)conversationCreationPending.value=false}
}
async function updateConversation(summary:ConversationSummary,patch:Record<string,unknown>){
  const conversation=await apiRequest<ConversationSummary>('/api/conversations/'+summary.id,{method:'PUT',body:JSON.stringify(patch)})
  await loadConversationSummaries(summary.server_id,false)
  return conversation
}
async function handleConversationAction(action:ConversationAction,summary:ConversationSummary){
  try{
    if(action==='rename'||action==='group'||action==='delete'){
      conversationDialog.value={kind:action,conversation:summary}
      conversationDialogValue.value=action==='rename'?summary.title:(summary.group??'')
      return
    }
    if(action==='pin')await updateConversation(summary,{pinned:!summary.pinned})
    if(action==='unread')await updateConversation(summary,{unread:!summary.unread})
    if(action==='archive'){
      await updateConversation(summary,{archived:!summary.archived})
      if(summary.id===selectedConversationId.value&& !summary.archived){saveCurrentDraft();selectedConversationId.value='';messages.value=[];resetConversationTransient();restoreCurrentDraft()}
    }
    if(action==='fork'){
      const fork=await apiRequest<Conversation>('/api/conversations/'+summary.id+'/fork',{method:'POST'})
      await loadConversationSummaries(summary.server_id,false)
      await selectConversation(toConversationSummary(fork))
    }
  }catch(error){flash('对话操作失败：'+String(error))}
}
async function submitConversationDialog(){
  const dialog=conversationDialog.value;if(!dialog)return
  try{
    if(dialog.kind==='delete'){
      chatControllers.get(dialog.conversation.id)?.abort();pendingSteerIds.delete(dialog.conversation.id);setQueue(dialog.conversation.id,[])
      await apiRequest('/api/conversations/'+dialog.conversation.id,{method:'DELETE'})
      discardConversationDraft(dialog.conversation.id)
      if(selectedConversationId.value===dialog.conversation.id){selectedConversationId.value='';messages.value=[];resetConversationTransient();restoreCurrentDraft()}
    }else if(dialog.kind==='rename'){
      const title=conversationDialogValue.value.trim();if(!title)return
      await updateConversation(dialog.conversation,{title})
    }else await updateConversation(dialog.conversation,{group:conversationDialogValue.value.trim()})
    conversationDialog.value=null
    await loadConversationSummaries(dialog.conversation.server_id,false)
  }catch(error){flash('对话操作失败：'+String(error))}
}
function queueFor(conversationId:string){return chatQueues.value[conversationId]??[]}
function setQueue(conversationId:string,items:QueuedChat[]){const next={...chatQueues.value};if(items.length)next[conversationId]=items;else delete next[conversationId];chatQueues.value=next}
function enqueueChat(item:QueuedChat,front=false){const items=queueFor(item.conversationId).filter(existing=>existing.id!==item.id);setQueue(item.conversationId,front?[item,...items]:[...items,item])}
function removeQueuedChat(conversationId:string,itemId:string){setQueue(conversationId,queueFor(conversationId).filter(item=>item.id!==itemId));if(editingQueuedId.value===itemId){editingQueuedId.value='';editingQueuedContent.value=''}}
function beginQueuedEdit(item:QueuedChat){editingQueuedId.value=item.id;editingQueuedContent.value=item.content;nextTick(()=>document.querySelector<HTMLTextAreaElement>('[data-queue-editor="'+item.id+'"]')?.focus())}
function saveQueuedEdit(item:QueuedChat){const raw=editingQueuedContent.value.trim();if(!raw){flashSafeNotice('排队内容不能为空');return}const prepared=prepareChatSend(raw);if(!prepared)return;setQueue(item.conversationId,queueFor(item.conversationId).map(existing=>existing.id===item.id?{...existing,content:prepared.content,modelOverride:prepared.modelOverride??existing.modelOverride,agentOverride:prepared.agentOverride??existing.agentOverride}:existing));editingQueuedId.value='';editingQueuedContent.value=''}
function snapshotBinding(binding:ModelBinding|null){return binding?{...binding}:null}
function newQueuedChat(conversationId:string,serverId:string,content:string):QueuedChat{return{id:crypto.randomUUID(),conversationId,serverId,content,mode:'queue',createdAt:new Date().toISOString(),modelOverride:snapshotBinding(chatModelOverride.value),agentOverride:chatAgentOverride.value,reasoningEffort:chatReasoningEffort.value}}
async function historyForChat(item:QueuedChat){
  if(selectedConversationId.value===item.conversationId)return messages.value.filter(message=>!message.error&&!message.interrupted).slice(-20).map(message=>({role:message.role,content:message.content}))
  const conversation=await apiRequest<Conversation>('/api/conversations/'+item.conversationId)
  return conversation.messages.slice(-20).map(message=>({role:message.role,content:message.content}))
}
async function publishQueuedChat(conversationId:string,itemId:string){
  const item=queueFor(conversationId).find(candidate=>candidate.id===itemId);if(!item)return
  const run=chatRuns.value[conversationId]
  if(run&&['connecting','streaming','stopping'].includes(run.phase))return
  removeQueuedChat(conversationId,itemId)
  await runChatItem(item)
}
async function runNextQueuedChat(conversationId:string){
  const run=chatRuns.value[conversationId];if(run&&['connecting','streaming','stopping'].includes(run.phase))return
  const next=queueFor(conversationId)[0];if(!next)return
  await publishQueuedChat(conversationId,next.id)
}
async function steerQueuedChat(item:QueuedChat){
  const steered={...item,mode:'steer' as const}
  enqueueChat(steered,true)
  const run=chatRuns.value[item.conversationId]
  if(run&&['connecting','streaming','stopping'].includes(run.phase)){
    pendingSteerIds.set(item.conversationId,item.id)
    if(run.phase!=='stopping'){run.phase='stopping';chatControllers.get(item.conversationId)?.abort()}
    flashSafeNotice('正在停止当前生成，随后发送这条引导')
    return
  }
  await publishQueuedChat(item.conversationId,item.id)
}
async function sendQueuedNow(item:QueuedChat){
  const run=chatRuns.value[item.conversationId]
  if(run&&['connecting','streaming','stopping'].includes(run.phase)){await steerQueuedChat(item);return}
  await publishQueuedChat(item.conversationId,item.id)
}
async function runChatItem(item:QueuedChat){
  const {conversationId,serverId,content}=item
  const outgoingContent=item.mode==='steer'?`[引导消息]\n这是对当前任务的补充引导，请基于刚才的目标调整后续执行：\n${content}`:content
  const userMessage:Message={id:crypto.randomUUID(),role:'user',content:outgoingContent,time:now()}
  const reply:Message={id:crypto.randomUUID(),role:'assistant',content:'',time:now(),streaming:true,retryContent:content}
  const controller=new AbortController()
  chatRuns.value={...chatRuns.value,[conversationId]:{conversationId,serverId,phase:'connecting',user:userMessage,reply}}
  chatControllers.set(conversationId,controller)
  let history:{role:'assistant'|'user';content:string}[]
  try{history=await historyForChat(item)}catch(error){chatControllers.delete(conversationId);const nextRuns={...chatRuns.value};if(nextRuns[conversationId]?.user.id===userMessage.id)delete nextRuns[conversationId];chatRuns.value=nextRuns;enqueueChat(item,true);flash('读取对话上下文失败：'+String(error));return}
  if(selectedConversationId.value===conversationId)messages.value.push({...userMessage})
  if(selectedConversationId.value===conversationId){await nextTick();scrollChat()}
  let completed=false,hasOutput=false
  try {
    await postSse('/api/chat/stream',{server_id:serverId,conversation_id:conversationId,message:outgoingContent,history,model_override:item.modelOverride,agent_override:item.agentOverride,reasoning_effort:item.reasoningEffort},{
      onMeta:meta=>{const run=chatRuns.value[conversationId];if(!run||run.user.id!==userMessage.id)return;run.phase='streaming';run.reply.fallback=meta.fallback;syncVisibleReply(conversationId)},
      onDelta:text=>{const run=chatRuns.value[conversationId];if(!run||run.user.id!==userMessage.id)return;hasOutput=true;run.phase='streaming';run.reply.content+=text;syncVisibleReply(conversationId);if(selectedConversationId.value===conversationId)throttledScroll()},
      onError:message=>{const run=chatRuns.value[conversationId];if(!run||run.user.id!==userMessage.id)return;run.reply.warning=message;syncVisibleReply(conversationId)},
      onDone:done=>{const run=chatRuns.value[conversationId];if(!run||run.user.id!==userMessage.id)return;completed=true;run.reply.time=done.time;run.reply.actions=done.actions;run.reply.task_id=done.task?.id;run.reply.streaming=false;if(done.task){const index=tasks.value.findIndex(task=>task.id===done.task?.id);if(index>=0)tasks.value[index]=done.task;else tasks.value.unshift(done.task)}syncVisibleReply(conversationId)},
    },controller.signal)
    if(!completed)throw new Error(hasOutput?'响应流提前结束':'未收到模型响应')
    chatControllers.delete(conversationId)
    const completedRun=chatRuns.value[conversationId]
    const nextRuns={...chatRuns.value};delete nextRuns[conversationId];chatRuns.value=nextRuns
    await loadConversationSummaries(serverId,false)
    if(selectedConversationId.value===conversationId){
      try{
        const conversation=await apiRequest<Conversation>('/api/conversations/'+conversationId)
        messages.value=conversation.messages.map(message=>({...message}))
        if(completedRun?.reply.warning){const last=messages.value.at(-1);if(last?.role==='assistant')last.warning=completedRun.reply.warning}
      }catch{}
    }
  }catch(error){
    const run=chatRuns.value[conversationId]
    if(run&&run.user.id===userMessage.id){
      const interrupted=controller.signal.aborted
      run.phase=interrupted?'interrupted':'failed';run.reply.streaming=false;run.reply.interrupted=interrupted;run.reply.error=!interrupted;run.reply.retryContent=content
      if(!run.reply.content)run.reply.content=interrupted?'已停止生成。':'本次请求未完成。'
      run.reply.warning=interrupted?'内容尚未保存，可重新生成。':String(error);syncVisibleReply(conversationId)
    }
    chatControllers.delete(conversationId)
    if(!controller.signal.aborted)flash('对话请求失败：'+String(error))
  }
  if(selectedConversationId.value===conversationId){await nextTick();scrollChat()}
  if(completed){pendingSteerIds.delete(conversationId);void runNextQueuedChat(conversationId);return}
  const steerId=pendingSteerIds.get(conversationId)
  if(controller.signal.aborted&&steerId){pendingSteerIds.delete(conversationId);void publishQueuedChat(conversationId,steerId)}
}
function prepareChatSend(raw:string):{content:string;modelOverride?:ModelBinding;agentOverride?:string}|null{
  const shortcut=parseModelShortcut(raw);if(!shortcut)return{content:raw}
  if(!shortcut.content.trim()){flashSafeNotice('模型快捷指令中的内容不能为空');return null}
  const wanted=shortcut.model.toLocaleLowerCase('zh-CN')
  const available=(aiSettings.value?.providers??[]).filter(provider=>provider.enabled).flatMap(provider=>provider.models.filter(model=>model.enabled).map(model=>({provider,model})))
  let matches=available.filter(item=>item.model.id.toLocaleLowerCase('zh-CN')===wanted)
  if(matches.length!==1){const qualified=available.filter(item=>[`${item.provider.id}/${item.model.id}`,`${item.provider.name}/${item.model.id}`].some(label=>label.toLocaleLowerCase('zh-CN')===wanted));if(qualified.length)matches=qualified}
  if(matches.length!==1){flashSafeNotice(matches.length?'模型名不唯一，请使用“提供商/模型名”':'没有找到已启用的模型：'+shortcut.model);return null}
  const match=matches[0]!
  flashSafeNotice(`本条消息将快捷发送给 ${match.provider.name} / ${match.model.id}`)
  return{content:shortcut.content.trim(),modelOverride:{provider_id:match.provider.id,model_id:match.model.id},agentOverride:'default'}
}
async function send(preset?:string) {
  const raw=(preset??input.value).trim();if(!raw)return
  if(!selectedId.value){flash(workspaceMode.value==='project'?'请先创建或选择项目':'请先创建或选择服务器项目');return}
  if(conversationSelectionPending.value){flashSafeNotice('正在切换对话，请稍候再发送');return}
  const serverId=selectedId.value
  if(!selectedConversationId.value&&ensureConversationPromises.has(serverId)){flashSafeNotice('正在创建对话，本条内容仍保留在草稿中');return}
  const outgoingDraftKey=draftKey(serverId,selectedConversationId.value)
  const prepared=prepareChatSend(raw);if(!prepared)return
  const policyMessage=fullAccessAgentPolicyMessage(prepared.agentOverride==='default'?null:activeAgent.value)
  if(policyMessage){flashSafeNotice(policyMessage);return}
  if(preset===undefined){input.value='';saveCurrentDraft()}
  const ensured=await ensureConversation(serverId);if(!ensured.selected||!ensured.id||selectedId.value!==serverId){if(preset===undefined)restoreUnsentDraft(ensured.id?draftKey('',ensured.id):outgoingDraftKey,raw);flashSafeNotice('工作区已变化或对话创建失败，本条内容已恢复到草稿');return}
  const conversationId=ensured.id
  const item=newQueuedChat(conversationId,serverId,prepared.content)
  if(prepared.modelOverride)item.modelOverride=prepared.modelOverride
  if(prepared.agentOverride)item.agentOverride=prepared.agentOverride
  const run=chatRuns.value[conversationId]
  enqueueChat(item)
  if(run&&['connecting','streaming','stopping'].includes(run.phase)){
    flashSafeNotice(`已加入当前对话队列（第 ${queueFor(conversationId).length} 条）`)
    return
  }
  await runNextQueuedChat(conversationId)
}
function stopGeneration(){
  const conversationId=selectedConversationId.value;const run=chatRuns.value[conversationId];if(!run||!['connecting','streaming'].includes(run.phase))return
  run.phase='stopping';chatControllers.get(conversationId)?.abort()
}
async function retryMessage(message:Message){
  const content=message.retryContent?.trim();const conversationId=selectedConversationId.value;if(!content||!conversationId)return
  const run=chatRuns.value[conversationId]
  if(run&&['failed','interrupted'].includes(run.phase)){
    messages.value=messages.value.filter(item=>item.id!==run.user.id&&item.id!==run.reply.id)
    const nextRuns={...chatRuns.value};delete nextRuns[conversationId];chatRuns.value=nextRuns
  }
  await send(content)
}
async function api(url:string, options?:RequestInit){return apiRequest<any>(url,options)}
const remoteConnections=computed(()=>(uiSettings.value?.connections??[]).filter(connection=>connection.enabled))
const selectedLocationLabel=computed(()=>createForm.value.location==='local'?(systemInfo.value?.data_dir||'数据目录尚未获取'):'远程服务器（暂未支持）')
const projectLocationLabel=computed(()=>projectForm.value.location==='local'?'本机项目工作区 · data/projects':'远程项目（暂未支持）')
const openDirectoryModeLabel=computed(()=>workspaceMode.value==='project'?'项目':'服务器')
const openDirectoryDetectedRows=computed(()=>{
  const detected=openDirectorySummary.value?.detected
  if(!detected||typeof detected!=='object')return[]
  const labels:Record<string,string>={kind:'类型',name:'名称',core:'核心',version:'Minecraft 版本',port:'端口',memory_gb:'内存上限',max_players:'最大玩家',core_ready:'核心状态',eula_accepted:'EULA',launch_jar:'启动核心',config_files:'配置文件',jar_candidates:'JAR 候选',script_candidates:'启动脚本'}
  const visibleKeys=['kind','name','core','version','port','memory_gb','max_players','core_ready','eula_accepted','launch_jar','config_files','jar_candidates','script_candidates']
  return visibleKeys.flatMap(key=>{const value=detected[key];return value===null||value===undefined?[]:[{key,label:labels[key]??key,value:formatOpenDirectoryValue(value)}]})
})
function formatOpenDirectoryValue(value:unknown):string{
  if(typeof value==='boolean')return value?'已检测到':'未检测到'
  if(Array.isArray(value))return value.map(item=>formatOpenDirectoryValue(item)).join('、')
  if(value&&typeof value==='object'){
    try{return JSON.stringify(value)}catch{return String(value)}
  }
  return String(value)
}
function openExistingDirectory(){
  if(openingDirectory.value)return
  openDirectoryPath.value='';openDirectoryName.value='';openDirectoryError.value='';openDirectorySummary.value=null
  showOpenDirectory.value=true
}
function closeOpenDirectory(){if(!openingDirectory.value)showOpenDirectory.value=false}
async function importExistingDirectory(){
  const path=openDirectoryPath.value.trim()
  if(openingDirectory.value)return
  if(!path){openDirectoryError.value='请输入已有目录的绝对路径';return}
  openDirectoryError.value='';openingDirectory.value=true
  try{
    const body:{path:string;kind:WorkspaceKind;name?:string}={path,kind:workspaceMode.value}
    const name=openDirectoryName.value.trim();if(name)body.name=name
    const response=await api('/api/servers/import',{method:'POST',body:JSON.stringify(body)}) as OpenDirectoryResponse
    if(!response?.server?.id)throw new Error('后端未返回有效工作区')
    response.warnings=Array.isArray(response.warnings)?response.warnings.filter(item=>typeof item==='string'):[]
    response.files=Array.isArray(response.files)?response.files.filter(item=>typeof item==='string'):[]
    openDirectorySummary.value=response
    upsertServer(response.server)
    showOpenDirectory.value=false
    await selectServer(response.server.id)
    if(workspaceKind(response.server)==='project')tab.value='overview'
    const warningCount=response.warnings?.length??0
    const detected=Object.entries(response.detected??{}).filter(([,value])=>value!==null&&value!==undefined).slice(0,3).map(([key,value])=>`${key}: ${formatOpenDirectoryValue(value)}`).join(' · ')
    flash(`已打开${workspaceKind(response.server)==='project'?'项目':'服务器'}“${response.server.name}”${detected?` · ${detected}`:''}${warningCount?` · ${warningCount} 条提醒`:''}`)
  }catch(error){openDirectoryError.value=error instanceof ApiError?error.message:String(error)}
  finally{openingDirectory.value=false}
}
async function switchWorkspaceMode(mode:WorkspaceKind){
  if(workspaceMode.value===mode)return
  saveCurrentDraft()
  workspaceMode.value=mode
  localStorage.setItem(WORKSPACE_MODE_KEY,mode)
  if(mode==='project'&&['mirror','community'].includes(surface.value))surface.value='control'
  const next=servers.value.find(item=>workspaceKind(item)===mode)
  selectedId.value='';selectedConversationId.value='';messages.value=[];activeFile.value='';fileEntries.value=[];terminal.value=[]
  disconnectLogStream();stopDownloadPolling()
  if(next)await selectServer(next.id)
  else tab.value=mode==='project'?'files':'overview'
}
function syncCreateOptions(){
  if(!availableCores.value.includes(createForm.value.core)){
    if(importedTemplateTitle.value&&createForm.value.core)return
    createForm.value.core=availableCores.value[0]??''
  }
  const versions=createMinecraftVersions.value
  if(!versions.includes(createForm.value.version)){
    if(importedTemplateTitle.value&&createForm.value.version)return
    createForm.value.version=versions[0]??''
  }
}
async function loadSystemInfo(){
  systemState.value='loading';systemError.value=''
  try{systemInfo.value=await api('/api/system');systemState.value='ready';syncCreateOptions();return true}
  catch(error){systemInfo.value=null;systemState.value='error';systemError.value=String(error);return false}
}
async function loadCreateCatalog(){
  try{catalogCores.value=await api('/api/catalog/cores')}catch{catalogCores.value=[]}
  syncCreateOptions()
}
async function openCreate(){
  importedTemplateTitle.value=''
  showCreate.value=true;createStep.value=1;javaInstallState.value='idle';javaInstallError.value=''
  if(workspaceMode.value==='project'){
    projectForm.value={name:'',location:'local'}
    await loadSystemInfo()
    return
  }
  await Promise.all([loadSystemInfo(),loadCreateCatalog(),loadUi().catch(()=>{})])
}
async function loadServerTemplate(template:ServerTemplate){
  importedTemplateTitle.value=template.title
  showCreate.value=true;createStep.value=2;javaInstallState.value='idle';javaInstallError.value='';surface.value='control'
  await Promise.all([loadSystemInfo(),loadCreateCatalog(),loadUi().catch(()=>{})])
  createForm.value={
    name:template.server.name,
    location:'local',
    core:template.server.core,
    version:template.server.minecraft_version,
    memory_gb:template.server.memory_gb,
    port:template.server.port,
    eula_accepted:false,
  }
  const url=new URL(window.location.href);url.searchParams.delete('cloud');window.history.replaceState({},'',url.pathname+url.search+url.hash)
  flash(`已载入“${template.title}”；请确认本机兼容性与 EULA 后再创建`)
}
async function importServerTemplate(event:Event){
  const input=event.target as HTMLInputElement;const file=input.files?.[0];input.value='';if(!file)return
  try{
    if(file.size>MAX_SERVER_TEMPLATE_FILE_BYTES)throw new Error('配置文件不能超过 64 KiB')
    await loadServerTemplate(parseServerTemplateManifest(await file.text()))
  }catch(error){flash('导入配置失败：'+String(error))}
}
async function installJava(){
  const major=systemInfo.value?.recommended_java
  if(!major||!systemInfo.value?.java_install_supported||javaInstallState.value==='installing')return
  javaInstallState.value='installing';javaInstallError.value=''
  try{
    const response=await fetch(API_BASE+'/api/runtime/java/install',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({major})})
    if(!response.ok)throw new Error((await response.text())||`请求失败（HTTP ${response.status}）`)
    await loadSystemInfo()
    if(javaReady.value)javaInstallState.value='success'
    else{javaInstallState.value='error';javaInstallError.value='安装接口已返回，但刷新后 Java 仍未就绪'}
  }catch(error){javaInstallState.value='error';javaInstallError.value=String(error)}
}
async function advanceCreateStep(){
  if(createStep.value===2&&parameterIssue.value){flash(parameterIssue.value);return}
  if(createStep.value===3&&environmentIssue.value){flash(environmentIssue.value);return}
  createStep.value++
}
async function createProject(){
  const name=projectForm.value.name.trim()
  if(creating.value||!name)return
  creating.value=true
  try{
    const response=await api('/api/projects',{method:'POST',body:JSON.stringify({name,location:projectForm.value.location})}) as {project:ServerItem;directory:string}
    upsertServer(response.project)
    selectedId.value=response.project.id
    selectedConversationId.value='';messages.value=[]
    showCreate.value=false;surface.value='control';tab.value='files';activeFile.value=''
    await loadFiles()
    flash('项目文件夹已创建，可以开始编辑文件或新建对话任务')
  }catch(error){flash('创建项目失败：'+String(error))}
  finally{creating.value=false}
}
async function createNewServer(){
  if(creating.value)return
  if(parameterIssue.value){createStep.value=2;flash(parameterIssue.value);return}
  if(environmentIssue.value){createStep.value=3;flash(environmentIssue.value);return}
  creating.value=true
  let created:{server:ServerItem;provision_task?:TaskInfo}|null=null
  try{
    const response=await api('/api/servers',{method:'POST',body:JSON.stringify(createForm.value)}) as {server:ServerItem;provision_task?:TaskInfo}
    created=response
    upsertServer(response.server)
    if(response.provision_task)upsertTask(response.provision_task)
    selectedId.value=response.server.id;showCreate.value=false;surface.value='control';tab.value='overview'
    flash('服务器已创建，正在下载并校验服务端核心')
  }catch(error){flash('创建失败：'+String(error))}finally{creating.value=false}
  if(!created)return
  const conversation=await createConversation(created.server.id,'服务器初始化')
  if(!conversation)flash('服务器已创建并正在初始化；初始化对话暂未建立，可稍后新建对话')
}
async function createSmartServer(){
  if(creating.value||!createForm.value.name.trim())return;creating.value=true
  try{
    const data=await api('/api/servers/plan',{method:'POST',body:JSON.stringify({name:createForm.value.name,location:createForm.value.location})})
    servers.value.push(data.server);selectedId.value=data.server.id;showCreate.value=false;surface.value='control';tab.value='overview'
    const summary=toConversationSummary(data.conversation)
    conversationsByServer.value={...conversationsByServer.value,[data.server.id]:[summary]}
    await selectConversation(summary);flash('服务器项目已加入列表，继续通过对话完成开服规划')
  }catch(error){flash('智能创建失败：'+String(error))}finally{creating.value=false}
}
async function retryBackend(){await Promise.all([loadDashboard(),loadSystemInfo()])}
function openDeleteServer(item:ServerItem){deleteServerTarget.value=item;deleteServerFiles.value=false;deleteServerStep.value=1;deleteServerConfirmation.value=''}
async function advanceDeleteServer(){if(!deleteServerTarget.value)return;if(deleteServerFiles.value){deleteServerStep.value=2;return}await executeDeleteServer()}
function stopWorkspaceChats(serverId:string){
  const conversationIds=new Set((conversationsByServer.value[serverId]??[]).map(item=>item.id))
  for(const [conversationId,items] of Object.entries(chatQueues.value)){
    if(conversationIds.has(conversationId)||items.some(item=>item.serverId===serverId))setQueue(conversationId,[])
  }
  for(const run of Object.values(chatRuns.value)){
    if(run.serverId!==serverId)continue
    pendingSteerIds.delete(run.conversationId)
    chatControllers.get(run.conversationId)?.abort()
  }
}
async function executeDeleteServer(){
  const target=deleteServerTarget.value;if(!target)return
  if(deleteServerFiles.value&&deleteServerConfirmation.value!=='delete all')return
  busy.value=true
  let deletionCommitted=false
  try{
    deletedWorkspaceIds.add(target.id)
    stopWorkspaceChats(target.id)
    try{
      await api('/api/servers/'+target.id,{method:'DELETE',body:JSON.stringify({delete_files:deleteServerFiles.value,confirmation:deleteServerFiles.value?deleteServerConfirmation.value:null})})
    }catch(error){
      if(!(error instanceof ApiError&&error.status===404))throw error
    }
    deletionCommitted=true
    servers.value=servers.value.filter(item=>item.id!==target.id)
    tasks.value=tasks.value.filter(item=>item.server_id!==target.id)
    if(focusedTaskId.value&&!tasks.value.some(item=>item.id===focusedTaskId.value))focusedTaskId.value=''
    const next={...conversationsByServer.value};delete next[target.id];conversationsByServer.value=next
    deleteServerTarget.value=null
    if(selectedId.value===target.id){selectedId.value=visibleServers.value[0]?.id??'';selectedConversationId.value='';messages.value=[];if(selectedId.value)await selectServer(selectedId.value)}
    const noun=workspaceKind(target)==='project'?'项目':'服务器'
    flash(deleteServerFiles.value?`${noun}与磁盘文件已删除`:`${noun}已从列表移除，磁盘文件已保留`)
    void loadDashboard(false)
  }catch(error){if(!deletionCommitted)deletedWorkspaceIds.delete(target.id);flash('删除失败：'+String(error))}finally{busy.value=false}
}

function flash(message:string){notice.value=message;window.setTimeout(()=>{if(notice.value===message)notice.value=''},2600)}
async function loadLogs(){const id=selectedId.value;if(!id||isProject.value){terminal.value=[];return}try{const data=await api('/api/servers/'+id+'/logs');if(selectedId.value===id)terminal.value=data.lines}catch{}}
const logSocket=ref<WebSocket|null>(null),logStreamLive=ref(false),termScroller=ref<HTMLElement|null>(null)
function scrollTerminal(){nextTick(()=>{termScroller.value?.scrollTo({top:termScroller.value.scrollHeight})})}
function disconnectLogStream(){logStreamLive.value=false;if(logSocket.value){logSocket.value.onclose=null;logSocket.value.close();logSocket.value=null}}
function connectLogStream(){
  disconnectLogStream()
  const id=selectedId.value
  if(!id||isProject.value)return
  try{
    const socket=new WebSocket(API_BASE.replace(/^http/,'ws')+'/api/servers/'+id+'/ws/logs')
    socket.onopen=()=>{if(selectedId.value!==id){socket.close();return}terminal.value=[];logStreamLive.value=true}
    socket.onmessage=event=>{terminal.value.push(String(event.data));if(terminal.value.length>1200)terminal.value.splice(0,terminal.value.length-1200);scrollTerminal()}
    socket.onclose=()=>{if(logSocket.value===socket){logStreamLive.value=false;logSocket.value=null;loadLogs()}}
    socket.onerror=()=>{socket.close()}
    logSocket.value=socket
  }catch{logStreamLive.value=false;loadLogs()}
}
async function loadFiles(path=''){const id=selectedId.value;if(!id){fileEntries.value=[];currentPath.value='';parentPath.value=null;return}try{const data=await api('/api/servers/'+id+'/files?path='+encodeURIComponent(path));if(selectedId.value!==id)return;fileEntries.value=data.entries;currentPath.value=data.path;parentPath.value=data.parent}catch(error){flash('目录读取失败：'+String(error))}}
async function openFile(path:string){const id=selectedId.value,epoch=++fileOperationEpoch;if(!id)return;try{const data=await api('/api/servers/'+id+'/file?path='+encodeURIComponent(path));if(selectedId.value!==id||epoch!==fileOperationEpoch)return;activeFile.value=data.path;fileContent.value=data.content;fileReadonly.value=data.readonly;fileSelection.value=null}catch(error){if(epoch===fileOperationEpoch)flash('文件读取失败：'+String(error))}}
async function openEntry(entry:FileEntry){if(entry.kind==='folder'){fileOperationEpoch+=1;activeFile.value='';fileContent.value='';fileSelection.value=null;await loadFiles(entry.path)}else await openFile(entry.path)}
async function fileTransferError(response:Response){
  const body=await response.text()
  if(!body)return `请求失败（HTTP ${response.status}）`
  try{
    const payload=JSON.parse(body) as Record<string,unknown>
    for(const key of ['message','detail','error'])if(typeof payload[key]==='string'&&payload[key].trim())return payload[key] as string
  }catch{}
  return body
}
function fileTransferUnavailable(){flashSafeNotice('请先选择一个工作区')}
function triggerFileUpload(){if(busy.value)return;if(!canTransferFiles.value){fileTransferUnavailable();return}fileUploadInput.value?.click()}
async function uploadWorkspaceFile(event:Event){
  const input=event.target as HTMLInputElement,file=input.files?.[0]
  input.value=''
  if(!file)return
  if(!canTransferFiles.value){fileTransferUnavailable();return}
  const id=selectedId.value,targetPath=currentPath.value
  busy.value=true
  try{
    const formData=new FormData();formData.append('file',file);formData.append('path',targetPath)
    const response=await fetch(API_BASE+'/api/servers/'+encodeURIComponent(id)+'/file/upload',{method:'POST',body:formData})
    if(!response.ok)throw new Error(await fileTransferError(response))
    if(selectedId.value!==id)return
    await loadFiles(targetPath);flash('文件已上传')
  }catch(error){flash('上传失败：'+String(error))}finally{busy.value=false}
}
async function downloadCurrentFile(path=activeFile.value,kind:'file'|'folder'='file'){
  if(busy.value)return
  if(!canTransferFiles.value){fileTransferUnavailable();return}
  if(kind==='folder'){flashSafeNotice('目录不能下载，请选择文件');return}
  if(!path){flashSafeNotice('请先选择要下载的文件');return}
  const id=selectedId.value;busy.value=true
  try{
    const response=await fetch(API_BASE+'/api/servers/'+encodeURIComponent(id)+'/file/download?path='+encodeURIComponent(path))
    if(!response.ok)throw new Error(await fileTransferError(response))
    const url=URL.createObjectURL(await response.blob()),link=document.createElement('a')
    link.href=url;link.download=path.split('/').at(-1)||'download';document.body.append(link);link.click();link.remove();window.setTimeout(()=>URL.revokeObjectURL(url),0)
    flash('文件下载已开始')
  }catch(error){flash('下载失败：'+String(error))}finally{busy.value=false}
}
async function saveCurrentFile(){if(!selectedId.value||!activeFile.value||fileReadonly.value)return;busy.value=true;try{await api('/api/servers/'+selectedId.value+'/file',{method:'PUT',body:JSON.stringify({path:activeFile.value,content:fileContent.value})});flash(activeFile.value+' 已保存')}catch(error){flash('保存失败：'+String(error))}finally{busy.value=false}}
function cancelNewEntry(){showNewFile.value=false;showNewFolder.value=false;newFileName.value='';newFolderName.value=''}
function openNewEntry(kind:'file'|'folder'){
  showNewFile.value=kind==='file';showNewFolder.value=kind==='folder';newFileName.value='';newFolderName.value=''
  nextTick(()=>document.querySelector<HTMLInputElement>('.new-entry-form input')?.focus())
}
async function submitNewEntry(){
  if(showNewFile.value){if(newFileName.value.trim())await createFile();else cancelNewEntry()}
  else if(showNewFolder.value){if(newFolderName.value.trim())await createFolder();else cancelNewEntry()}
}
async function createFolder(){const name=newFolderName.value.trim();if(!selectedId.value||!name||busy.value)return;const path=[currentPath.value,name].filter(Boolean).join('/');busy.value=true;try{await api('/api/servers/'+selectedId.value+'/directory',{method:'POST',body:JSON.stringify({path})});newFolderName.value='';showNewFolder.value=false;await loadFiles(currentPath.value);flash('目录已创建')}catch(error){flash('创建失败：'+String(error))}finally{busy.value=false}}
async function createFile(){const name=newFileName.value.trim();if(!selectedId.value||!name||busy.value)return;const path=[currentPath.value,name].filter(Boolean).join('/');busy.value=true;try{await api('/api/servers/'+selectedId.value+'/file',{method:'PUT',body:JSON.stringify({path,content:''})});newFileName.value='';showNewFile.value=false;await loadFiles(currentPath.value);await openFile(path);flash('文件已创建')}catch(error){flash('创建文件失败：'+String(error))}finally{busy.value=false}}
function openFileContextMenu(event:MouseEvent,entry:FileEntry|null=null){event.preventDefault();event.stopPropagation();fileContextMenu.value={entry,x:Math.max(8,Math.min(event.clientX,window.innerWidth-190)),y:Math.max(8,Math.min(event.clientY,window.innerHeight-250))}}
function openContextEntry(){const entry=fileContextMenu.value?.entry;fileContextMenu.value=null;if(entry)void openEntry(entry)}
async function createFromContext(kind:'file'|'folder',entry?:FileEntry){
  const base=entry?.kind==='folder'?entry.path:currentPath.value
  fileContextMenu.value=null
  if(base!==currentPath.value)await loadFiles(base)
  openNewEntry(kind)
}
function addContextEntryToConversation(){const entry=fileContextMenu.value?.entry;fileContextMenu.value=null;if(entry)void addEntryToConversation(entry)}
function downloadContextFile(){const entry=fileContextMenu.value?.entry;fileContextMenu.value=null;if(entry)void downloadCurrentFile(entry.path,entry.kind)}
function copyContextPath(){const path=fileContextMenu.value?.entry?.path??currentPath.value;fileContextMenu.value=null;void copyFilePath(path)}
function openFileAction(kind:'rename'|'delete',entry:FileEntry){fileContextMenu.value=null;fileActionDialog.value={kind,entry};fileActionValue.value=kind==='rename'?entry.name:''}
async function copyFilePath(path=currentPath.value){try{await writeClipboard(path||'.');flashSafeNotice('相对路径已复制')}catch{flash('复制路径失败')}}
function remapWorkspacePath(path:string,source:string,target:string){return path===source||path.startsWith(source+'/')?target+path.slice(source.length):path}
function captureFileSelection(){
  const element=fileEditor.value
  fileSelection.value=element&&element.selectionEnd>element.selectionStart?{start:element.selectionStart,end:element.selectionEnd}:null
}
function selectedFileExcerpt(){
  const range=fileSelection.value
  if(!range||range.end<=range.start||range.end>fileContent.value.length)return null
  const content=fileContent.value.slice(range.start,range.end)
  const startLine=(fileContent.value.slice(0,range.start).match(/\n/g)?.length??0)+1
  const endPosition=range.end>range.start&&fileContent.value[range.end-1]==='\n'?range.end-1:range.end
  const endLine=(fileContent.value.slice(0,endPosition).match(/\n/g)?.length??0)+1
  return{content,label:`${activeFile.value}（第 ${startLine}${endLine===startLine?'':'–'+endLine} 行选区）`}
}
async function addEntryToConversation(entry?:FileEntry){
  const path=entry?.path??activeFile.value;if(!path||entry?.kind==='folder')return
  const selection=entry?null:selectedFileExcerpt()
  await insertFileIntoComposer(path,null,selection?.content??(path===activeFile.value?fileContent.value:undefined),selection?.label??path)
}
async function submitFileAction(){
  const dialog=fileActionDialog.value;if(!dialog||!selectedId.value)return
  fileOperationEpoch+=1
  fileActionBusy.value=true
  try{
    if(dialog.kind==='rename'){
      const name=fileActionValue.value.trim();if(!name){flashSafeNotice('新名称不能为空');return}
      const parent=dialog.entry.path.split('/').slice(0,-1).join('/'),newPath=[parent,name].filter(Boolean).join('/')
      const renamed=await api('/api/servers/'+selectedId.value+'/file/rename',{method:'POST',body:JSON.stringify({path:dialog.entry.path,new_path:newPath})}) as {new_path?:unknown}
      const target=typeof renamed.new_path==='string'&&renamed.new_path?renamed.new_path:newPath
      activeFile.value=remapWorkspacePath(activeFile.value,dialog.entry.path,target);currentPath.value=remapWorkspacePath(currentPath.value,dialog.entry.path,target);parentPath.value=parentPath.value===null?null:remapWorkspacePath(parentPath.value,dialog.entry.path,target)
      flash('已重命名为 '+target.split('/').at(-1))
    }else{
      await api('/api/servers/'+selectedId.value+'/file',{method:'DELETE',body:JSON.stringify({path:dialog.entry.path,recursive:dialog.entry.kind==='folder'})})
      if(activeFile.value===dialog.entry.path||activeFile.value.startsWith(dialog.entry.path+'/')){activeFile.value='';fileContent.value='';fileReadonly.value=false;fileSelection.value=null}
      flash(`${dialog.entry.kind==='folder'?'目录':'文件'}已删除`)
    }
    composerFilesWorkspace.value='';composerFiles.value=[];fileActionDialog.value=null;await loadFiles(currentPath.value)
  }catch(error){flash((dialog.kind==='rename'?'重命名':'删除')+'失败：'+String(error))}
  finally{fileActionBusy.value=false}
}
async function openProperties(){if(!selectedId.value)return;tab.value='files';await loadFiles();await openFile('server.properties')}
async function selectServer(id:string){
  const selected=servers.value.find(item=>item.id===id)
  if(!selected)return
  const epoch=++workspaceSelectionEpoch;conversationSelectionEpoch+=1
  saveCurrentDraft()
  const project=workspaceKind(selected)==='project'
  if(workspaceMode.value!==workspaceKind(selected)){
    workspaceMode.value=workspaceKind(selected)
    localStorage.setItem(WORKSPACE_MODE_KEY,workspaceMode.value)
  }
  resetConversationTransient();selectedConversationId.value='';messages.value=[];chatModelOverride.value=null;chatAgentOverride.value=null;chatReasoningEffort.value=null;conversationSelectionPending.value=true;conversationCreationPending.value=ensureConversationPromises.has(id)
  selectedId.value=id;surface.value='control';tab.value=project?'files':'overview';activeFile.value='';mirrorPanel.value=false;showServiceSettings.value=false;serviceSettingsDraft.value=null;previewCandidates.value=[];downloadStatus.value=null;stopDownloadPolling()
  if(!conversationsByServer.value[id])await loadConversationSummaries(id,false)
  if(epoch!==workspaceSelectionEpoch||selectedId.value!==id)return
  const items=conversationsByServer.value[id]??[]
  const remembered=selectedConversationByServer.value[id]
  const preferred=items.find(item=>item.id===remembered&&!item.archived)??items.find(item=>!item.archived)??items[0]
  if(preferred)await selectConversation(preferred);else{conversationSelectionPending.value=false;restoreCurrentDraft()}
  if(epoch!==workspaceSelectionEpoch||selectedId.value!==id)return
  if(project)await loadFiles()
  else if(server.value.status!=='planning')await Promise.all([loadLogs(),fetchDownloadStatus()])
}
async function openMirrorPanel(){if(!selectedId.value)return;mirrorPanel.value=!mirrorPanel.value;if(!mirrorPanel.value)return;try{mirrors.value=await api('/api/download/mirrors');selectedMirrorIds.value=mirrors.value.filter(mirror=>mirror.enabled).map(mirror=>mirror.id);previewCandidates.value=[]}catch(error){flash('镜像列表加载失败：'+String(error))}}
const downloadStatus=ref<DownloadStatus|null>(null)
let downloadTimer:number|undefined
const downloadActive=computed(()=>!!downloadStatus.value&&['resolving','downloading','verifying'].includes(downloadStatus.value.phase))
const downloadPhaseLabels:Record<string,string>={resolving:'解析下载源',downloading:'下载中',verifying:'校验中',completed:'下载完成',failed:'下载失败',cancelled:'已取消'}
const downloadPhaseLabel=computed(()=>downloadStatus.value?(downloadPhaseLabels[downloadStatus.value.phase]??downloadStatus.value.phase):'')
const formatMb=(n:number)=>(n/1048576).toFixed(1)+' MB'
const downloadTraffic=computed(()=>{const s=downloadStatus.value;if(!s)return'';const parts=[formatMb(s.received)+(s.total?' / '+formatMb(s.total):'')];if(s.message)parts.push(s.message);return parts.join(' · ')})
const downloadSummary=computed(()=>{if(downloadStatus.value)return downloadPhaseLabel.value+(downloadStatus.value.source?' · '+downloadStatus.value.source:'');return serverCoreReady.value?'核心已就绪，可在停服后手动更新':'可手动选择下载源作为替代尝试'})
function stopDownloadPolling(){if(downloadTimer){window.clearInterval(downloadTimer);downloadTimer=undefined}}
function startDownloadPolling(){if(downloadTimer)return;downloadTimer=window.setInterval(async()=>{await fetchDownloadStatus(false);if(!downloadActive.value){stopDownloadPolling();await loadDashboard(false)}},1000)}
async function fetchDownloadStatus(schedule=true){const id=selectedId.value;if(!id){downloadStatus.value=null;return}try{const data=await api('/api/servers/'+id+'/download/status');if(selectedId.value!==id)return;downloadStatus.value=data.status??null;if(schedule&&downloadActive.value)startDownloadPolling()}catch{}}
async function startCoreDownload(){if(!selectedId.value||downloadActive.value||serverOperationState.value!=='idle')return;busy.value=true;try{const task=await api('/api/servers/'+selectedId.value+'/download/core',{method:'POST',body:JSON.stringify({mirror_ids:selectedMirrorIds.value})});if(task?.id)upsertTask(task);flash('核心下载任务已启动（资源目录优先）');await fetchDownloadStatus()}catch(error){flash('下载启动失败：'+String(error))}finally{busy.value=false}}
async function cancelCoreDownload(){if(!selectedId.value)return;try{await api('/api/servers/'+selectedId.value+'/download/cancel',{method:'POST'});flash('已请求取消下载')}catch(error){flash('取消失败：'+String(error))}}
async function previewDownloads(){if(!selectedId.value||!selectedMirrorIds.value.length)return;busy.value=true;try{const data=await api('/api/download/preview',{method:'POST',body:JSON.stringify({core:server.value.core,version:server.value.version,mirror_ids:selectedMirrorIds.value})});previewCandidates.value=data.candidates;flash('已生成 '+data.candidates.length+' 个下载候选地址')}catch(error){flash('预览失败：'+String(error))}finally{busy.value=false}}
async function retryProvision(){const id=selectedId.value;if(!id||busy.value)return;busy.value=true;try{const data=await api('/api/servers/'+id+'/provision',{method:'POST'});if(data?.server)upsertServer(data.server);const task:TaskInfo|undefined=data?.provision_task??data?.task??(data?.id?data:undefined);if(task)upsertTask(task);flash('已重新开始首次初始化');await loadDashboard(false)}catch(error){flash('初始化重试失败：'+String(error))}finally{busy.value=false}}
async function cancelProvision(){const task=bootstrapTask.value;if(!task||!activeProvisionStatuses.has(task.status)||task.status==='cancelling'||busy.value)return;busy.value=true;try{const updated=await api('/api/tasks/'+task.id+'/cancel',{method:'POST'});if(updated?.id)upsertTask(updated);flash('已请求取消首次初始化');await loadDashboard(false)}catch(error){flash('取消初始化失败：'+String(error))}finally{busy.value=false}}
async function toggleServer(){if(!selectedId.value||serverControlDisabled.value){if(server.value.status!=='online'&&serverStartBlocker.value)flash(serverStartBlocker.value);return}busy.value=true;try{const action=server.value.status==='online'?'stop':'start';const data=await api('/api/servers/'+selectedId.value+'/action',{method:'POST',body:JSON.stringify({action})});upsertServer(data.server);if(data.log)terminal.value.push(data.log);flash(action==='start'?'服务器进程已创建，正在等待就绪':'服务器进程已确认停止')}catch(error){flash('操作失败：'+String(error))}finally{busy.value=false}}
async function restartServer(){if(!selectedId.value||busy.value||serverTransitioning.value||!serverCoreReady.value||!javaReady.value||provisionActive.value)return;busy.value=true;try{const data=await api('/api/servers/'+selectedId.value+'/action',{method:'POST',body:JSON.stringify({action:'restart'})});upsertServer(data.server);if(data.log)terminal.value.push(data.log);flash('旧进程已退出，服务器正在重新启动')}catch(error){flash('重启失败：'+String(error))}finally{busy.value=false}}
async function runCommand(){const value=command.value.trim();if(!selectedId.value||!value||busy.value)return;if(!terminalCommandReady.value){flash(terminalCommandPlaceholder.value);return}command.value='';busy.value=true;try{const data=await api('/api/servers/'+selectedId.value+'/command',{method:'POST',body:JSON.stringify({command:value})});if(!logStreamLive.value){terminal.value.push(...data.lines);scrollTerminal()}}catch(error){terminal.value.push('[ERROR]: '+String(error))}finally{busy.value=false}}
let dashboardLoadPromise:Promise<void>|null=null
function loadDashboard(loadRelated=true):Promise<void>{
  if(dashboardLoadPromise)return dashboardLoadPromise
  dashboardLoadPromise=loadDashboardOnce(loadRelated).finally(()=>{dashboardLoadPromise=null})
  return dashboardLoadPromise
}
async function refreshSelectedConversationTaskResults(){
  const conversationId=selectedConversationId.value
  if(!conversationId||chatBusy.value||conversationSelectionPending.value)return
  const linkedIds=new Set(messages.value.flatMap(message=>message.task_id?[message.task_id]:[]))
  const relevant=[...pendingConversationTaskRefresh].filter(taskId=>linkedIds.has(taskId))
  if(!relevant.length)return
  try{
    const conversation=await apiRequest<Conversation>('/api/conversations/'+conversationId)
    if(selectedConversationId.value!==conversationId||chatBusy.value)return
    messages.value=mergeRunMessages(conversation.id,conversation.messages.map(message=>({...message})))
    relevant.forEach(taskId=>pendingConversationTaskRefresh.delete(taskId))
    await loadConversationSummaries(conversation.server_id,false)
  }catch{
    // 保留待刷新标记，下一次仪表盘轮询继续尝试。
  }
}
async function loadDashboardOnce(loadRelated=true){
  const previousState=dashboardState.value
  const previousTasks=new Map(tasks.value.map(task=>[task.id,task.status]))
  try{
    const data=await api('/api/dashboard')
    const incomingServers:ServerItem[]=Array.isArray(data.servers)?data.servers:[]
    const incomingTelemetry:Record<string,ServerTelemetry>=data.telemetry&&typeof data.telemetry==='object'?data.telemetry as Record<string,ServerTelemetry>:{}
    const filtered=filterDeletedWorkspaceSnapshot(incomingServers,Array.isArray(data.tasks)?data.tasks as TaskInfo[]:[],incomingTelemetry,deletedWorkspaceIds)
    servers.value=filtered.servers;tasks.value=filtered.tasks;dashboardTelemetry.value=filtered.telemetry
    for(const task of filtered.tasks){
      if(TERMINAL_TASK_STATUSES.has(task.status)&&previousTasks.get(task.id)!==task.status)pendingConversationTaskRefresh.add(task.id)
    }
    for(const id of filtered.acknowledgedIds)deletedWorkspaceIds.delete(id)
    dashboardState.value='ready';dashboardError.value=''
    if(!visibleServers.value.some(item=>item.id===selectedId.value)){
      const nextId=visibleServers.value[0]?.id??''
      if(nextId)await selectServer(nextId)
      else{workspaceSelectionEpoch+=1;conversationSelectionEpoch+=1;saveCurrentDraft();selectedId.value='';selectedConversationId.value='';messages.value=[];conversationSelectionPending.value=false;conversationCreationPending.value=false;resetConversationTransient();restoreCurrentDraft()}
    }
    if(loadRelated&&selectedId.value&&!isProject.value&&server.value.status!=='planning')await loadLogs()
    await refreshSelectedConversationTaskResults()
    if(previousState==='error'&&selectedId.value){await loadAllConversationSummaries();await selectServer(selectedId.value)}
  }catch(error){
    if(previousState==='ready'){
      dashboardError.value=String(error)
      notice.value='状态刷新失败，正在保留最后一次有效数据'
      return
    }
    dashboardState.value='error';dashboardError.value=String(error);servers.value=[];tasks.value=[];selectedId.value='';selectedConversationId.value='';messages.value=[];terminal.value=[];disconnectLogStream();stopDownloadPolling()
  }
}
watch(()=>createForm.value.core,()=>syncCreateOptions())
watch(input,()=>{if(!suppressedDraftWrites)saveCurrentDraft();nextTick(resizeComposer)})
watch(messageSearch,()=>{activeSearchMatch.value=0;nextTick(()=>stepMessageSearch(0))})
watch(conversationDrafts,value=>{try{localStorage.setItem(DRAFT_STORAGE_KEY,JSON.stringify(value))}catch{}},{deep:true})
watch(chatQueues,value=>{try{localStorage.setItem(CHAT_QUEUE_STORAGE_KEY,JSON.stringify(value))}catch{}},{deep:true})
watch(selectedId,(id)=>{composerFilesWorkspace.value='';composerFiles.value=[];composerToken.value=null;fileContextMenu.value=null;fileSelection.value=null;if(!id){terminal.value=[];fileEntries.value=[];activeFile.value='';disconnectLogStream();return}if(tab.value==='files'||isProject.value)loadFiles();else if(tab.value==='terminal')connectLogStream();else loadLogs()})
watch(tab,(next,prev)=>{if(next==='terminal'&&selectedId.value&&!isProject.value)connectLogStream();else if(prev==='terminal')disconnectLogStream()})
watch(surface,(_next,prev)=>{if(prev==='settings')loadAiSettings()})
let refreshTimer:number|undefined
onMounted(async()=>{document.addEventListener('click',closeMenus);loadUi().catch(()=>{});await Promise.all([loadDashboard(),loadAiSettings(),loadSystemInfo()]);if(dashboardState.value==='ready'){await loadAllConversationSummaries();if(selectedId.value)await selectServer(selectedId.value);if(selectedId.value&&!isProject.value&&server.value.status!=='planning')await fetchDownloadStatus()}refreshTimer=window.setInterval(async()=>{if(!showCreate.value&&!creating.value&&!(surface.value==='control'&&tab.value==='files')){await loadDashboard(false);if(selectedId.value&&!isProject.value&&tab.value==='terminal'&&!logStreamLive.value&&server.value.status!=='planning')connectLogStream()}},2000)})
onUnmounted(()=>{document.removeEventListener('click',closeMenus);if(refreshTimer)window.clearInterval(refreshTimer);for(const controller of chatControllers.values())controller.abort();chatControllers.clear();abortSpeechCapture();disconnectLogStream();stopDownloadPolling()})
</script>

<template>
  <main class="app" :class="{collapsed,'mirror-mode':surface==='mirror'||surface==='settings'}">
    <aside class="sidebar">
      <div class="brand"><span class="logo"><Box :size="17"/></span><div v-if="!collapsed"><b>Sculk Catalyst</b><small>AI Workspace Studio</small></div><button @click="collapsed=!collapsed"><PanelLeftClose v-if="!collapsed" :size="16"/><ChevronRight v-else :size="16"/></button></div>
      <div v-if="!collapsed" class="workspace-mode-switch" role="tablist" aria-label="工作区模式"><button role="tab" :aria-selected="workspaceMode==='project'" :class="{active:workspaceMode==='project'}" @click="switchWorkspaceMode('project')"><FolderTree/>项目模式</button><button role="tab" :aria-selected="workspaceMode==='server'" :class="{active:workspaceMode==='server'}" @click="switchWorkspaceMode('server')"><Server/>服务器模式</button></div>
      <div class="workspace-actions">
        <button class="create" @click="openCreate"><Plus :size="16"/><span v-if="!collapsed">{{workspaceMode==='project'?'创建项目':'创建服务器'}}</span></button>
        <button class="open-existing" title="打开已有目录" @click="openExistingDirectory"><FolderOpen :size="16"/><span v-if="!collapsed">打开已有目录</span></button>
      </div>
      <nav><button aria-label="控制中心" :class="{active:surface==='control'}" @click="surface='control'"><LayoutDashboard/><span v-if="!collapsed">控制中心</span></button><button v-if="workspaceMode==='server'" aria-label="资源中心" title="资源中心" :class="{active:surface==='mirror'}" @click="surface='mirror'"><Archive/><span v-if="!collapsed">资源中心</span></button><button aria-label="任务执行器" :class="{active:surface==='automation'}" @click="openTaskCenter()"><Sparkles/><span v-if="!collapsed">任务执行器</span><i v-if="!collapsed&&tasks.some(task=>task.status==='awaiting_approval')">{{tasks.filter(task=>task.status==='awaiting_approval').length}}</i></button><button v-if="workspaceMode==='server'" aria-label="玩家社区" :class="{active:surface==='community'}" @click="surface='community'"><Vote/><span v-if="!collapsed">玩家社区</span></button><button aria-label="Skills & MCP" :class="{active:surface==='integrations'}" @click="surface='integrations'"><PlugZap/><span v-if="!collapsed">Skills & MCP</span></button></nav>
      <div v-if="!collapsed" class="label">{{workspaceMode==='project'?'项目':'服务器'}} <MoreHorizontal :size="15"/></div>
      <ConversationTree
        :servers="visibleServers"
        :mode="workspaceMode"
        :conversations="conversationsByServer"
        :agents="aiSettings?.agents ?? []"
        :selected-server-id="selectedId"
        :selected-conversation-id="selectedConversationId"
        :running-conversation-ids="activeConversationIds"
        :collapsed="collapsed"
        @select-server="selectServer"
        @select-conversation="selectConversation"
        @new-conversation="createConversation"
        @conversation-action="handleConversationAction"
        @delete-server="openDeleteServer"
      />
      <div class="spacer"/><div v-if="!collapsed" class="codex"><GitBranch :size="17"/><span><b>{{dashboardState==='loading'?'后端连接中':dashboardState==='ready'?'后端已连接':'后端未连接'}}</b><small>{{dashboardState==='ready'?'工作区数据来自本机服务':'不会显示演示运行数据'}}</small></span><i class="dot" :class="{online:dashboardState==='ready',warning:dashboardState==='loading'}"/></div><button class="settings" :class="{active:surface==='settings'}" @click="surface='settings'"><Settings/><span v-if="!collapsed">设置</span></button>
    </aside>

    <section v-if="surface!=='mirror'&&surface!=='settings'" class="chat-panel">
      <header><div><small>{{selectedId?server.name:dashboardState==='loading'?'正在连接后端':dashboardState==='error'?'后端未连接':'首次使用'}}</small><h1>{{selectedId?(selectedConversation?.title || '新对话任务'):dashboardState==='ready'?(workspaceMode==='project'?'尚无项目':'尚无服务器'):'等待运行数据'}}</h1><em v-if="selectedId">{{server.task}}</em></div><span><button :class="{active:showMessageSearch}" :disabled="!selectedConversation" title="搜索当前对话" @click="toggleMessageSearch"><Search/></button><button :disabled="!selectedConversation" title="重命名当前对话" @click="selectedConversation&&handleConversationAction('rename',selectedConversation)"><MoreHorizontal/></button></span></header>
      <div v-if="showMessageSearch" class="message-search"><Search/><input ref="messageSearchInput" v-model="messageSearch" placeholder="搜索当前对话" @keydown.enter.prevent="stepMessageSearch(1)"/><span>{{messageSearch.trim()?messageSearchMatches.length?`${activeSearchMatch+1} / ${messageSearchMatches.length}`:'无结果':''}}</span><button :disabled="!messageSearchMatches.length" title="上一个结果" @click="stepMessageSearch(-1)"><ChevronUp/></button><button :disabled="!messageSearchMatches.length" title="下一个结果" @click="stepMessageSearch(1)"><ChevronDown/></button><button title="关闭搜索" @click="toggleMessageSearch"><X/></button></div>
      <div ref="scroller" class="chat-scroll">
        <section v-if="dashboardState==='loading'" class="connection-state"><LoaderCircle class="spin"/><b>正在加载服务器数据</b><small>等待本机后端响应，不会使用演示服务器填充。</small></section>
        <section v-else-if="dashboardState==='error'" class="connection-state error"><PlugZap/><b>后端未连接</b><small>{{dashboardError}}。启动或恢复后端后可在此重试。</small><button @click="retryBackend">重新连接</button></section>
        <section v-else-if="!selectedId" class="connection-state first-run"><FolderTree v-if="workspaceMode==='project'"/><Server v-else/><b>{{workspaceMode==='project'?'还没有通用项目':'还没有服务器项目'}}</b><small>{{workspaceMode==='project'?'创建项目只会建立一个空文件夹，不会进入 Minecraft 核心或插件引导。':'这是首次使用时的正常状态。可以直接打开创建向导进行环境检查。'}}</small><div class="first-run-actions"><button @click="openCreate"><Plus/>{{workspaceMode==='project'?'创建第一个项目':'创建第一台服务器'}}</button><button class="secondary" @click="openExistingDirectory"><FolderOpen/>打开已有目录</button></div></section>
        <section v-else-if="server.status==='planning'" class="mission planning-mission"><div><span class="agent"><BrainCircuit :size="20"/></span><p><small>智能创建 · 规划阶段</small><b>通过对话确定核心、版本与部署方案</b></p><em><i/>仅有 sculk.yml</em></div><footer><span>可迁移标识已建立</span><span>方案确认后再下载服务端文件</span></footer></section>
        <section v-else-if="!selectedConversationId" class="mission empty-mission"><div><span class="agent"><MessageSquareText :size="20"/></span><p><small>{{isProject?'项目对话任务':'服务器对话任务'}}</small><b>新建一个独立对话开始工作</b></p></div><footer><span>每个任务拥有独立历史与上下文</span><button @click="createConversation(selectedId)"><Plus/>新建对话</button></footer></section>
        <div v-if="selectedId&&messages.length" class="day">今天</div>
        <article v-for="message in selectedId?messages:[]" :key="message.id" class="message" :class="[message.role,{error:message.error,interrupted:message.interrupted,'search-match':isSearchMatch(message.id),'active-match':isActiveSearchMatch(message.id)}]" :data-message-id="message.id">
          <span v-if="message.role==='assistant'" class="avatar bot"><Bot :size="17"/></span><div><header><b>{{message.role==='assistant'?'Sculk Agent':'你'}}</b><time>{{message.streaming?'正在响应':message.time}}</time><em v-if="message.role==='assistant'&&message.fallback" class="fallback-tag">本地规则</em><em v-if="message.interrupted" class="message-state">已停止</em><em v-else-if="message.error" class="message-state error">未完成</em></header><div class="message-body"><ConversationMessageContent :content="message.content"/><i v-if="message.streaming" class="stream-cursor"/></div><p v-if="message.warning" class="message-warning"><AlertTriangle/>{{message.warning}}</p><section v-if="taskForMessage(message)" class="inline-task"><header><span><Activity/><b>{{taskForMessage(message)?.title}}</b></span><em :class="taskForMessage(message)?.risk">{{taskRiskLabel(taskForMessage(message)?.risk||'low')}}</em></header><div><i :style="{width:(taskForMessage(message)?.progress||0)+'%'}"/></div><footer><span>{{taskStatusLabel[taskForMessage(message)?.status||'']||taskForMessage(message)?.status}}</span><button @click="openTaskCenter(message.task_id)">打开任务中心<ChevronRight/></button></footer></section><footer v-if="message.actions?.length&&!taskForMessage(message)" class="response-actions"><button v-for="action in message.actions" :key="action" @click="handleMessageAction(action,message)">{{action}}<ChevronRight :size="13"/></button></footer><nav class="message-tools"><button title="复制消息" @click="copyMessage(message.content)"><Copy/></button><button v-if="message.retryContent&&(message.error||message.interrupted)" title="重新生成" @click="retryMessage(message)"><RotateCcw/>重试</button></nav></div><span v-if="message.role==='user'" class="avatar user">A</span>
        </article>
        <article v-if="thinking" class="message assistant"><span class="avatar bot"><Bot :size="17"/></span><div><header><b>Sculk Agent</b><time>{{chatStatusLabel}}</time></header><span class="typing"><i/><i/><i/></span></div></article>
      </div>
      <div v-if="selectedId" class="compose-wrap">
        <div class="composer-toolbar">
          <span class="composer-menu-anchor composer-agent-picker">
            <button class="agent-picker" :class="{active:showAgentMenu}" title="选择当前对话使用的 Agent" @click.stop="showAgentMenu=!showAgentMenu;showModelMenu=false;showReviewMenu=false"><Bot/>{{chatAgentLabel}}<ChevronDown/></button>
            <div v-if="showAgentMenu" class="composer-menu composer-agent-menu"><small>当前对话的 Agent</small><button :class="{picked:!activeAgentId}" @click="pickChatAgent(null)"><span><b>Sculk Agent（内置）</b><em>直连模型提供商，模型和思考强度在输入框下方选择</em></span><Check v-if="!activeAgentId"/></button><template v-if="agentMenuItems.length"><small class="group">已接入的智能体</small><div v-for="agent in agentMenuItems" :key="agent.id" class="agent-option" :class="{picked:activeAgentId===agent.id}"><button :disabled="!!fullAccessAgentPolicyMessage(agent)" :title="fullAccessAgentPolicyMessage(agent)||'选择该 Agent'" @click="pickChatAgent(agent.id)"><span><b>{{agent.name}}</b><em>{{agentMenuDetail(agent)}}</em></span><Check v-if="activeAgentId===agent.id"/></button><label v-if="(agent.transport??'acp')==='cli'" :title="fullAccessAgentPolicyMessage(agent)||'拖动后会选择该 Agent 并保存思考强度'" @click.stop><BrainCircuit/><input type="range" min="0" :max="reasoningItemsForAgent(agent).length" step="1" :disabled="!!fullAccessAgentPolicyMessage(agent)" :value="agentReasoningIndex(agent)" @change="changeChatAgentReasoning(agent,$event)"/><em>{{agentReasoningLabel(agent)}}</em></label></div></template><small v-else class="empty-hint">尚未接入外部智能体，可到「设置」自动检测 CLI 或手动接入</small></div>
          </span>
          <div v-if="cloudPrompts.length" class="quick-prompts" :class="{open:showQuickPrompts}"><button class="quick-prompts-toggle" :aria-expanded="showQuickPrompts" title="展开快捷指令" @click="showQuickPrompts=!showQuickPrompts"><MessageSquareText/><span>快捷指令</span><small>{{cloudPrompts.length}}</small><ChevronDown/></button><div v-if="showQuickPrompts" class="prompts"><button v-for="prompt in cloudPrompts" :key="prompt.id" @click="showQuickPrompts=false;send(prompt.content)"><Sparkles/>{{ prompt.title }}</button></div></div>
        </div>
        <section v-if="currentChatQueue.length" class="chat-queue">
          <header><span><ListPlus/>等待发送</span><div><small>{{ currentChatQueue.length }} 条 · {{currentQueuePaused?'队列已暂停，可确认后继续':'当前回复正常结束后依次发送'}}</small><button v-if="currentQueuePaused" @click="runNextQueuedChat(selectedConversationId)"><Play/>继续队列</button></div></header>
          <article v-for="(queued,index) in currentChatQueue" :key="queued.id" :class="queued.mode">
            <span>{{ index+1 }}</span>
            <textarea v-if="editingQueuedId===queued.id" :data-queue-editor="queued.id" v-model="editingQueuedContent" maxlength="64000" @keydown.ctrl.enter.prevent="saveQueuedEdit(queued)"/>
            <p v-else><b>{{ queued.mode==='steer'?'引导发送':'排队消息' }}</b><small>{{ queued.content }}</small></p>
            <nav v-if="editingQueuedId===queued.id"><button title="保存修改" @click="saveQueuedEdit(queued)"><Check/></button><button title="取消修改" @click="editingQueuedId='';editingQueuedContent='' "><X/></button></nav>
            <nav v-else><button title="修改未发送内容" @click="beginQueuedEdit(queued)"><Pencil/></button><button class="queue-steer" :title="chatBusy?'停止当前回复并立即发送为引导':'立即发送'" @click="sendQueuedNow(queued)"><CornerDownRight/><span>{{chatBusy?'引导':'发送'}}</span></button><button title="移出队列" @click="removeQueuedChat(queued.conversationId,queued.id)"><Trash2/></button></nav>
          </article>
        </section>
        <div class="composer" :class="{running:chatBusy}">
          <div v-if="composerToken" class="composer-assist" @mousedown.stop.prevent>
            <header><span>{{composerToken.kind==='skill'?'选择技能':composerToken.kind==='file'?'选择工作区文件':'选择智能体'}}</span><small>{{composerToken.trigger}}{{composerToken.query}}</small></header>
            <button v-for="(suggestion,index) in composerSuggestions" :key="suggestion.id" :class="{active:index===composerSuggestionIndex}" @mousedown="selectComposerSuggestion(suggestion)"><Sparkles v-if="suggestion.kind==='skill'"/><FileCode2 v-else-if="suggestion.kind==='file'"/><Bot v-else/><span><b>{{suggestion.label}}</b><small>{{suggestion.detail}}</small></span><em>Enter</em></button>
            <p v-if="composerSourceLoading===composerToken.kind"><LoaderCircle class="spin"/>正在读取候选项…</p>
            <p v-else-if="!composerSuggestions.length">没有匹配项</p>
          </div>
          <textarea ref="composerInput" v-model="input" maxlength="64000" placeholder="描述任务；/ 或 % 技能，# 文件，@ 智能体，!{模型}：内容! 快捷发送…" @input="nextTick(updateComposerAssist)" @click="updateComposerAssist" @keyup="handleComposerKeyup" @keydown="handleComposerKeydown"/><footer><span>
        <button class="speech-input" :class="{recording:speechState==='recording',transcribing:speechState==='transcribing'}" :aria-pressed="speechState==='recording'" :disabled="speechState==='transcribing'" :title="speechButtonTitle" @click="toggleSpeechCapture"><LoaderCircle v-if="speechState==='transcribing'" class="spin"/><CircleStop v-else-if="speechState==='recording'"/><Mic v-else/></button>
        <span class="composer-menu-anchor">
          <button :class="{active:showContextMenu}" title="查看发送上下文" @click.stop="showContextMenu=!showContextMenu;showAgentMenu=false;showModelMenu=false;showReviewMenu=false"><Paperclip/></button>
          <div v-if="showContextMenu" class="composer-menu context-menu">
            <small>本次消息携带的上下文</small>
            <div><Server/><span><b>{{server.name}}</b><em>{{server.core || '待规划'}} {{server.version}} · {{server.status==='online'?'运行中':'未运行'}}</em></span><Check/></div>
            <div><MessageSquareText/><span><b>当前对话</b><em>最近 20 条消息</em></span><Check/></div>
            <div v-if="reasoningSupported"><BrainCircuit/><span><b>{{reasoningLabel}}</b><em>该设置会传入本次模型或原生 CLI 调用</em></span><Check/></div>
            <div><ShieldCheck/><span><b>{{reviewModeLabel}}</b><em>{{safeHint}}</em></span><Check/></div>
          </div>
        </span>
        <span v-if="!activeAgentId" class="composer-menu-anchor">
          <button class="model" :class="{active:showModelMenu}" @click.stop="showModelMenu=!showModelMenu;showAgentMenu=false;showReviewMenu=false"><Cpu/>{{chatModelLabel}} · {{reasoningShortLabel}}<ChevronDown/></button>
          <div v-if="showModelMenu" class="composer-menu model-config-menu">
            <small>模型与思考强度</small>
            <div class="model-option" :class="{picked:!chatModelOverride}"><button @click="pickChatModel(null)"><span><b>自动模型</b><em>按情景绑定选择模型</em></span><Check v-if="!chatModelOverride"/></button><label title="拖动后保存自动模型的对话思考强度" @click.stop><BrainCircuit/><input type="range" min="0" :max="reasoningMenuItems.length" step="1" :value="modelReasoningIndex(null)" @change="changeChatModelReasoning(null,$event)"/><em>{{modelReasoningLabel(null)}}</em></label></div>
            <template v-for="group in modelMenuGroups" :key="group.id">
              <small class="group">{{group.name}}</small>
              <div v-for="model in group.models" :key="group.id+'::'+model.id" class="model-option" :class="{picked:chatModelOverride?.provider_id===group.id&&chatModelOverride?.model_id===model.id}"><button @click="pickChatModel({provider_id:group.id,model_id:model.id})"><span><b>{{model.id}}</b></span><Check v-if="chatModelOverride?.provider_id===group.id&&chatModelOverride?.model_id===model.id"/></button><label title="拖动后选择该模型并保存思考强度" @click.stop><BrainCircuit/><input type="range" min="0" :max="reasoningMenuItems.length" step="1" :value="modelReasoningIndex({provider_id:group.id,model_id:model.id})" @change="changeChatModelReasoning({provider_id:group.id,model_id:model.id},$event)"/><em>{{modelReasoningLabel({provider_id:group.id,model_id:model.id})}}</em></label></div>
            </template>
            <small v-if="!modelMenuGroups.length" class="empty-hint">尚无可用模型，请到「设置」添加提供商并启用模型</small>
          </div>
        </span>
        <span class="composer-menu-anchor">
          <button class="model review" :class="{active:showReviewMenu,warn:reviewMode==='full'}" @click.stop="showReviewMenu=!showReviewMenu;showAgentMenu=false;showModelMenu=false"><ShieldCheck/>{{reviewModeLabel}}<ChevronDown/></button>
          <div v-if="showReviewMenu" class="composer-menu">
            <small>审核模式</small>
            <button v-for="mode in REVIEW_MODES" :key="mode.key" :class="{picked:reviewMode===mode.key}" @click="pickReviewMode(mode.key)"><span><b>{{mode.label}}</b><em>{{mode.hint}}</em></span><Check v-if="reviewMode===mode.key"/></button>
          </div>
        </span>
      </span><button v-if="chatBusy" class="send stop-generation" title="停止生成" @click="stopGeneration"><CircleStop/></button><button class="send" :disabled="conversationSelectionPending||conversationCreationPending||!input.trim()||!!activeAgentPolicyMessage" :title="activeAgentPolicyMessage|| (conversationCreationPending?'正在创建对话':conversationSelectionPending?'正在切换对话':chatBusy?'加入发送队列':'发送消息')" @click="send()"><ListPlus v-if="chatBusy"/><Send v-else/></button></footer></div><div class="composer-status"><span v-if="speechState==='recording'"><i/>正在录音 · 点击麦克风按钮结束</span><span v-else-if="speechState==='transcribing'"><LoaderCircle class="spin"/>正在调用 ASR 模型转写</span><span v-else-if="chatBusy"><LoaderCircle v-if="currentChatRun?.phase==='stopping'" class="spin"/><i v-else/>{{chatStatusLabel}} · Enter 加入队列</span><span v-else-if="conversationCreationPending"><LoaderCircle class="spin"/>正在创建对话</span><span v-else-if="conversationSelectionPending"><LoaderCircle class="spin"/>正在载入对话</span><span v-else-if="activeAgentPolicyMessage"><ShieldCheck/>{{activeAgentPolicyMessage}}</span><span v-else>Enter 发送 · Shift + Enter 换行</span><p v-if="safeNotice" class="safe" :class="{warn:reviewMode==='full'}"><ShieldCheck/>{{safeNotice}}</p></div></div>
      <WorkspacePanelResizer/>
    </section>

    <section class="work-panel" :class="{'mirror-work':surface==='mirror'||surface==='settings'}">
      <template v-if="surface==='control'">
      <header class="work-header"><nav><button :disabled="!selectedId" :class="{active:tab==='overview'}" @click="tab='overview'"><Gauge/>总览</button><button :disabled="!selectedId||(!isProject&&server.status==='planning')" :class="{active:tab==='files'}" @click="tab='files';loadFiles()"><Files/>文件</button><button v-if="isProject" :disabled="!selectedId" :class="{active:tab==='build'}" @click="tab='build'"><Wrench/>构建</button><button :disabled="!selectedId||isProject||server.status==='planning'" :title="isProject?'项目 Shell 需要通过已认证 Agent 接入，当前不会暴露未鉴权本机命令接口':''" :class="{active:tab==='terminal'}" @click="tab='terminal'"><SquareTerminal/>{{isProject?'Shell':'终端'}}</button></nav><span v-if="notice" class="notice">{{notice}}</span><button :disabled="!selectedId"><MoreHorizontal/></button></header>
      <div v-if="dashboardState==='loading'" class="work-scroll workspace-state"><LoaderCircle class="spin"/><small>LOADING</small><h2>正在加载运行数据</h2><p>正在等待本机后端返回服务器列表。</p></div>
      <div v-else-if="dashboardState==='error'" class="work-scroll workspace-state error"><PlugZap/><small>BACKEND OFFLINE</small><h2>后端未连接</h2><p>当前没有可验证的服务器运行状态。{{dashboardError}}</p><button @click="retryBackend">重新连接</button></div>
      <div v-else-if="!selectedId" class="work-scroll workspace-state first-run"><FolderTree v-if="workspaceMode==='project'"/><Server v-else/><small>FIRST RUN</small><h2>{{workspaceMode==='project'?'创建第一个通用项目':'开始创建第一台服务器'}}</h2><p>{{workspaceMode==='project'?'项目模式只创建一个空文件夹，随后可直接使用文件编辑和独立对话，不会启动 Minecraft 引导。':'服务器列表为空。创建向导会先检查 Java、数据目录、磁盘与内存，再允许创建普通服务器。'}}</p><div class="first-run-actions"><button @click="openCreate"><Plus/>{{workspaceMode==='project'?'创建项目':'打开创建向导'}}</button><button class="secondary" @click="openExistingDirectory"><FolderOpen/>打开已有目录</button></div></div>
      <div v-else-if="tab==='overview'" class="work-scroll">
        <section v-if="openDirectorySummary" class="directory-import-summary">
          <header><p><small>DIRECTORY IMPORT</small><b>已接管已有{{workspaceKind(openDirectorySummary.server)==='project'?'项目':'服务器目录'}}</b></p><button title="关闭检测摘要" @click="openDirectorySummary=null"><X/></button></header>
          <div class="directory-import-path"><FolderOpen/><p><b>{{openDirectorySummary.server.name}}</b><small>{{openDirectorySummary.directory||openDirectorySummary.server.location||'本机工作区'}}</small></p></div>
          <dl v-if="openDirectoryDetectedRows.length"><div v-for="row in openDirectoryDetectedRows" :key="row.key"><dt>{{row.label}}</dt><dd>{{row.value}}</dd></div></dl>
          <div v-if="openDirectorySummary.warnings?.length" class="directory-import-warnings"><AlertTriangle/><p><b>检测提醒</b><small v-for="warning in openDirectorySummary.warnings" :key="warning">{{warning}}</small></p></div>
          <footer><span v-if="openDirectorySummary.files?.length">已读取 {{openDirectorySummary.files.length}} 项配置或目录信息</span><button @click="tab='files';loadFiles()"><Files/>查看文件</button></footer>
        </section>
        <section v-if="isProject" class="project-workspace"><span><FolderTree/></span><small>GENERAL PROJECT</small><h2>{{server.name}}</h2><p>这是一个通用项目目录，不包含 Minecraft 核心、插件或开服引导。你可以直接编辑文件，并通过左侧独立对话让 Codex、Claude 或其他 ACP Agent 参与开发。</p><div><button @click="tab='files';loadFiles()"><Files/>打开文件</button><button @click="createConversation(selectedId)"><MessageSquareText/>新建对话</button></div></section>
        <section v-else-if="server.status==='planning'" class="planning-workspace"><span><BrainCircuit/></span><small>PLANNING WORKSPACE</small><h2>服务器尚在规划阶段</h2><p>当前只创建了用于迁移与接手的 sculk.yml 标识，还没有下载核心或生成服务端配置。确认方案后会创建受审计的开服任务；{{reviewMode==='full'?'当前为完全访问权限，任务会立即下载、写入和启动。':'在当前审核模式下，任务获批后才会下载、写入和启动。'}}</p><div class="planning-actions"><button @click="send('请根据我的需求推荐合适的服务端核心，并说明取舍')"><Sparkles/>继续规划</button><button class="next" :disabled="chatBusy||conversationSelectionPending||conversationCreationPending" @click="send('开始创建服务器')"><Play/>按当前方案创建</button><button class="discard" @click="openDeleteServer(server)"><Trash2/>删除此规划</button></div></section>
        <section v-else class="server-hero"><div><span class="big-icon"><Server/></span><p><b>{{server.name}} <em :class="[server.status,serverOperationState]">{{serverStatusLabel}}</em></b><small>{{server.core}} {{server.version}} · {{systemState==='ready'?(systemInfo?.java_version||'未安装 Java'):'Java 状态未知'}} · 内存 {{serverMemoryLimit}} GB · 端口 {{server.port}}</small></p></div><aside><button :class="{active:mirrorPanel}" :disabled="serverTransitioning" @click="openMirrorPanel"><Download/>核心</button><button :disabled="serverOperationState==='provisioning'" @click="openProperties"><Settings/>配置</button><button @click="openServiceSettings"><SlidersHorizontal/>服务设置</button><button v-if="server.status==='online'" :disabled="busy||serverTransitioning||!serverCoreReady||!javaReady||provisionActive" @click="restartServer"><RotateCw/>重启</button><button :disabled="serverControlDisabled" :class="server.status==='online'?'stop':'start'" @click="toggleServer"><LoaderCircle v-if="serverTransitioning" class="spin"/><CircleStop v-else-if="server.status==='online'"/><Play v-else/>{{serverOperationLabel|| (server.status==='online'?'停止服务器':'启动服务器')}}</button></aside></section>
        <section v-if="!isProject" class="card lifecycle-card">
          <header><p><small>服务器生命周期</small><b>当前处于{{lifecyclePhase.label}}阶段</b></p><nav><button v-if="lifecyclePhase.key==='build'" :disabled="lifecycleSaving" @click="setLifecyclePhase('operate')"><Play/>进入运营</button><button v-else-if="lifecyclePhase.key==='operate'" :disabled="lifecycleSaving" @click="setLifecyclePhase('build')"><RotateCcw/>返回建设</button><button @click="openServiceSettings"><SlidersHorizontal/>服务设置</button></nav></header>
          <div class="lifecycle-track"><span v-for="stage in lifecycleStages" :key="stage.key" :class="lifecycleStageClass(stage.key)"><i><Check v-if="lifecycleStageClass(stage.key).done"/><em v-else/></i><b>{{stage.label}}</b></span></div>
          <p class="lifecycle-detail">{{lifecyclePhase.detail}}</p>
          <small>创建完成核心、Java、配置与首次启动；建设负责插件和玩法配置；运营模块可同时开启并持续迭代。</small>
        </section>
        <section v-if="!isProject&&showServiceSettings&&serviceSettingsDraft" class="card service-settings-card">
          <header><p><small>服务设置</small><b>按需开启运营能力</b></p><button title="关闭" @click="closeServiceSettings"><X/></button></header>
          <div class="service-settings-grid">
            <label><input v-model="serviceSettingsDraft.social.enabled" type="checkbox"/><span><b>社交运营</b><small>QQ、Bilibili、抖音机器人接入与动态轮询</small></span></label>
            <label><input v-model="serviceSettingsDraft.economy" type="checkbox"/><span><b>经济运营</b><small>读取经济插件后生成受审计的调控任务</small></span></label>
            <label><input v-model="serviceSettingsDraft.player_support" type="checkbox"/><span><b>用户运维</b><small>玩家画像、问题识别与帮助任务</small></span></label>
            <label><input v-model="serviceSettingsDraft.game_operations" type="checkbox"/><span><b>游戏运营</b><small>AI 生物、亲密度、任务等插件能力编排</small></span></label>
            <label><input v-model="serviceSettingsDraft.content_improvement" type="checkbox"/><span><b>内容改进运营</b><small>意见、计划、投票、镜像测试和反馈闭环</small></span></label>
          </div>
          <div v-if="serviceSettingsDraft.social.enabled" class="social-service-settings">
            <div class="service-channel-row"><b>机器人渠道</b><label><input v-model="serviceSettingsDraft.social.qq_bot" type="checkbox"/>QQ</label><label><input v-model="serviceSettingsDraft.social.bilibili_bot" type="checkbox"/>Bilibili</label><label><input v-model="serviceSettingsDraft.social.douyin_bot" type="checkbox"/>抖音</label></div>
            <div class="service-frequency-grid"><label><span>常规同步</span><input v-model.number="serviceSettingsDraft.social.sync_interval_seconds" type="number" min="10" max="86400"/><em>秒</em></label><label><span>发现评论后</span><input v-model.number="serviceSettingsDraft.social.burst_interval_seconds" type="number" min="1" max="3600"/><em>秒</em></label><label><span>高频持续</span><input v-model.number="serviceSettingsDraft.social.burst_recovery_seconds" type="number" min="1" max="86400"/><em>秒</em></label></div>
            <p>推荐常规每 240 秒同步；发现新评论后切换为每 10 秒同步，并在 240 秒后恢复。机器人凭据接入后才会执行外部发送。</p>
          </div>
          <footer><button @click="closeServiceSettings">取消</button><button class="primary" :disabled="serviceSettingsSaving" @click="saveServiceSettings"><LoaderCircle v-if="serviceSettingsSaving" class="spin"/><Check v-else/>保存设置</button></footer>
        </section>
        <section v-if="!isProject&&server.status!=='planning'&&(server.last_error||(server.status!=='online'&&serverStartBlocker))" class="server-blocker" :class="{error:!!server.last_error}"><AlertTriangle/><p><b>{{provisionFailed?'首次初始化未完成':server.last_error?'上次运行未正常完成':'当前还不能启动'}}</b><small>{{provisionFailed&&serverCoreReady?`${serverStartBlocker}${server.last_error?'：'+server.last_error:''}`:server.last_error||serverStartBlocker}}</small></p><button v-if="server.last_error&&!provisionFailed" @click="tab='terminal'"><SquareTerminal/>查看终端</button></section>
        <section v-if="!isProject&&server.status!=='planning'" class="metrics"><div><span class="cyan"><Users/></span><p><small>在线玩家</small><b>{{playerTelemetryValue}}</b><em>{{playerTelemetryDetail}}</em></p></div><div><span class="purple"><Cpu/></span><p><small>CPU 使用率</small><b>{{server.cpu}}%</b><em>受管进程采样值</em></p></div><div><span class="green"><Database/></span><p><small>进程内存</small><b>{{formatRuntimeMemory(server.memory)}}</b><em>RSS · 上限 {{serverMemoryLimit}} GB</em></p></div><div><span class="orange"><Gauge/></span><p><small>TPS / MSPT</small><b>{{tpsTelemetryValue}}</b><em>{{tpsTelemetryDetail}}</em></p></div><div><span class="orange"><Activity/></span><p><small>监听端口</small><b>{{server.port}}</b><em>{{server.status==='online'?'服务器运行中':'服务器未运行'}}</em></p></div></section>
        <section v-if="!isProject&&server.status!=='planning'&&bootstrapTask" class="card bootstrap-card provision-card" :class="bootstrapTask.status">
          <header><p><small>首次初始化</small><b>{{server.core}} {{server.version}} · 下载、校验并安装核心</b></p><span>{{bootstrapTask.progress}}%</span></header>
          <div class="bootstrap-progress"><i :style="{width:bootstrapTask.progress+'%'}"/></div>
          <div v-if="bootstrapTask.events?.length" class="provision-events"><p v-for="(event,index) in bootstrapTask.events.slice(-3)" :key="`${event.at}-${index}`" :class="event.level"><i/>{{event.message}}</p></div>
          <p v-if="bootstrapTask.error||(provisionFailed&&server.last_error)" class="provision-error"><AlertTriangle/>{{bootstrapTask.error||server.last_error}}</p>
          <p v-if="provisionFailed&&serverCoreReady" class="provision-incomplete"><AlertTriangle/>核心已经安装，但初始化检查尚未完成。请重试初始化后再启动服务器。</p>
          <footer><p>{{taskStatusLabel[bootstrapTask.status]??bootstrapTask.status}}{{serverCoreReady?' · 核心已就绪':''}}</p><span><button v-if="provisionActive&&bootstrapTask.status!=='cancelling'" :disabled="busy" @click="cancelProvision"><CircleStop/>取消初始化</button><button v-if="provisionFailed" :disabled="busy" @click="retryProvision"><RotateCcw/>重试初始化</button><button @click="openTaskCenter(bootstrapTask.id)">任务详情<ChevronRight/></button></span></footer>
        </section>
        <section v-if="!isProject&&server.status!=='planning'&&(mirrorPanel||downloadStatus)" class="card bootstrap-card" :class="downloadStatus?(downloadActive?'running':downloadStatus.phase):''">
          <header><p><small>{{serverCoreReady?'核心维护':'替代下载'}}</small><b>{{server.core}} {{server.version}} · server.jar</b></p><span>{{downloadStatus?downloadStatus.percent+'%':''}}</span></header>
          <div v-if="downloadStatus" class="bootstrap-progress"><i :style="{width:downloadStatus.percent+'%'}"/></div>
          <footer><p>{{downloadSummary}}</p><button :class="{active:mirrorPanel}" @click="openMirrorPanel"><Download/>{{mirrorPanel?'收起下载源':'选择下载源'}}</button></footer>
          <div v-if="mirrorPanel" class="mirror-panel">
            <div class="mirror-list"><label v-for="mirror in mirrors" :key="mirror.id" :class="{disabled:!mirror.enabled}"><input v-model="selectedMirrorIds" type="checkbox" :value="mirror.id" :disabled="!mirror.enabled"/><span><b>{{mirror.name}}</b><small>{{mirror.region}} · 优先级 {{mirror.priority}} · {{mirror.cores.join(' / ')}}</small></span></label></div>
            <button class="preview-button" :disabled="busy||!selectedMirrorIds.length" @click="previewDownloads"><LoaderCircle v-if="busy" class="spin"/><Search v-else/>预览下载接口</button>
            <div v-if="previewCandidates.length" class="candidate-list"><article v-for="candidate in previewCandidates" :key="candidate.mirror_id" :class="{unsupported:!candidate.supported}"><span>{{candidate.priority}}</span><p><b>{{candidate.mirror_name}}</b><code>{{candidate.url}}</code></p><em>{{candidate.supported?'可用':'不支持当前核心'}}</em></article></div>
            <button class="preview-button" :disabled="downloadActive||serverTransitioning||server.status==='online'" @click="startCoreDownload"><LoaderCircle v-if="downloadActive" class="spin"/><Download v-else/>{{downloadActive?'下载中…':'下载并校验 server.jar'}}</button>
          </div>
          <div v-if="downloadStatus" class="candidate-list download-status">
            <article><span>{{downloadStatus.percent}}</span><p><b>{{downloadPhaseLabel}}<template v-if="downloadStatus.source"> · {{downloadStatus.source}}</template></b><code>{{downloadTraffic||'等待下载源响应…'}}</code></p><em v-if="downloadActive"><button class="cancel-btn" @click="cancelCoreDownload">取消</button></em><em v-else>{{downloadPhaseLabel}}</em></article>
          </div>
        </section>
        <section v-if="!isProject&&server.status!=='planning'&&selectedTasks.length" class="card workflow"><header><p><small>服务器任务</small><b>后端执行记录</b></p><button @click="openTaskCenter()">查看全部<ChevronRight/></button></header><div v-for="(task,index) in selectedTasks" :key="task.id" class="step" :class="{done:task.status==='completed',active:['awaiting_approval','queued','running','cancelling'].includes(task.status),failed:['failed','interrupted','rollback_failed'].includes(task.status)}"><span><i>{{task.status==='completed'?'✓':task.status==='failed'||task.status==='rollback_failed'?'!':index+1}}</i><u v-if="index<selectedTasks.length-1"/></span><p><b>{{task.title}}</b><small>{{task.kind}} · {{task.progress}}%</small></p><em>{{taskStatusLabel[task.status]??task.status}}</em></div></section>
        <section v-else-if="!isProject&&server.status!=='planning'" class="card honest-empty"><Activity/><p><b>暂无执行任务</b><small>后端尚未返回这台服务器的任务记录。</small></p></section>
      </div>
      <div v-else-if="tab==='build'&&isProject" class="project-build-view"><ProjectBuildManager :server-id="selectedId" @completed="() => loadFiles(currentPath)"/></div>
      <div v-else-if="tab==='files'" class="files-view">
        <aside @contextmenu="openFileContextMenu($event,null)">
           <header><span>{{currentPath||(isProject?'项目文件':'服务器文件')}}</span><button title="刷新文件树" @click="loadFiles(currentPath)"><RefreshCw/></button><button :disabled="busy||!canTransferFiles" title="上传文件到当前目录" @click="triggerFileUpload"><FileUp/></button><input ref="fileUploadInput" type="file" style="display:none" @change="uploadWorkspaceFile"/><button data-new-entry-toggle title="新建文件" @click="showNewFile&& !showNewFolder?cancelNewEntry():openNewEntry('file')"><FileCode2/></button><button data-new-entry-toggle title="新建目录" @click="showNewFolder&& !showNewFile?cancelNewEntry():openNewEntry('folder')"><Folder/></button></header>
           <form v-if="showNewFile" class="new-entry-form" @submit.prevent="submitNewEntry"><input v-model="newFileName" autofocus spellcheck="false" aria-label="新文件名" placeholder="文件名，回车创建 · Esc 取消" @keydown.esc.prevent="cancelNewEntry"/></form><form v-if="showNewFolder" class="new-entry-form" @submit.prevent="submitNewEntry"><input v-model="newFolderName" autofocus spellcheck="false" aria-label="新目录名" placeholder="目录名，回车创建 · Esc 取消" @keydown.esc.prevent="cancelNewEntry"/></form>
          <button v-if="parentPath!==null" @click="loadFiles(parentPath||'')"><Folder/><span>..</span></button>
          <button v-for="entry in fileEntries" :key="entry.path" :class="{active:activeFile===entry.path}" @click="openEntry(entry)" @contextmenu="openFileContextMenu($event,entry)"><Folder v-if="entry.kind==='folder'"/><FileCode2 v-else/><span>{{entry.name}}</span><small v-if="entry.kind==='file'">{{entry.size<1024?entry.size+' B':Math.ceil(entry.size/1024)+' KB'}}</small></button>
        </aside>
        <section><header><FileCode2/>{{activeFile||'未选择文件'}}</header><textarea v-if="activeFile" ref="fileEditor" v-model="fileContent" class="config-editor" :readonly="fileReadonly" spellcheck="false" @select="captureFileSelection" @click="captureFileSelection" @keyup="captureFileSelection"/><div v-else class="empty"><Files/><b>{{currentPath||(isProject?'项目工作区':'服务器工作区')}}</b><small>{{isProject?'新建文件，或选择已有文本文件进行编辑':'选择文本文件进行安全编辑'}}</small></div><footer><span>UTF-8　LF　{{fileReadonly?'只读文件':'路径保护已开启'}}</span><nav><button v-if="activeFile" :disabled="busy||!canTransferFiles" @click="downloadCurrentFile()"><Download/>下载当前文件</button><button v-if="activeFile" @click="addEntryToConversation()"><MessageSquareText/>{{hasFileSelection?'添加选中内容':'添加文件到对话'}}</button><button v-if="activeFile&&!fileReadonly" :disabled="busy" @click="saveCurrentFile">保存文件</button></nav></footer></section>
        <div v-if="fileContextMenu" class="file-context-menu" :style="{left:fileContextMenu.x+'px',top:fileContextMenu.y+'px'}" @click.stop>
          <template v-if="fileContextMenu.entry"><button @click="openContextEntry"><Folder v-if="fileContextMenu.entry.kind==='folder'"/><FileCode2 v-else/>打开</button><button :disabled="fileContextMenu.entry.kind==='folder'||busy||!canTransferFiles" @click="downloadContextFile"><Download/>下载文件</button><button v-if="fileContextMenu.entry.kind==='folder'" @click="createFromContext('file',fileContextMenu.entry)"><FileCode2/>在此新建文件</button><button v-if="fileContextMenu.entry.kind==='folder'" @click="createFromContext('folder',fileContextMenu.entry)"><Folder/>新建子目录</button><button v-if="fileContextMenu.entry.kind==='file'" @click="addContextEntryToConversation"><MessageSquareText/>添加到对话</button><button @click="copyContextPath"><Copy/>复制相对路径</button><i/><button @click="openFileAction('rename',fileContextMenu.entry)"><Pencil/>重命名</button><button class="danger" @click="openFileAction('delete',fileContextMenu.entry)"><Trash2/>删除</button></template>
          <template v-else><button @click="createFromContext('file')"><FileCode2/>新建文件</button><button @click="createFromContext('folder')"><Folder/>新建目录</button><button @click="fileContextMenu=null;loadFiles(currentPath)"><RefreshCw/>刷新</button><button @click="copyContextPath"><Copy/>复制当前路径</button></template>
        </div>
      </div>
      <div v-else-if="isProject" class="work-scroll workspace-state"><SquareTerminal/><small>AUTHENTICATED SHELL</small><h2>项目 Shell 尚未连接</h2><p>项目模式不会暴露未鉴权的本机命令接口。连接拥有完整 Shell 能力的 Sculk Agent 后，可从云端持久终端进入这个工作区。</p><button @click="surface='settings'"><Settings/>打开连接设置</button></div>
      <div v-else class="terminal-view"><header><span><i :class="{live:logStreamLive}"/>{{server.name}} / 控制台 · {{logStreamLive?'实时连接':'等待日志连接'}}</span><Search/></header><main ref="termScroller"><div v-if="!terminal.length" class="terminal-empty">暂无服务器日志</div><p v-for="(line,index) in terminal" :key="index" :class="{ai:line.includes('AI]')}">{{line}}</p></main><form @submit.prevent="runCommand"><ChevronRight/><input v-model="command" :disabled="!terminalCommandReady||busy" :placeholder="terminalCommandPlaceholder"/><button :disabled="!terminalCommandReady||busy||!command.trim()">执行</button></form></div>
      </template>
      <AutomationView v-else-if="surface==='automation'&&selectedId&&!isProject" :server-id="selectedId" :tasks="tasks" :focused-task-id="focusedTaskId" @task-updated="upsertTask" @refresh-requested="loadDashboard(false)"/>
      <CommunityView v-else-if="surface==='community'&&selectedId" :server-id="selectedId"/>
      <section v-else-if="surface==='automation'&&isProject" class="workspace-state surface-state"><FolderTree/><h2>通用项目任务</h2><p>项目文件与独立对话已经可用；服务器启动、停服和诊断任务不会混入通用项目。</p><button @click="surface='control';tab='files';loadFiles()"><Files/>返回项目文件</button></section>
      <section v-else-if="surface==='automation'||surface==='community'" class="workspace-state surface-state"><Server/><h2>请先创建服务器</h2><p>这个功能需要一个真实的服务器项目，不会使用演示 ID 发起请求。</p><button @click="openCreate"><Plus/>创建服务器</button></section>
       <IntegrationsView v-else-if="surface==='integrations'"/>
       <SettingsView
         v-else-if="surface==='settings'"
         :initial-section="cloudWorkspaceLaunch ? 'account' : 'general'"
         :initial-cloud-tab="cloudWorkspaceLaunch ? 'workspace' : 'overview'"
         @apply-server-template="loadServerTemplate"
       />
       <MirrorCenterView v-else-if="surface==='mirror'" :initial-core="server.core" :initial-minecraft="server.version"/>
    </section>
    <div v-if="showOpenDirectory" class="modal-backdrop" @click.self="closeOpenDirectory">
      <section class="action-modal open-directory-modal">
        <header><div><small>OPEN EXISTING DIRECTORY</small><h2>打开已有目录</h2></div><button :disabled="openingDirectory" title="关闭" @click="closeOpenDirectory"><X/></button></header>
        <main>
          <div class="field"><label>{{openDirectoryModeLabel}}目录路径</label><input v-model="openDirectoryPath" autofocus autocomplete="off" spellcheck="false" :placeholder="workspaceMode==='server'?'例如 C:\\servers\\survival 或 /srv/minecraft':'例如 C:\\work\\my-project 或 /home/user/project'" @keydown.enter.prevent="importExistingDirectory"/><small class="field-hint">请输入运行后端所在机器上的绝对路径。后端会读取目录中的 sculk.yml、server.properties、核心文件和常见配置，并将结果接入当前工作台。</small></div>
          <div class="field"><label>显示名称（可选）</label><input v-model="openDirectoryName" autocomplete="off" maxlength="64" placeholder="留空使用目录或配置中的名称" @keydown.enter.prevent="importExistingDirectory"/></div>
          <div class="directory-import-hint"><FolderOpen/><p><b>{{workspaceMode==='server'?'服务器模式会自动读取配置':'项目模式会保留目录中的现有文件'}}</b><small>{{workspaceMode==='server'?'不会下载或覆盖核心；检测完成后可直接查看 server.properties、插件和日志。':'不会移动或复制文件；接管后通过工作区文件编辑和对话继续工作。'}}</small></p></div>
          <div v-if="openDirectoryError" class="environment-loading error"><AlertTriangle/><span><b>打开目录失败</b><small>{{openDirectoryError}}</small></span></div>
        </main>
        <footer><button class="back" :disabled="openingDirectory" @click="closeOpenDirectory">取消</button><button class="next" :disabled="openingDirectory||!openDirectoryPath.trim()" @click="importExistingDirectory"><LoaderCircle v-if="openingDirectory" class="spin"/><FolderOpen v-else/>打开目录</button></footer>
      </section>
    </div>
    <div v-if="showCreate" class="modal-backdrop" @click.self="showCreate=false">
      <section v-if="workspaceMode==='project'" class="create-modal project-create-modal">
        <header><div><small>NEW GENERAL PROJECT</small><h2>创建通用项目</h2></div><button @click="showCreate=false"><X/></button></header>
        <main class="wizard-page location-page">
          <div class="field wide"><label>项目名称</label><input v-model="projectForm.name" autofocus placeholder="例如：网站、插件或自动化脚本" @keydown.enter.prevent="createProject"/></div>
          <div class="field wide"><label>项目位置</label><select v-model="projectForm.location"><option value="local">本机 · 默认工作区目录</option><option v-for="connection in remoteConnections" :key="connection.id" :value="'remote:'+connection.id" disabled>{{connection.name}} · {{connection.host}}（暂未支持）</option></select></div>
          <div class="location-preview wide"><FolderTree/><p><b>{{projectLocationLabel}}</b><small>只创建一个独立空文件夹，不生成 Minecraft 核心、插件、配置、启动脚本或 EULA 文件。</small></p></div>
          <div class="project-mode-note wide"><FileCode2/><p><b>通用 Web IDE 工作区</b><small>创建后直接进入文件编辑；对话、模型、ACP Agent、Skills 与 MCP 均可按项目独立使用。</small></p></div>
        </main>
        <footer><button class="back" :disabled="creating" @click="showCreate=false">取消</button><button class="next" :disabled="creating||!projectForm.name.trim()" @click="createProject"><LoaderCircle v-if="creating" class="spin"/><Plus v-else/>创建项目</button></footer>
      </section>
      <section v-else class="create-modal">
        <header><div><small>NEW SERVER WORKSPACE</small><h2>创建 Minecraft 服务器</h2></div><button @click="showCreate=false"><X/></button></header>
        <nav class="wizard-steps"><span v-for="index in 4" :key="index" :class="{active:createStep===index,done:createStep>index}"><i><Check v-if="createStep>index"/><template v-else>{{index}}</template></i>{{['名称与位置','服务器参数','环境检查','确认创建'][index-1]}}</span></nav>
        <main v-if="createStep===1" class="wizard-page location-page">
          <div class="field wide"><label>服务器项目名称</label><input v-model="createForm.name" placeholder="例如：深暗生存服"/></div>
          <div class="field wide"><label>服务器位置</label><select v-model="createForm.location"><option value="local">本机 · 默认数据目录</option><option v-for="connection in remoteConnections" :key="connection.id" :value="'remote:'+connection.id" disabled>{{connection.name}} · {{connection.host}}（暂未支持）</option><option v-if="!remoteConnections.length" value="remote:placeholder" disabled>远程服务器（接口已预留，暂未支持）</option></select></div>
          <div class="location-preview wide"><MapPin/><p><b>{{selectedLocationLabel}}</b><small>服务器项目会使用独立目录；智能创建阶段只写入可迁移的 sculk.yml 标识，不下载核心。</small></p></div>
          <div class="portable-template-import wide"><FileUp/><p><b>载入便携配置</b><small>支持开放的 Sculk JSON 参数模板；导入只填充向导，不会创建文件或继承 EULA。</small></p><button @click="manifestInput?.click()">选择配置文件</button><input ref="manifestInput" type="file" accept="application/json,.json" @change="importServerTemplate"/></div>
          <div class="creation-mode wide"><article><span><FolderTree/></span><p><b>普通创建</b><small>继续选择核心、版本、内存和端口，完成环境检查后创建工作区。</small></p><button :disabled="!createForm.name.trim()" @click="createStep=2">继续配置<ChevronRight/></button></article><article class="smart"><span><BrainCircuit/></span><p><b>智能创建</b><small>先创建可迁移的 sculk.yml 标识，不预设或下载核心；随后通过独立对话完成选型与部署。</small></p><button :disabled="!createForm.name.trim()||creating" @click="createSmartServer"><LoaderCircle v-if="creating" class="spin"/><Sparkles v-else/>进入智能规划</button></article></div>
        </main>
        <main v-else-if="createStep===2" class="wizard-page">
          <div class="field"><label>服务端核心</label><select v-model="createForm.core" :disabled="!availableCores.length"><option v-if="createForm.core&&!availableCores.includes(createForm.core)" :value="createForm.core" disabled>{{createForm.core}}（当前目录不可用）</option><option v-if="!availableCores.length" value="">核心目录不可用</option><option v-for="core in availableCores" :key="core">{{core}}</option></select></div>
          <div class="field"><label>Minecraft 版本</label><select v-model="createForm.version" :disabled="!createMinecraftVersions.length"><option v-if="createForm.version&&!createMinecraftVersions.includes(createForm.version)" :value="createForm.version" disabled>{{createForm.version}}（当前核心不支持）</option><option v-if="!createMinecraftVersions.length" value="">版本目录不可用</option><option v-for="version in createMinecraftVersions" :key="version">{{version}}</option></select></div>
          <div class="field"><label>最大内存</label><div class="input-unit"><input v-model.number="createForm.memory_gb" type="number" min="2" :max="Math.max(2,reasonableMemoryMax)" step="1"/><span>GB</span></div><small v-if="memoryIssue" class="field-error">{{memoryIssue}}</small><small v-else-if="totalMemoryGb!==null" class="field-hint">检测到 {{totalMemoryGb.toFixed(1)}} GB，总分配上限 {{reasonableMemoryMax}} GB</small></div>
          <div class="field"><label>服务器端口</label><input v-model.number="createForm.port" type="number" min="1024" max="65535" step="1"/><small v-if="portIssue" class="field-error">{{portIssue}}</small></div>
          <div class="core-note wide"><Sparkles/><p><b>{{importedTemplateTitle?`已载入模板：${importedTemplateTitle}`:'参数只用于普通创建'}}</b><small>{{importedTemplateTitle?'模板仍需通过本机目录、端口和资源检查；不兼容参数不会被静默替换。':'核心选择不再硬编码推荐。智能创建会在对话中结合玩法、插件生态和维护成本给出选型建议。'}}</small></p></div>
        </main>
        <main v-else-if="createStep===3" class="wizard-page environment-page">
          <div v-if="systemState==='loading'" class="environment-loading"><LoaderCircle class="spin"/>正在向后端查询环境…</div>
          <div v-else-if="systemState==='error'" class="environment-loading error"><PlugZap/><span><b>无法获取环境信息</b><small>{{systemError}}</small></span><button @click="loadSystemInfo">重试</button></div>
          <div class="environment-card" :class="{ok:javaReady,error:systemState==='ready'&&!javaReady,unknown:systemState!=='ready'}"><span><Coffee/></span><p><b>Java 运行环境</b><small v-if="systemState!=='ready'">尚未取得 Java 检测结果</small><small v-else-if="javaReady">{{systemInfo?.java_version}} · {{systemInfo?.java_executable||systemInfo?.java_home||'可执行路径未返回'}}</small><small v-else-if="systemInfo?.java_installed">{{systemInfo.java_version||'已安装版本未知'}} 与当前环境不兼容，推荐 Java {{systemInfo.recommended_java}}</small><small v-else>{{systemInfo?.java_install_hint||('未检测到 Java，推荐安装 Java '+systemInfo?.recommended_java)}}</small></p><div class="environment-actions"><em>{{javaReady?'可用':systemState==='ready'?(systemInfo?.java_installed?'不兼容':'未安装'):'未知'}}</em><button v-if="systemState==='ready'&&!javaReady&&systemInfo?.java_install_supported" :disabled="javaInstallState==='installing'" @click="installJava"><LoaderCircle v-if="javaInstallState==='installing'" class="spin"/><Download v-else/>{{javaInstallState==='installing'?'正在安装…':'安装 Java '+systemInfo?.recommended_java}}</button></div></div>
          <div v-if="javaInstallState==='success'" class="install-result success">Java 安装完成，环境信息已刷新。</div><div v-else-if="javaInstallState==='error'" class="install-result error">Java 安装失败：{{javaInstallError}}</div>
          <div class="environment-card" :class="{ok:systemState==='ready'&&systemInfo?.data_dir_writable,error:systemState==='ready'&&!systemInfo?.data_dir_writable,unknown:systemState!=='ready'}"><span><HardDrive/></span><p><b>服务器工作区</b><small>{{systemState==='ready'?(systemInfo?.data_dir||'后端未返回数据目录'):'数据目录未知'}}</small></p><em>{{systemState==='ready'?(systemInfo?.data_dir_writable?'可写入':'不可写'):'未知'}}</em></div>
          <div class="environment-card" :class="{ok:diskReady,error:systemState==='ready'&&systemInfo?.data_dir_free_bytes!==undefined&&!diskReady,unknown:systemState!=='ready'||systemInfo?.data_dir_free_bytes===undefined}"><span><Database/></span><p><b>数据目录空间</b><small>{{systemState==='ready'?'可用 '+formatBytes(systemInfo?.data_dir_free_bytes):'磁盘空间未知'}}；普通创建至少需要 2 GB</small></p><em>{{systemInfo?.data_dir_free_bytes===undefined?'未知':diskReady?'充足':'不足'}}</em></div>
          <div class="environment-card" :class="{ok:totalMemoryGb!==null&&!memoryIssue,error:totalMemoryGb!==null&&!!memoryIssue,unknown:totalMemoryGb===null}"><span><Cpu/></span><p><b>系统内存</b><small>{{totalMemoryGb===null?'总内存未知':'共 '+totalMemoryGb.toFixed(1)+' GB；当前计划分配 '+createForm.memory_gb+' GB'}}</small></p><em>{{totalMemoryGb===null?'未知':memoryIssue?'不满足':'充足'}}</em></div>
          <div class="environment-card info" :class="{unknown:systemState!=='ready'}"><span><ShieldCheck/></span><p><b>系统架构</b><small>{{systemState==='ready'?((systemInfo?.os||'系统未知')+' · '+(systemInfo?.arch||'架构未知')):'尚未识别系统与架构'}}</small></p><em>{{systemState==='ready'?'已识别':'未知'}}</em></div>
          <div class="environment-summary" :class="{blocked:!!environmentIssue,unknown:!environmentIssue&&environmentUnknown}"><ShieldCheck/><p><b>{{environmentIssue?'环境检查尚未通过':environmentUnknown?'基础检查通过，部分资源信息未知':'环境检查已通过'}}</b><small>{{environmentIssue||(environmentUnknown?'Java 与目录检查已通过；后端未返回完整磁盘或内存信息，因此不标记为全部正常。':'数据目录、Java 与资源条件满足普通创建要求。')}}</small></p></div>
        </main>
        <main v-else class="wizard-page review-page">
          <div class="review-server"><span><Server/></span><p><b>{{createForm.name}}</b><small>{{createForm.core}} {{createForm.version}} · {{systemInfo?.java_version}}（兼容）</small></p></div>
          <dl><div><dt>服务器位置</dt><dd>本机默认数据目录</dd></div><div><dt>内存限制</dt><dd>{{createForm.memory_gb}} GB</dd></div><div><dt>监听端口</dt><dd>{{createForm.port}}</dd></div><div><dt>初始状态</dt><dd>停止 · 等待核心下载</dd></div></dl>
          <label class="eula-check"><input v-model="createForm.eula_accepted" type="checkbox"/><span>我已阅读并同意 Minecraft EULA，允许工具生成 <code>eula=true</code></span></label>
        </main>
        <footer><button class="back" :disabled="createStep===1||creating" @click="createStep--">上一步</button><button v-if="createStep===1" class="next" :disabled="!createForm.name.trim()" @click="createStep=2">普通创建<ChevronRight/></button><button v-else-if="createStep<4" class="next" :disabled="createStep===2?!!parameterIssue:!!environmentIssue" @click="advanceCreateStep">继续<ChevronRight/></button><button v-else class="next" :disabled="!createForm.eula_accepted||creating||!!parameterIssue||!!environmentIssue" @click="createNewServer"><LoaderCircle v-if="creating" class="spin"/><Plus v-else/>创建服务器</button></footer>
      </section>
    </div>

    <div v-if="conversationDialog" class="modal-backdrop" @click.self="conversationDialog=null">
      <section class="action-modal">
        <header><div><small>CONVERSATION TASK</small><h2>{{conversationDialog.kind==='rename'?'重命名对话':conversationDialog.kind==='group'?'移动到组':'删除对话'}}</h2></div><button @click="conversationDialog=null"><X/></button></header>
        <main v-if="conversationDialog.kind==='delete'" class="danger-copy"><Trash2/><p><b>删除「{{conversationDialog.conversation.title}}」？</b><small>该对话的全部历史消息会被永久删除，工作区本身不受影响。</small></p></main>
        <main v-else><div class="field"><label>{{conversationDialog.kind==='rename'?'新名称':'分组名称'}}</label><input v-model="conversationDialogValue" :placeholder="conversationDialog.kind==='rename'?'输入对话任务名称':'输入分组名称；留空移出分组'" @keydown.enter.prevent="submitConversationDialog"/></div></main>
        <footer><button class="back" @click="conversationDialog=null">取消</button><button class="next" :class="{danger:conversationDialog.kind==='delete'}" :disabled="conversationDialog.kind==='rename'&&!conversationDialogValue.trim()" @click="submitConversationDialog">{{conversationDialog.kind==='delete'?'删除对话':'保存'}}</button></footer>
      </section>
    </div>

    <div v-if="fileActionDialog" class="modal-backdrop" @click.self="fileActionDialog=null">
      <section class="action-modal">
        <header><div><small>WORKSPACE FILE</small><h2>{{fileActionDialog.kind==='rename'?'重命名':'删除'}}{{fileActionDialog.entry.kind==='folder'?'目录':'文件'}}</h2></div><button @click="fileActionDialog=null"><X/></button></header>
        <main v-if="fileActionDialog.kind==='rename'"><div class="field"><label>新名称</label><input v-model="fileActionValue" autocomplete="off" @keydown.enter.prevent="submitFileAction"/><small class="field-hint">可直接修改后缀，例如 README.md、notes.txt 或 config.yml。</small></div><small class="file-action-path">{{fileActionDialog.entry.path}}</small></main>
        <main v-else><div class="danger-copy"><Trash2/><p><b>删除「{{fileActionDialog.entry.name}}」？</b><small>{{fileActionDialog.entry.kind==='folder'?'目录中的全部文件和子目录都会永久删除。':'该文件会从工作区永久删除。'}}此操作不可撤销。</small></p></div><small class="file-action-path">{{fileActionDialog.entry.path}}</small></main>
        <footer><button class="back" @click="fileActionDialog=null">取消</button><button class="next" :class="{danger:fileActionDialog.kind==='delete'}" :disabled="fileActionBusy||(fileActionDialog.kind==='rename'&&!fileActionValue.trim())" @click="submitFileAction"><LoaderCircle v-if="fileActionBusy" class="spin"/>{{fileActionDialog.kind==='rename'?'保存':'确认删除'}}</button></footer>
      </section>
    </div>

    <div v-if="deleteServerTarget" class="modal-backdrop" @click.self="deleteServerTarget=null">
      <section class="action-modal delete-server-modal">
        <header><div><small>DELETE WORKSPACE</small><h2>{{deleteServerStep===1?`删除${workspaceKind(deleteServerTarget)==='project'?'项目':'服务器项目'}`:'确认删除磁盘文件'}}</h2></div><button @click="deleteServerTarget=null"><X/></button></header>
        <main v-if="deleteServerStep===1">
          <div class="danger-copy"><Trash2/><p><b>从项目列表删除「{{deleteServerTarget.name}}」？</b><small>关联的对话任务、自动化任务和运行状态会一并移除。</small></p></div>
          <label class="delete-files-check"><input v-model="deleteServerFiles" type="checkbox"/><span><b>同时删除磁盘上的全部文件</b><small>{{workspaceKind(deleteServerTarget)==='project'?'包括项目目录中的源码、配置和其他文件。':'包括地图、插件、配置和日志。'}}勾选后还需要第二次确认。</small></span></label>
        </main>
        <main v-else>
          <div class="danger-copy critical"><ShieldCheck/><p><b>这是不可恢复的磁盘删除</b><small>将永久删除「{{deleteServerTarget.name}}」的完整工作区目录。请手动输入 <code>delete all</code>。</small></p></div>
          <div class="field"><label>确认文本</label><input v-model="deleteServerConfirmation" autocomplete="off" placeholder="delete all" @keydown.enter.prevent="executeDeleteServer"/></div>
        </main>
        <footer><button class="back" @click="deleteServerStep===2?deleteServerStep=1:deleteServerTarget=null">{{deleteServerStep===2?'上一步':'取消'}}</button><button class="next danger" :disabled="busy||(deleteServerStep===2&&deleteServerConfirmation!=='delete all')" @click="deleteServerStep===1?advanceDeleteServer():executeDeleteServer()"><LoaderCircle v-if="busy" class="spin"/><Trash2 v-else/>{{deleteServerStep===1?(deleteServerFiles?'继续确认':'移除项目'):'永久删除文件'}}</button></footer>
      </section>
    </div>

  </main>
</template>

<style scoped>
.workspace-mode-switch{display:grid;grid-template-columns:1fr 1fr;gap:3px;margin:10px 0 0;padding:3px;border:1px solid rgba(255,255,255,.075);border-radius:9px;background:#0a0f14}.workspace-mode-switch button{height:31px;display:flex;align-items:center;justify-content:center;gap:5px;padding:0 6px;border:0;border-radius:6px;color:#75818e;background:transparent;font-weight:600}.workspace-mode-switch button:hover{color:#c8d0d8;background:rgba(255,255,255,.04)}.workspace-mode-switch button.active{color:#9ce8d6;background:rgba(50,213,176,.11);box-shadow:inset 0 0 0 1px rgba(50,213,176,.12)}.workspace-mode-switch svg{width:14px}.workspace-mode-switch+.create{margin-top:8px}
.workspace-actions{display:grid;gap:6px;margin:14px 0 12px}.workspace-actions .create{margin:0}.workspace-actions .open-existing{height:31px;display:flex;align-items:center;justify-content:center;gap:7px;border:1px solid rgba(255,255,255,.09);border-radius:7px;color:#8d99a5;background:rgba(255,255,255,.025);font-size:10px}.workspace-actions .open-existing:hover{color:#c9d3dc;background:rgba(255,255,255,.06)}.workspace-actions .open-existing svg{width:15px}.collapsed .workspace-actions{margin-top:14px}.collapsed .workspace-actions .create,.collapsed .workspace-actions .open-existing{width:36px;align-self:center;padding:0}.first-run-actions{display:flex;align-items:center;justify-content:center;gap:8px;margin-top:5px}.first-run-actions button{margin-top:0!important}.first-run-actions button.secondary{border-color:rgba(255,255,255,.1);color:#8f9ba7;background:rgba(255,255,255,.035)}.first-run-actions button.secondary:hover{color:#d1d9e0;background:rgba(255,255,255,.07)}.first-run-actions button svg{width:14px}
.directory-import-summary{margin-bottom:12px;padding:15px 16px;border:1px solid rgba(50,213,176,.17);border-radius:10px;background:rgba(50,213,176,.045)}.directory-import-summary>header{display:flex;align-items:flex-start;justify-content:space-between;gap:10px}.directory-import-summary>header p{display:flex;flex-direction:column;margin:0}.directory-import-summary>header small{color:#6b9d90;font-size:8px;font-weight:700;letter-spacing:.12em}.directory-import-summary>header b{margin-top:4px;font-size:12px}.directory-import-summary>header button{width:25px;height:25px;display:grid;place-items:center;border:0;border-radius:5px;color:#6f7c88;background:transparent}.directory-import-summary>header button:hover{color:#d8e2e7;background:rgba(255,255,255,.05)}.directory-import-summary>header svg{width:13px}.directory-import-path{display:flex;align-items:center;gap:9px;margin-top:12px;padding:9px;border-radius:7px;background:rgba(0,0,0,.12)}.directory-import-path>svg{width:16px;flex:none;color:#72d5bf}.directory-import-path p{display:flex;min-width:0;flex-direction:column;margin:0}.directory-import-path b,.directory-import-path small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.directory-import-path b{font-size:10px}.directory-import-path small{margin-top:3px;color:#687a85;font:8px 'Cascadia Code',monospace}.directory-import-summary dl{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:6px;margin:10px 0 0}.directory-import-summary dl div{display:flex;justify-content:space-between;gap:8px;padding:7px 8px;border:1px solid rgba(255,255,255,.06);border-radius:6px;background:rgba(255,255,255,.018);font-size:8px}.directory-import-summary dt{color:#687681}.directory-import-summary dd{min-width:0;margin:0;overflow:hidden;color:#b6d9cf;text-overflow:ellipsis;white-space:nowrap}.directory-import-warnings{display:flex;align-items:flex-start;gap:8px;margin-top:10px;padding:9px;border:1px solid rgba(243,167,92,.18);border-radius:7px;color:#d4a16a;background:rgba(243,167,92,.045)}.directory-import-warnings>svg{width:14px;flex:none;margin-top:1px}.directory-import-warnings p{display:flex;min-width:0;flex-direction:column;gap:3px;margin:0}.directory-import-warnings b{font-size:8px}.directory-import-warnings small{color:#a88766;font-size:8px;line-height:1.45}.directory-import-summary>footer{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-top:11px;padding-top:10px;border-top:1px solid rgba(255,255,255,.06)}.directory-import-summary>footer span{color:#6d7c86;font-size:8px}.directory-import-summary>footer button{height:27px;display:flex;align-items:center;gap:5px;padding:0 9px;border:1px solid rgba(50,213,176,.18);border-radius:6px;color:#88d9c7;background:rgba(50,213,176,.06);font-size:8px}.directory-import-summary>footer button svg{width:12px}.directory-import-hint{display:flex;align-items:flex-start;gap:10px;padding:11px 12px;border:1px solid rgba(50,213,176,.13);border-radius:8px;color:#6ed1ba;background:rgba(50,213,176,.045)}.directory-import-hint>svg{width:17px;flex:none;margin-top:1px}.directory-import-hint p{display:flex;flex-direction:column;margin:0}.directory-import-hint b{color:#b4e9dc;font-size:9px}.directory-import-hint small{margin-top:4px;color:#6b7c86;font-size:8px;line-height:1.5}
.project-build-view{flex:1;min-height:0;overflow:auto;background:#0d1319}.project-workspace{min-height:360px;display:flex;align-items:center;justify-content:center;flex-direction:column;padding:40px;text-align:center;border:1px solid rgba(50,213,176,.12);border-radius:12px;background:radial-gradient(circle at 50% 25%,rgba(50,213,176,.08),transparent 55%),#11161c}.project-workspace>span{width:58px;height:58px;display:grid;place-items:center;border-radius:14px;color:#72dec5;background:rgba(50,213,176,.1)}.project-workspace>span svg{width:28px}.project-workspace>small{margin-top:18px;color:#70b7a7;font-weight:700;letter-spacing:.14em}.project-workspace h2{margin:8px 0 0}.project-workspace p{max-width:570px;margin:12px 0 0;color:#87939f}.project-workspace>div{display:flex;gap:8px;margin-top:20px}.project-workspace button{height:36px;display:flex;align-items:center;gap:6px;padding:0 13px;border:1px solid rgba(50,213,176,.16);border-radius:8px;color:#9ee7d6;background:rgba(50,213,176,.07)}.project-workspace button:first-child{border:0;color:#06251e;background:var(--accent)}.project-workspace button svg{width:15px}
.project-create-modal .wizard-page{min-height:320px}.project-mode-note{display:flex;align-items:center;gap:12px;padding:14px;border:1px solid rgba(156,140,255,.14);border-radius:9px;color:#ada2f3;background:rgba(156,140,255,.055)}.project-mode-note>svg{width:20px;flex:none}.project-mode-note p{display:flex;flex-direction:column;margin:0}.project-mode-note b{color:#d5d0fa}.project-mode-note small{margin-top:4px;color:#747f8c}
@media(max-width:1180px){.workspace-mode-switch{display:none}.workspace-actions .open-existing span{display:none}.workspace-actions .open-existing{width:36px;align-self:center;padding:0}}
.chat-panel>header button.active{color:#dff8f1;background:rgba(50,213,176,.09)}
.message-search{height:42px;display:flex;align-items:center;gap:7px;flex:0 0 auto;padding:0 14px;border-bottom:1px solid rgba(255,255,255,.07);background:#0f141a;color:#6f7b87}
.message-search>svg{width:14px;flex:none}.message-search input{min-width:0;flex:1;border:0;outline:0;color:#dfe5eb;background:transparent;font-size:10px}.message-search input::placeholder{color:#596572}.message-search>span{min-width:46px;color:#65717e;font-size:8px;text-align:right}.message-search button{width:25px;height:25px;display:grid;place-items:center;padding:0;border:0;border-radius:5px;color:#6f7b87;background:transparent}.message-search button:hover:not(:disabled){color:#e7edf2;background:rgba(255,255,255,.05)}.message-search button:disabled{opacity:.3}.message-search button svg{width:13px}
.message.search-match{padding:8px;margin-inline:-8px;border-radius:10px;background:rgba(156,140,255,.035)}.message.search-match.active-match{box-shadow:inset 2px 0 #9c8cff;background:rgba(156,140,255,.075)}
.message-body{position:relative;min-width:0}.message.user .message-body{padding:10px 12px;border:1px solid rgba(156,140,255,.11);border-radius:10px 3px 10px 10px;background:rgba(156,140,255,.09)}
.message-body :deep(.message-content>p){padding:0;border:0;border-radius:0;background:transparent}.message.user .message-body :deep(.message-content){color:#d7d9e7}
.message-warning{display:flex;align-items:flex-start;gap:6px;margin:7px 0 0!important;padding:7px 8px!important;border:1px solid rgba(243,167,92,.15)!important;border-radius:6px!important;color:#c89a6c!important;background:rgba(243,167,92,.05)!important;font-size:8px!important;line-height:1.5!important}.message-warning svg{width:12px;flex:none;margin-top:1px}.message.error .message-warning{border-color:rgba(226,92,101,.17)!important;color:#d38a8f!important;background:rgba(226,92,101,.05)!important}
.inline-task{width:min(340px,100%);margin-top:10px;padding:10px;border:1px solid rgba(255,255,255,.08);border-radius:8px;background:#10161c}.inline-task>header{display:flex;align-items:center;justify-content:space-between;margin:0 0 8px}.inline-task>header span{display:flex;align-items:center;min-width:0;gap:6px}.inline-task>header svg{width:13px;color:#32d5b0}.inline-task>header b{overflow:hidden;font-size:9px;text-overflow:ellipsis;white-space:nowrap}.inline-task>header em{padding:2px 5px;border-radius:4px;color:#72d6be;background:rgba(50,213,176,.08);font:normal 7px Inter}.inline-task>header em.medium{color:#ebb078;background:rgba(243,167,92,.08)}.inline-task>header em.high{color:#e98a90;background:rgba(226,92,101,.08)}.inline-task>div{height:3px;overflow:hidden;border-radius:4px;background:#252c34}.inline-task>div i{display:block;height:100%;border-radius:inherit;background:#32d5b0}.inline-task>footer{display:flex;align-items:center;justify-content:space-between;margin-top:8px}.inline-task>footer span{color:#64717d;font-size:8px}.inline-task>footer button{height:24px;display:flex;align-items:center;gap:3px;padding:0 6px;border:0;border-radius:5px;color:#83d9c5;background:rgba(50,213,176,.06);font-size:8px}.inline-task>footer button:hover{background:rgba(50,213,176,.1)}.inline-task>footer svg{width:11px}
.message-state{padding:2px 5px;border-radius:4px;color:#b99572;background:rgba(243,167,92,.09);font:normal 7px Inter}.message-state.error{color:#d78489;background:rgba(226,92,101,.09)}
.message-tools{min-height:24px;display:flex;align-items:center;gap:2px;margin-top:4px;opacity:0;transition:opacity .15s}.message:hover .message-tools,.message:focus-within .message-tools,.message.error .message-tools,.message.interrupted .message-tools{opacity:1}.message.user .message-tools{justify-content:flex-end}.message-tools button{height:23px;display:flex;align-items:center;gap:4px;padding:0 6px;border:0;border-radius:5px;color:#66727f;background:transparent;font-size:8px}.message-tools button:hover{color:#cbd3db;background:rgba(255,255,255,.05)}.message-tools svg{width:12px}
.chat-queue{display:flex;flex-direction:column;gap:4px;margin:0 0 7px;padding:7px;border:1px solid rgba(156,140,255,.14);border-radius:9px;background:rgba(13,18,24,.96);box-shadow:0 8px 24px rgba(0,0,0,.18)}.chat-queue>header{display:flex;align-items:center;justify-content:space-between;padding:1px 3px 4px;color:#8f84d8}.chat-queue>header span{display:flex;align-items:center;gap:5px;font-size:8px;font-weight:700}.chat-queue>header svg{width:12px}.chat-queue>header small{color:#5f6b77;font-size:7px}.chat-queue article{display:grid;grid-template-columns:20px minmax(0,1fr) auto;align-items:center;gap:7px;min-height:40px;padding:6px 7px;border:1px solid rgba(255,255,255,.055);border-radius:7px;background:#11171d}.chat-queue article.steer{border-color:rgba(50,213,176,.15);background:rgba(50,213,176,.035)}.chat-queue article>span{width:19px;height:19px;display:grid;place-items:center;border-radius:5px;color:#8f84d8;background:rgba(156,140,255,.1);font-size:7px}.chat-queue article.steer>span{color:#70d6bd;background:rgba(50,213,176,.1)}.chat-queue p{min-width:0;display:flex;flex-direction:column;gap:3px;margin:0}.chat-queue b{color:#aab4be;font-size:7px}.chat-queue small{overflow:hidden;color:#727e8a;font-size:8px;text-overflow:ellipsis;white-space:nowrap}.chat-queue textarea{min-height:46px;resize:vertical;padding:7px;border:1px solid rgba(255,255,255,.08);border-radius:6px;outline:0;color:#d8dfe6;background:#0d1217;font:9px/1.5 Inter}.chat-queue nav{display:flex;align-items:center;gap:2px}.chat-queue nav button{width:25px;height:25px;display:grid;place-items:center;border:0;border-radius:5px;color:#697582;background:transparent}.chat-queue nav button:hover{color:#d5dce3;background:rgba(255,255,255,.055)}.chat-queue nav button:nth-last-child(2):not(:first-child){color:#69cdb5}.chat-queue nav button:last-child{color:#c57479}.chat-queue nav svg{width:12px}
.chat-queue nav button.queue-steer{width:auto;grid-auto-flow:column;gap:4px;padding:0 7px;color:#69cdb5;font-size:7px}
.chat-queue>header>div{display:flex;align-items:center;gap:7px}.chat-queue>header>div button{height:22px;display:flex;align-items:center;gap:4px;padding:0 7px;border:1px solid rgba(50,213,176,.16);border-radius:5px;color:#79d7c1;background:rgba(50,213,176,.055);font-size:7px}.chat-queue>header>div button svg{width:10px}
.composer{position:relative}.composer.running{border-color:rgba(50,213,176,.2)}.composer textarea{min-height:42px;max-height:160px;overflow-y:auto}.composer textarea:disabled{cursor:not-allowed;opacity:.62}.composer .stop-generation{color:#ff9ba0;background:rgba(226,92,101,.11)}.composer .stop-generation:hover{background:rgba(226,92,101,.17)}.prompts button:disabled{opacity:.42;cursor:not-allowed}
.composer-assist{position:absolute;right:0;bottom:calc(100% + 8px);left:0;z-index:55;max-height:300px;overflow:auto;padding:7px;border:1px solid rgba(255,255,255,.1);border-radius:10px;background:#141a21;box-shadow:0 18px 46px rgba(0,0,0,.55)}.composer-assist>header{display:flex;align-items:center;justify-content:space-between;padding:4px 6px 6px}.composer-assist>header span{color:#8d98a3;font-size:8px;font-weight:700}.composer-assist>header small{color:#5f6b77;font:8px 'Cascadia Code',monospace}.composer-assist>button{width:100%;display:grid;grid-template-columns:22px minmax(0,1fr) auto;align-items:center;gap:7px;padding:7px;border:0;border-radius:6px;color:#788592;background:transparent;text-align:left}.composer-assist>button.active,.composer-assist>button:hover{color:#dce3e9;background:rgba(50,213,176,.07)}.composer-assist>button>svg{width:14px;color:#72d8c0}.composer-assist>button>span{min-width:0;display:flex;flex-direction:column;gap:3px}.composer-assist>button b,.composer-assist>button small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.composer-assist>button b{font-size:9px}.composer-assist>button small{color:#65717d;font-size:7px}.composer-assist>button em{color:#596572;font:normal 7px Inter}.composer-assist>p{display:flex;align-items:center;justify-content:center;gap:6px;margin:0;padding:13px;color:#64717d;font-size:8px}.composer-assist>p svg{width:13px}
.composer-status{min-height:19px;display:flex;align-items:center;justify-content:center;position:relative;color:#55616e;font-size:8px}.composer-status>span{display:flex;align-items:center;gap:5px}.composer-status>span i{width:5px;height:5px;border-radius:50%;background:#32d5b0;box-shadow:0 0 7px rgba(50,213,176,.6);animation:pulse 1.2s infinite}.composer-status>span svg{width:11px}.composer-status .safe{position:absolute;right:0;margin:0}
.composer-menu-anchor{position:relative;display:inline-flex}
.composer-menu{position:absolute;bottom:calc(100% + 8px);left:0;z-index:40;display:flex;flex-direction:column;gap:2px;width:250px;max-height:320px;overflow:auto;padding:8px;border:1px solid rgba(255,255,255,.09);border-radius:9px;background:#141a21;box-shadow:0 14px 38px rgba(0,0,0,.5)}
.composer-menu>small{padding:4px 6px 2px;color:#66727f;font-size:7px;text-transform:uppercase}
.composer-menu>small.group{margin-top:4px;border-top:1px solid rgba(255,255,255,.05);padding-top:7px;text-transform:none;color:#8b96a2}
.composer-menu>small.empty-hint{padding:6px;text-transform:none;line-height:1.6}
.composer-menu>button{display:flex;align-items:center;justify-content:space-between;gap:8px;width:100%;padding:7px 8px;border:0;border-radius:6px;background:transparent;color:#c7d1db;text-align:left;cursor:pointer}
.composer-menu>button:hover{background:rgba(255,255,255,.05)}
.composer-menu>button.picked{background:rgba(50,213,176,.08)}
.composer-menu>button span{display:flex;min-width:0;flex-direction:column;gap:3px}
.composer-menu>button b{overflow:hidden;font-size:8px;text-overflow:ellipsis;white-space:nowrap}
.composer-menu>button em{color:#697582;font:normal 7px Inter;line-height:1.5}
.composer-menu>button svg{width:12px;color:#32d5b0;flex:none}
.composer-menu.context-menu{width:280px}.composer-menu.context-menu>div{display:grid;grid-template-columns:22px minmax(0,1fr) 14px;align-items:center;gap:7px;padding:7px 6px;color:#73808c}.composer-menu.context-menu>div>svg{width:14px}.composer-menu.context-menu>div>svg:last-child{width:11px;color:#32d5b0}.composer-menu.context-menu>div span{display:flex;min-width:0;flex-direction:column;gap:3px}.composer-menu.context-menu>div b{color:#b9c3cd;font-size:8px}.composer-menu.context-menu>div em{overflow:hidden;color:#64717d;font:normal 7px/1.45 Inter;text-overflow:ellipsis;white-space:nowrap}
.composer-toolbar{display:flex;align-items:flex-start;gap:6px;margin-bottom:7px}.composer-agent-picker{flex:none}.composer-agent-picker .agent-picker{width:auto;max-width:180px;height:25px;display:flex;align-items:center;gap:5px;padding:0 8px;border:1px solid rgba(50,213,176,.13);border-radius:6px;color:#9ddfce;background:rgba(50,213,176,.055);font-size:9px;cursor:pointer}.composer-agent-picker .agent-picker:hover,.composer-agent-picker .agent-picker.active{border-color:rgba(50,213,176,.25);background:rgba(50,213,176,.09)}.composer-agent-picker .agent-picker svg{width:13px;flex:none}.composer-agent-picker .agent-picker svg:last-child{width:11px}.composer-agent-menu{width:330px}.agent-option,.model-option{border-radius:7px;background:transparent}.agent-option.picked,.model-option.picked{background:rgba(50,213,176,.075)}.agent-option>button,.model-option>button{width:100%;min-height:34px;display:flex;align-items:center;justify-content:space-between;gap:8px;padding:7px 8px;border:0;border-radius:6px;color:#c7d1db;background:transparent;text-align:left}.agent-option>button:hover,.model-option>button:hover{background:rgba(255,255,255,.045)}.agent-option>button span,.model-option>button span{display:flex;min-width:0;flex-direction:column;gap:3px}.agent-option>button b,.model-option>button b{overflow:hidden;font-size:8px;text-overflow:ellipsis;white-space:nowrap}.agent-option>button em,.model-option>button em{color:#697582;font:normal 7px Inter}.agent-option>button svg,.model-option>button svg{width:12px;flex:none;color:#32d5b0}.agent-option>label,.model-option>label{display:grid;grid-template-columns:14px minmax(80px,1fr) 34px;align-items:center;gap:7px;padding:1px 8px 8px;color:#697582}.agent-option>label>svg,.model-option>label>svg{width:12px}.agent-option>label>input,.model-option>label>input{width:100%;min-width:0;accent-color:#32d5b0;cursor:pointer}.agent-option>label>em,.model-option>label>em{color:#8bdac7;font:normal 7px Inter;text-align:right}.model-config-menu{width:360px;max-height:390px}.quick-prompts{min-width:0;display:flex;align-items:center;gap:6px;flex:1;margin-bottom:0}.quick-prompts-toggle{height:25px;display:flex;align-items:center;gap:5px;flex:none;padding:0 7px;border:1px solid rgba(255,255,255,.075);border-radius:6px;color:#75818d;background:rgba(255,255,255,.025);font-size:8px}.quick-prompts-toggle:hover,.quick-prompts.open .quick-prompts-toggle{border-color:rgba(50,213,176,.17);color:#9de3d2;background:rgba(50,213,176,.055)}.quick-prompts-toggle svg{width:12px}.quick-prompts-toggle svg:last-child{width:10px;transition:transform .15s}.quick-prompts.open .quick-prompts-toggle svg:last-child{transform:rotate(180deg)}.quick-prompts-toggle small{min-width:15px;height:15px;display:grid;place-items:center;border-radius:8px;color:#9c8cff;background:rgba(156,140,255,.11);font-size:7px}.quick-prompts .prompts{min-width:0;flex:1;margin:0;overflow-x:auto}.quick-prompts .prompts button{height:25px;white-space:nowrap}
.composer :deep(button.model.review.warn){color:#f3a75c}
.composer :deep(button.model.active){color:#e8edf2}
.composer .speech-input{position:relative}.composer .speech-input:hover{color:#9de3d2;background:rgba(50,213,176,.06)}.composer .speech-input.recording{color:#ff8f95;background:rgba(255,92,103,.1)}.composer .speech-input.recording:after{position:absolute;right:3px;bottom:3px;width:5px;height:5px;border-radius:50%;background:#ff5c67;box-shadow:0 0 7px rgba(255,92,103,.65);content:'';animation:pulse 1.1s infinite}.composer .speech-input.transcribing{color:#9c8cff}.composer .speech-input:disabled{opacity:.72;cursor:wait}
.safe.warn{color:#f3a75c}
.safe.warn :deep(svg){color:#f3a75c}
.files-view>section>footer nav{display:flex;align-items:center;gap:6px}.files-view>section>footer nav button{height:27px;display:flex;align-items:center;gap:5px;padding:0 9px;border:1px solid rgba(255,255,255,.08);border-radius:6px;color:#8bdac7;background:rgba(50,213,176,.055);font-size:8px}.files-view>section>footer nav button:last-child{border:0;color:#06251e;background:#32d5b0}.files-view>section>footer nav button:disabled{opacity:.45}.files-view>section>footer nav svg{width:12px}.file-context-menu{position:fixed;z-index:120;width:178px;padding:5px;border:1px solid rgba(255,255,255,.1);border-radius:9px;background:#151b22;box-shadow:0 18px 45px rgba(0,0,0,.58)}.file-context-menu button{width:100%;height:31px;display:flex;align-items:center;gap:7px;padding:0 8px;border:0;border-radius:5px;color:#a0abb6;background:transparent;font-size:8px;text-align:left}.file-context-menu button:hover{color:#e1e7ec;background:rgba(255,255,255,.055)}.file-context-menu button.danger{color:#df7d83}.file-context-menu button svg{width:13px}.file-context-menu i{display:block;height:1px;margin:4px;background:rgba(255,255,255,.065)}.file-action-path{overflow:hidden;padding:7px 9px;border-radius:6px;color:#65717d;background:#0d1217;font:8px 'Cascadia Code',monospace;text-overflow:ellipsis;white-space:nowrap}
.fallback-tag{padding:2px 5px;border-radius:4px;color:#8f84d8;background:rgba(156,140,255,.1);font:normal 6px Inter}
.stream-cursor{display:inline-block;width:6px;height:11px;margin-left:2px;vertical-align:-1px;background:#32d5b0;animation:blink 1s steps(2) infinite}
.sidebar :deep(.conversation-tree){flex:1}.sidebar>.spacer{display:none}
.wizard-steps{grid-template-columns:repeat(4,1fr)}
.location-page{grid-template-columns:1fr}.location-preview{display:flex;align-items:center;gap:11px;padding:13px;border:1px solid rgba(156,140,255,.16);border-radius:9px;color:#9c8cff;background:rgba(156,140,255,.055)}.location-preview>svg{width:18px}.location-preview p{display:flex;flex-direction:column;margin:0}.location-preview b{color:#d6d0ff;font-size:10px}.location-preview small{margin-top:4px;color:#6e7985;font-size:8px}.creation-mode{display:grid;grid-template-columns:1fr 1fr;gap:10px}.creation-mode article{display:grid;grid-template-columns:34px 1fr;gap:10px;padding:13px;border:1px solid rgba(255,255,255,.075);border-radius:10px;background:#0e1318}.creation-mode article.smart{border-color:rgba(50,213,176,.16);background:linear-gradient(135deg,rgba(50,213,176,.06),#0e1318 62%)}.creation-mode article>span{width:34px;height:34px;display:grid;place-items:center;border-radius:8px;color:#7d8995;background:#171d24}.creation-mode article.smart>span{color:#32d5b0;background:rgba(50,213,176,.09)}.creation-mode article>span svg{width:17px}.creation-mode p{display:flex;flex-direction:column;margin:0}.creation-mode b{font-size:10px}.creation-mode small{margin-top:5px;color:#687481;font-size:8px;line-height:1.55}.creation-mode button{grid-column:1/-1;height:31px;display:flex;align-items:center;justify-content:center;gap:5px;border:1px solid rgba(255,255,255,.08);border-radius:7px;color:#9aa5b0;background:#171d24;font-size:8px}.creation-mode article.smart button{border:0;color:#06251e;background:#32d5b0;font-weight:700}.creation-mode button:disabled{opacity:.38}.creation-mode button svg{width:13px}
.planning-mission{border-color:rgba(156,140,255,.17);background:linear-gradient(120deg,rgba(156,140,255,.09),rgba(50,213,176,.035))}.planning-mission .agent{color:#b4a9ff;background:rgba(156,140,255,.12)}.empty-mission{border-style:dashed}.empty-mission footer button{height:26px;display:flex;align-items:center;gap:5px;padding:0 9px;border:1px solid rgba(50,213,176,.18);border-radius:6px;color:#83ddc7;background:rgba(50,213,176,.07);font-size:8px}.empty-mission footer button svg{width:12px}
.planning-workspace{min-height:300px;display:flex;align-items:center;justify-content:center;flex-direction:column;padding:40px;border:1px solid rgba(156,140,255,.15);border-radius:12px;background:radial-gradient(circle at 50% 0,rgba(156,140,255,.1),transparent 52%),#11161c;text-align:center}.planning-workspace>span{width:54px;height:54px;display:grid;place-items:center;border-radius:15px;color:#b1a5ff;background:rgba(156,140,255,.1);box-shadow:0 0 30px rgba(156,140,255,.1)}.planning-workspace>span svg{width:26px}.planning-workspace>small{margin-top:17px;color:#746ba8;font-size:7px;font-weight:700;letter-spacing:.15em}.planning-workspace h2{margin:8px 0 0;font-size:16px}.planning-workspace p{max-width:420px;margin:10px 0 0;color:#6e7985;font-size:9px;line-height:1.75}.planning-actions{display:flex;align-items:center;gap:8px;margin-top:18px}.planning-workspace .planning-actions button{height:33px;display:flex;align-items:center;gap:6px;padding:0 13px;border:0;border-radius:7px;color:#09241e;background:#32d5b0;font-size:8px;font-weight:700}.planning-workspace .planning-actions button:disabled{opacity:.45;cursor:not-allowed}.planning-workspace .planning-actions button.discard{border:1px solid rgba(226,92,101,.2);color:#df858b;background:rgba(226,92,101,.06)}.planning-workspace button svg{width:14px}.work-header nav button:disabled{opacity:.3;cursor:not-allowed}
.connection-state{min-height:220px;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:9px;padding:28px;border:1px dashed rgba(255,255,255,.1);border-radius:11px;color:#74808c;text-align:center}.connection-state>svg{width:28px;color:#7c8996}.connection-state b{color:#b9c2cb;font-size:13px}.connection-state small{max-width:360px;font-size:9px;line-height:1.7}.connection-state button,.workspace-state button,.surface-state button{height:32px;display:flex;align-items:center;gap:6px;margin-top:5px;padding:0 12px;border:1px solid rgba(50,213,176,.2);border-radius:7px;color:#83dfca;background:rgba(50,213,176,.08);font-size:9px}.connection-state button svg,.workspace-state button svg,.surface-state button svg{width:14px}.connection-state.error,.workspace-state.error{border-color:rgba(226,92,101,.18);background:rgba(226,92,101,.025)}.connection-state.error>svg,.workspace-state.error>svg{color:#df737a}.connection-state.first-run,.workspace-state.first-run{border-color:rgba(50,213,176,.16);background:radial-gradient(circle at 50% 0,rgba(50,213,176,.07),transparent 55%)}
.workspace-state{min-height:0;display:flex;align-items:center;justify-content:center;flex-direction:column;text-align:center}.workspace-state>svg{width:42px;color:#71808c}.workspace-state>small{margin-top:12px;color:#596774;font-size:8px;font-weight:700;letter-spacing:.15em}.workspace-state h2{margin:8px 0 0;font-size:16px}.workspace-state p{max-width:440px;margin:9px 0 0;color:#6c7884;font-size:9px;line-height:1.7}.surface-state{flex:1;padding:32px}.honest-empty{display:flex;align-items:center;gap:11px;color:#65717d}.honest-empty>svg{width:20px}.honest-empty p{display:flex;flex-direction:column;margin:0}.honest-empty b{color:#9da7b1;font-size:10px}.honest-empty small{margin-top:4px;font-size:8px}.terminal-empty{height:100%;display:grid;place-items:center;color:#596572;font:9px Inter,sans-serif}
.server-blocker{display:flex;align-items:center;gap:10px;margin-top:10px;padding:11px 13px;border:1px solid rgba(243,167,92,.18);border-radius:9px;color:#d7a36d;background:rgba(243,167,92,.055)}.server-blocker.error{border-color:rgba(226,92,101,.2);color:#df7d83;background:rgba(226,92,101,.055)}.server-blocker>svg{width:16px;flex:none}.server-blocker p{display:flex;min-width:0;flex:1;flex-direction:column;margin:0}.server-blocker b{font-size:9px}.server-blocker small{margin-top:4px;font-size:8px;line-height:1.5}.server-blocker button{height:28px;display:flex;align-items:center;gap:5px;padding:0 9px;border:1px solid currentColor;border-radius:6px;color:inherit;background:transparent;font-size:8px}.server-blocker button svg{width:12px}.server-hero p em.provisioning,.server-hero p em.starting,.server-hero p em.stopping{color:#f2b579;background:rgba(243,167,92,.1)}.provision-card.cancelled,.provision-card.interrupted,.provision-card.rollback_failed{border-color:rgba(255,107,114,.2)}.provision-card>footer>span{display:flex;align-items:center;gap:6px}.provision-events{display:grid;gap:5px;margin-top:10px;padding-top:9px;border-top:1px solid rgba(255,255,255,.05)}.provision-events p{display:flex;align-items:flex-start;gap:6px;margin:0;color:#71808c;font-size:8px;line-height:1.5}.provision-events i{width:5px;height:5px;flex:none;margin-top:4px;border-radius:50%;background:#4b8f80}.provision-events p.warn i{background:#d99b59}.provision-events p.error{color:#d98388}.provision-events p.error i{background:#d95f67}.provision-error,.provision-incomplete{display:flex;align-items:flex-start;gap:7px;margin:10px 0 0;padding:9px;border-radius:7px;color:#e18a8f;background:rgba(226,92,101,.06);font-size:8px;line-height:1.55}.provision-incomplete{color:#d9a56d;background:rgba(243,167,92,.06)}.provision-error svg,.provision-incomplete svg{width:13px;flex:none}.terminal-view input:disabled{opacity:.55;cursor:not-allowed}.terminal-view form button:disabled{cursor:not-allowed}
.lifecycle-card{margin-top:10px;padding:15px 16px;background:linear-gradient(135deg,rgba(50,213,176,.055),rgba(156,140,255,.035)),#11171d}.lifecycle-card>header{display:flex;align-items:center;justify-content:space-between}.lifecycle-card>header p{display:flex;flex-direction:column;margin:0}.lifecycle-card>header small{color:#6f7d89;font-size:10px}.lifecycle-card>header b{margin-top:4px;font-size:13px}.lifecycle-card>header nav{display:flex;align-items:center;gap:7px}.lifecycle-card>header nav button,.service-settings-card>header button{height:31px;display:flex;align-items:center;gap:6px;padding:0 10px;border:1px solid rgba(50,213,176,.18);border-radius:7px;color:#91dfcc;background:rgba(50,213,176,.06);font-size:10px}.lifecycle-card>header nav button:disabled{opacity:.45}.lifecycle-card>header nav button svg,.service-settings-card>header button svg{width:14px}.lifecycle-track{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin-top:15px}.lifecycle-track>span{position:relative;display:flex;align-items:center;gap:8px;padding:10px;border:1px solid rgba(255,255,255,.065);border-radius:8px;color:#697682;background:rgba(255,255,255,.018)}.lifecycle-track>span:after{position:absolute;right:-9px;width:9px;height:1px;background:rgba(255,255,255,.09);content:''}.lifecycle-track>span:last-child:after{display:none}.lifecycle-track>span>i{width:20px;height:20px;display:grid;place-items:center;border:1px solid rgba(255,255,255,.12);border-radius:50%}.lifecycle-track>span>i em{width:6px;height:6px;border-radius:50%;background:#5b6671}.lifecycle-track>span>i svg{width:11px}.lifecycle-track>span b{font-size:11px}.lifecycle-track>span.active{border-color:rgba(50,213,176,.24);color:#c8f4e9;background:rgba(50,213,176,.07)}.lifecycle-track>span.active>i{border-color:#32d5b0}.lifecycle-track>span.active>i em{background:#32d5b0;box-shadow:0 0 8px rgba(50,213,176,.55)}.lifecycle-track>span.done{color:#85cdbc}.lifecycle-track>span.done>i{color:#32d5b0;border-color:rgba(50,213,176,.26);background:rgba(50,213,176,.08)}.lifecycle-detail{margin:12px 0 0;color:#b8c3cc;font-size:11px}.lifecycle-card>small{display:block;margin-top:6px;color:#6e7a86;font-size:10px;line-height:1.55}
.service-settings-card{margin-top:10px;padding:16px;border-color:rgba(50,213,176,.16)}.service-settings-card>header{display:flex;align-items:center;justify-content:space-between}.service-settings-card>header p{display:flex;flex-direction:column;margin:0}.service-settings-card>header small{color:#6d7985;font-size:10px}.service-settings-card>header b{margin-top:4px;font-size:14px}.service-settings-card>header button{width:31px;padding:0;justify-content:center;border-color:rgba(255,255,255,.08);color:#73808c;background:transparent}.service-settings-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;margin-top:14px}.service-settings-grid>label{display:flex;align-items:flex-start;gap:9px;padding:11px;border:1px solid rgba(255,255,255,.07);border-radius:8px;background:#0e1419;cursor:pointer}.service-settings-grid input,.social-service-settings input{accent-color:#32d5b0}.service-settings-grid>label>input{margin-top:3px}.service-settings-grid span{display:flex;min-width:0;flex-direction:column}.service-settings-grid b{font-size:11px}.service-settings-grid small{margin-top:5px;color:#6e7a86;font-size:10px;line-height:1.45}.social-service-settings{margin-top:10px;padding:13px;border:1px solid rgba(156,140,255,.14);border-radius:9px;background:rgba(156,140,255,.035)}.service-channel-row{display:flex;align-items:center;gap:14px;color:#87939e;font-size:10px}.service-channel-row>b{margin-right:auto;color:#b9c4cd;font-size:11px}.service-channel-row label{display:flex;align-items:center;gap:5px}.service-frequency-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin-top:12px}.service-frequency-grid label{display:grid;grid-template-columns:1fr 68px auto;align-items:center;gap:6px;color:#788591;font-size:10px}.service-frequency-grid input{width:100%;height:30px;padding:0 7px;border:1px solid rgba(255,255,255,.08);border-radius:6px;outline:0;color:#dbe4ea;background:#0d1217}.service-frequency-grid input:focus{border-color:rgba(50,213,176,.35)}.service-frequency-grid em{font:normal 9px Inter}.social-service-settings>p{margin:10px 0 0;color:#737f8b;font-size:10px;line-height:1.55}.service-settings-card>footer{display:flex;justify-content:flex-end;gap:8px;margin-top:14px;padding-top:12px;border-top:1px solid rgba(255,255,255,.06)}.service-settings-card>footer button{height:32px;display:flex;align-items:center;gap:5px;padding:0 12px;border:1px solid rgba(255,255,255,.08);border-radius:7px;color:#87939e;background:#141a20;font-size:10px}.service-settings-card>footer button.primary{border:0;color:#06251e;background:#32d5b0;font-weight:700}.service-settings-card>footer button:disabled{opacity:.45}.service-settings-card>footer svg{width:13px}
.field-error,.field-hint{font-size:8px;line-height:1.45}.field-error{color:#e27c82}.field-hint{color:#64717e}.create-modal{max-height:calc(100vh - 40px);display:flex;flex-direction:column}.create-modal>.wizard-page{overflow-y:auto}.environment-loading{display:flex;align-items:center;gap:8px;padding:10px 12px;border:1px solid rgba(156,140,255,.14);border-radius:8px;color:#9f95de;background:rgba(156,140,255,.045);font-size:9px}.environment-loading>svg{width:16px}.environment-loading.error{border-color:rgba(226,92,101,.18);color:#df7c82;background:rgba(226,92,101,.04)}.environment-loading.error span{display:flex;flex:1;flex-direction:column}.environment-loading.error b{font-size:9px}.environment-loading.error small{margin-top:3px;color:#8a666a;font-size:8px}.environment-loading button{height:26px;padding:0 9px;border:1px solid rgba(226,92,101,.2);border-radius:6px;color:#e59a9e;background:rgba(226,92,101,.06);font-size:8px}
.environment-card.error{border-color:rgba(226,92,101,.2);background:rgba(226,92,101,.045)}.environment-card.error>span{color:#e47178;background:rgba(226,92,101,.09)}.environment-card.error em{color:#e78389}.environment-card.unknown{border-color:rgba(255,255,255,.08);background:rgba(255,255,255,.02)}.environment-card.unknown>span{color:#707c88;background:rgba(255,255,255,.045)}.environment-card.unknown em{color:#697581}.environment-card.info:not(.unknown){border-color:rgba(156,140,255,.14);background:rgba(156,140,255,.04)}.environment-card.info:not(.unknown)>span{color:#9c8cff;background:rgba(156,140,255,.08)}.environment-card.info:not(.unknown) em{color:#9c91dc}.environment-actions{display:flex;align-items:flex-end;flex-direction:column;gap:7px}.environment-actions button{height:27px;display:flex;align-items:center;gap:5px;padding:0 9px;border:1px solid rgba(243,167,92,.2);border-radius:6px;color:#f0b477;background:rgba(243,167,92,.07);font-size:8px}.environment-actions button:disabled{opacity:.55;cursor:wait}.environment-actions button svg{width:12px}.install-result{padding:9px 12px;border-radius:7px;font-size:8px}.install-result.success{color:#71ceb6;background:rgba(50,213,176,.065)}.install-result.error{color:#e78389;background:rgba(226,92,101,.065)}.environment-summary.blocked>svg{color:#d87980}.environment-summary.blocked b{color:#d99ca0}.environment-summary.unknown>svg{color:#77838f}.environment-summary.unknown b{color:#9aa5b0}
.action-modal{width:min(440px,calc(100vw - 40px));overflow:hidden;border:1px solid rgba(255,255,255,.1);border-radius:13px;background:#12171d;box-shadow:0 28px 80px rgba(0,0,0,.55)}.action-modal>header{height:66px;display:flex;align-items:center;justify-content:space-between;padding:0 20px;border-bottom:1px solid rgba(255,255,255,.07)}.action-modal>header small{color:#5f6b77;font-size:7px;font-weight:700;letter-spacing:.13em}.action-modal>header h2{margin:5px 0 0;font-size:15px}.action-modal>header>button{width:28px;height:28px;display:grid;place-items:center;border:0;border-radius:6px;color:#77838f;background:transparent}.action-modal>header svg{width:15px}.action-modal>main{display:flex;flex-direction:column;gap:13px;padding:20px}.action-modal>footer{height:58px;display:flex;align-items:center;justify-content:flex-end;gap:8px;padding:0 20px;border-top:1px solid rgba(255,255,255,.07);background:#10151b}.action-modal>footer button{height:32px;padding:0 12px;border-radius:7px;font-size:8px;font-weight:650}.action-modal .back{border:1px solid rgba(255,255,255,.08);color:#84909b;background:#161c23}.action-modal .next{border:0;color:#07251e;background:#32d5b0}.action-modal .next.danger{color:#fff;background:#c9575e}.action-modal button:disabled{opacity:.38}.danger-copy{display:flex;align-items:flex-start;gap:12px;padding:13px;border:1px solid rgba(226,92,101,.16);border-radius:9px;color:#e37178;background:rgba(226,92,101,.055)}.danger-copy>svg{width:19px;flex:none}.danger-copy p{display:flex;flex-direction:column;margin:0}.danger-copy b{color:#e6b7ba;font-size:10px}.danger-copy small{margin-top:5px;color:#7f6c70;font-size:8px;line-height:1.6}.danger-copy code{color:#ffb3b7}.danger-copy.critical{border-color:rgba(226,92,101,.28);background:rgba(226,92,101,.08)}.delete-files-check{display:flex;align-items:flex-start;gap:9px;padding:12px;border-radius:9px;background:#0e1318;color:#7c8793}.delete-files-check input{margin-top:2px;accent-color:#d75b63}.delete-files-check span{display:flex;flex-direction:column}.delete-files-check b{font-size:9px}.delete-files-check small{margin-top:4px;color:#606b76;font-size:7px;line-height:1.55}
@media(max-width:700px){.creation-mode,.service-settings-grid,.service-frequency-grid{grid-template-columns:1fr}.wizard-steps{grid-template-columns:repeat(4,1fr)}.server-blocker{align-items:flex-start;flex-wrap:wrap}.server-blocker button{margin-left:26px}.provision-card>footer{align-items:flex-start;flex-direction:column;gap:9px}.provision-card>footer>span{flex-wrap:wrap}.service-channel-row{align-items:flex-start;flex-wrap:wrap}.service-channel-row>b{width:100%}.lifecycle-card>header{align-items:flex-start;flex-direction:column;gap:10px}.lifecycle-card>header nav{width:100%;flex-wrap:wrap}.lifecycle-track{gap:4px}.lifecycle-track>span{justify-content:center;padding:8px 4px}.lifecycle-track>span>i{display:none}}
.workspace-mode-switch,.composer-toolbar,.composer-assist,.composer-menu,.file-context-menu{font-family:var(--menu-font-family,inherit)}
.workspace-mode-switch button{color:var(--menu-font-color,#75818e)}
.workspace-mode-switch button:hover{color:#c8d0d8}.workspace-mode-switch button.active{color:#9ce8d6}
.composer-assist>button,.composer-menu>button,.agent-option>button,.model-option>button,.file-context-menu button{color:var(--menu-font-color,#a0abb6)}
.composer-assist>button.active,.composer-assist>button:hover,.file-context-menu button:hover{color:#e1e7ec}
.file-context-menu button.danger{color:#df7d83}
@keyframes blink{50%{opacity:0}}
</style>
