<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Activity, Archive, Bot, Box, BrainCircuit, ChevronDown, ChevronRight, CircleStop, Cpu, Database, Download, FileCode2, Files, Folder, FolderTree, Gauge, GitBranch, LayoutDashboard, MapPin, MessageSquareText, MoreHorizontal, PanelLeftClose, Play, PlugZap, Plus, Search, Send, Server, Settings, ShieldCheck, Sparkles, SquareTerminal, Trash2, Users, Vote, Wrench, Zap, X, Coffee, HardDrive, Check, LoaderCircle } from 'lucide-vue-next'
import AutomationView from './components/AutomationView.vue'
import CommunityView from './components/CommunityView.vue'
import ConversationTree from './components/ConversationTree.vue'
import IntegrationsView from './components/IntegrationsView.vue'
import MirrorCenterView from './features/mirror/MirrorCenterView.vue'
import SettingsView from './features/settings/SettingsView.vue'
import { API_BASE, apiRequest } from './lib/api'
import { postSse } from './lib/sse'
import { loadUi, uiSettings } from './features/settings/store'
import { REVIEW_MODES } from './features/settings/types'
import type { AiSettingsView, ModelBinding, ReviewMode } from './features/settings/types'
import type { Conversation, ConversationAction, ConversationSummary } from './features/conversations/types'

type Status = 'online' | 'stopped' | 'warning' | 'planning'
type Tab = 'overview' | 'files' | 'terminal'
type Surface = 'control' | 'automation' | 'community' | 'integrations' | 'mirror' | 'settings'
interface ServerItem { id:string; name:string; core:string; version:string; status:Status; players:string; memory:number; memory_gb?:number; cpu:number; port:number; task:string; location?:string }
interface Message { id:string; role:'assistant'|'user'; content:string; time:string; actions?:string[]; streaming?:boolean; fallback?:boolean }
interface SystemInfo { java_installed:boolean; java_version?:string; java_home?:string; os:string; arch:string; data_dir:string; recommended_java:number; cores:string[] }
interface TaskInfo { id:string; server_id:string; title:string; kind:string; status:string; progress:number; created_at:string }
interface MirrorInfo { id:string; name:string; base_url:string; enabled:boolean; priority:number; cores:string[]; region:string }
interface DownloadCandidate { mirror_id:string; mirror_name:string; url:string; priority:number; region:string; supported:boolean }
interface FileEntry { name:string;path:string;kind:'folder'|'file';size:number;modified?:number }
interface CatalogCore { slug:string;name:string;minecraft_versions:string[] }
interface DownloadStatus { task_id:string; phase:string; source:string; received:number; total?:number|null; percent:number; message:string }

const seed: ServerItem[] = [
  { id:'sculk', name:'Sculk 生存服', core:'Paper', version:'1.21.4', status:'online', players:'18 / 60', memory:63, memory_gb:8, cpu:28, port:25565, task:'玩法迭代' },
  { id:'mirror', name:'镜像测试服', core:'Purpur', version:'1.21.4', status:'warning', players:'3 / 12', memory:41, memory_gb:8, cpu:16, port:25566, task:'插件测试' },
  { id:'event', name:'周末活动服', core:'Fabric', version:'1.21.1', status:'stopped', players:'0 / 40', memory:0, memory_gb:8, cpu:0, port:25567, task:'待部署' },
]
const servers = ref(seed), selectedId = ref('sculk'), collapsed = ref(false), tab = ref<Tab>('overview'), activeFile = ref('')
const surface = ref<Surface>('control')
const input = ref(''), command = ref(''), thinking = ref(false), busy = ref(false), notice = ref(''), scroller = ref<HTMLElement|null>(null)
const showCreate = ref(false), createStep = ref(1), creating = ref(false), systemInfo = ref<SystemInfo|null>(null)
const catalogCores = ref<CatalogCore[]>([])
const tasks = ref<TaskInfo[]>([])
const mirrors = ref<MirrorInfo[]>([]), selectedMirrorIds = ref<string[]>([]), previewCandidates = ref<DownloadCandidate[]>([]), mirrorPanel = ref(false)
const createForm = ref({name:'新的生存服',location:'local',core:'Paper',version:'1.21.4',memory_gb:8,port:25568,eula_accepted:false})
const createMinecraftVersions = computed(() => catalogCores.value.find(item=>item.name===createForm.value.core)?.minecraft_versions || ['1.21.4','1.21.1','1.20.6','1.20.4'])
const emptyServer:ServerItem={id:'',name:'未选择服务器',core:'',version:'',status:'stopped',players:'- / -',memory:0,memory_gb:8,cpu:0,port:0,task:'请创建或选择服务器项目',location:'local'}
const server = computed(() => servers.value.find(item => item.id === selectedId.value) ?? servers.value[0] ?? emptyServer)
const serverMemoryLimit = computed(() => server.value.memory_gb ?? 8)
const bootstrapTask = computed(() => tasks.value.find(task => task.server_id===selectedId.value&&task.kind==='bootstrap'))
const serverTransitioning = computed(() => server.value.status==='warning'&&(server.value.task.includes('启动')||server.value.task.includes('停止')))
const conversationsByServer = ref<Record<string,ConversationSummary[]>>({})
const selectedConversationId = ref('')
const selectedConversationByServer = ref<Record<string,string>>({})
const selectedConversation = computed(()=>Object.values(conversationsByServer.value).flat().find(item=>item.id===selectedConversationId.value))
const messages = ref<Message[]>([])
const conversationsLoading = ref(false)
const conversationDialog = ref<{kind:'rename'|'group'|'delete';conversation:ConversationSummary}|null>(null)
const conversationDialogValue = ref('')
const deleteServerTarget = ref<ServerItem|null>(null)
const deleteServerFiles = ref(false)
const deleteServerStep = ref<1|2>(1)
const deleteServerConfirmation = ref('')
const fileEntries=ref<FileEntry[]>([]),currentPath=ref(''),parentPath=ref<string|null>(null),fileContent=ref(''),fileReadonly=ref(false),showNewFolder=ref(false),newFolderName=ref('')
const workflow = [
  ['核心与 Java 环境','Paper 1.21.4 · Java 21','done'],['基础配置优化','视距、性能与安全项','done'],['插件方案生成','28 个插件 · 2 个冲突已消解','done'],['镜像服自动测试','回归测试进行中 · 76%','active'],['玩家灰度测试','等待邀请 12 名测试玩家','todo'],['正式服实装','等待人工批准','todo'],
]
const terminal = ref(['[10:23:41 INFO]: Starting minecraft server version 1.21.4','[10:23:43 INFO]: Running Java 21 (Eclipse Adoptium)','[10:23:46 INFO]: Loading 28 plugins...','[10:23:50 INFO]: Done (8.731s)! For help, type "help"','[10:25:12 AI]: Compatibility scan completed: Vault, LuckPerms, EconomyShopGUI ✓'])
const now = () => new Date().toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'})
const aiSettings = ref<AiSettingsView|null>(null)
const chatModelOverride = ref<ModelBinding|null>(null)
const chatAgentOverride = ref<string|null>(null) // null=跟随全局；'default'=强制内置；agent id=强制该 Agent
const showAgentMenu = ref(false), showModelMenu = ref(false), showReviewMenu = ref(false)
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
const chatModelLabel = computed(()=>{
  const override = chatModelOverride.value
  if(!override) return '自动模型'
  const provider = aiSettings.value?.providers.find(item=>item.id===override.provider_id)
  return (provider?.name ?? '未知提供商')+' / '+override.model_id
})
async function loadAiSettings(){try{aiSettings.value=await apiRequest<AiSettingsView>('/api/ai/settings')}catch{}}
async function persistConversationExecution(modelBinding:ModelBinding|null,agentOverride:string|null){
  const conversationId=await ensureConversation();if(!conversationId)throw new Error('无法创建对话任务')
  const summary=await apiRequest<ConversationSummary>('/api/conversations/'+conversationId+'/execution',{method:'PUT',body:JSON.stringify({model_binding:modelBinding,agent_override:agentOverride})})
  const serverItems=conversationsByServer.value[summary.server_id]??[]
  conversationsByServer.value={...conversationsByServer.value,[summary.server_id]:serverItems.map(item=>item.id===summary.id?summary:item)}
}
async function pickChatModel(binding:ModelBinding|null){
  const previous=chatModelOverride.value;showModelMenu.value=false
  try{chatModelOverride.value=binding;await persistConversationExecution(binding,chatAgentOverride.value);chatModelOverride.value=binding;flash(binding?'当前对话将使用 '+chatModelLabel.value:'当前对话已恢复按情景绑定自动选择模型')}
  catch(error){chatModelOverride.value=previous;flash('模型选择保存失败：'+String(error))}
}
async function pickChatAgent(agentId:string|null){
  const previous=chatAgentOverride.value
  const nextAgent=agentId===null?(aiSettings.value?.active_agent?'default':null):agentId
  showAgentMenu.value=false
  try{chatAgentOverride.value=nextAgent;await persistConversationExecution(chatModelOverride.value,nextAgent);chatAgentOverride.value=nextAgent;flash(agentId?'当前对话将通过 ACP 交给 '+chatAgentLabel.value:'当前对话使用内置 Sculk Agent')}
  catch(error){chatAgentOverride.value=previous;flash('Agent 选择保存失败：'+String(error))}
}
async function pickReviewMode(mode:ReviewMode){
  showReviewMenu.value=false
  if(mode===reviewMode.value)return
  try{aiSettings.value=await apiRequest<AiSettingsView>('/api/ai/review-mode',{method:'PUT',body:JSON.stringify({mode})});flashSafeNotice(safeHint.value??'')}
  catch(error){flash('切换失败：'+String(error))}
}
function closeMenus(event:MouseEvent){
  if(!(event.target as HTMLElement).closest('.composer-menu-anchor')){showAgentMenu.value=false;showModelMenu.value=false;showReviewMenu.value=false}
}
function scrollChat(smooth=true){scroller.value?.scrollTo({top:scroller.value.scrollHeight,behavior:smooth?'smooth':'auto'})}
let deltaScrollTimer:number|undefined
function throttledScroll(){if(deltaScrollTimer)return;deltaScrollTimer=window.setTimeout(()=>{deltaScrollTimer=undefined;scrollChat(false)},80)}
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
async function selectConversation(summary:ConversationSummary){
  if(summary.server_id!==selectedId.value){selectedId.value=summary.server_id;surface.value='control';tab.value='overview'}
  try{
    const conversation=await apiRequest<Conversation>('/api/conversations/'+summary.id)
    selectedConversationId.value=conversation.id
    selectedConversationByServer.value={...selectedConversationByServer.value,[conversation.server_id]:conversation.id}
    chatModelOverride.value=conversation.model_binding??null
    chatAgentOverride.value=conversation.agent_override??null
    messages.value=conversation.messages.map(message=>({...message}))
    if(conversation.unread){
      await apiRequest('/api/conversations/'+conversation.id,{method:'PUT',body:JSON.stringify({unread:false})})
      await loadConversationSummaries(conversation.server_id,false)
    }
    await nextTick();scrollChat(false)
  }catch(error){flash('对话打开失败：'+String(error))}
}
async function createConversation(serverId=selectedId.value,title='新对话'){
  if(!serverId)return null
  try{
    const conversation=await apiRequest<Conversation>('/api/servers/'+serverId+'/conversations',{method:'POST',body:JSON.stringify({title})})
    const items=conversationsByServer.value[serverId]??[]
    conversationsByServer.value={...conversationsByServer.value,[serverId]:[toConversationSummary(conversation),...items]}
    await selectConversation(toConversationSummary(conversation))
    input.value=''
    return conversation
  }catch(error){flash('新建对话失败：'+String(error));return null}
}
async function ensureConversation(){
  if(selectedConversationId.value)return selectedConversationId.value
  const conversation=await createConversation(selectedId.value)
  return conversation?.id??''
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
      if(summary.id===selectedConversationId.value&& !summary.archived){selectedConversationId.value='';messages.value=[]}
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
      await apiRequest('/api/conversations/'+dialog.conversation.id,{method:'DELETE'})
      if(selectedConversationId.value===dialog.conversation.id){selectedConversationId.value='';messages.value=[]}
    }else if(dialog.kind==='rename'){
      const title=conversationDialogValue.value.trim();if(!title)return
      await updateConversation(dialog.conversation,{title})
    }else await updateConversation(dialog.conversation,{group:conversationDialogValue.value.trim()})
    conversationDialog.value=null
    await loadConversationSummaries(dialog.conversation.server_id,false)
  }catch(error){flash('对话操作失败：'+String(error))}
}
async function send(preset?:string) {
  const content=(preset??input.value).trim(); if(!content||thinking.value)return
  if(!selectedId.value){flash('请先创建或选择服务器项目');return}
  const conversationId=await ensureConversation();if(!conversationId)return
  messages.value.push({id:crypto.randomUUID(),role:'user',content,time:now()}); input.value=''; thinking.value=true
  await nextTick(); scrollChat()
  const history=messages.value.slice(-21,-1).map(message=>({role:message.role,content:message.content}))
  const reply:Message={id:crypto.randomUUID(),role:'assistant',content:'',time:now(),streaming:true}
  let pushed=false
  try {
    await postSse('/api/chat/stream',{server_id:selectedId.value,conversation_id:conversationId,message:content,history,model_override:chatModelOverride.value,agent_override:chatAgentOverride.value},{
      onMeta:meta=>{thinking.value=false;reply.fallback=meta.fallback;if(!pushed){pushed=true;messages.value.push(reply)}},
      onDelta:text=>{if(!pushed){pushed=true;thinking.value=false;messages.value.push(reply)}reply.content+=text;throttledScroll()},
      onError:message=>{flash('模型响应中断：'+message)},
      onDone:done=>{reply.time=done.time;reply.actions=done.actions;reply.streaming=false;loadConversationSummaries(selectedId.value,false);if(done.task)loadDashboard(false)},
    })
    if(!pushed)throw new Error('empty stream')
    reply.streaming=false
  }
  catch { if(!pushed)messages.value.push({id:crypto.randomUUID(),role:'assistant',time:now(),content:'已理解你的目标："'+content+'"。我会先生成可审阅的执行计划，在涉及停服、修改玩家数据或正式部署前请求确认。',actions:['审阅执行计划','允许在镜像服执行']}); else reply.streaming=false }
  thinking.value=false; await nextTick(); scrollChat()
}
async function api(url:string, options?:RequestInit){return apiRequest<any>(url,options)}
const remoteConnections=computed(()=>(uiSettings.value?.connections??[]).filter(connection=>connection.enabled))
const selectedLocationLabel=computed(()=>createForm.value.location==='local'?(systemInfo.value?.data_dir||'backend/data/servers'):'远程服务器（暂未支持）')
async function openCreate(){showCreate.value=true;createStep.value=1;const [systemResult,catalogResult]=await Promise.allSettled([api('/api/system'),api('/api/catalog/cores'),loadUi()]);systemInfo.value=systemResult.status==='fulfilled'?systemResult.value:null;catalogCores.value=catalogResult.status==='fulfilled'?catalogResult.value:[]}
async function createNewServer(){
  if(creating.value)return;creating.value=true
  try{
    const data=await api('/api/servers',{method:'POST',body:JSON.stringify(createForm.value)})
    servers.value.push(data.server);selectedId.value=data.server.id;showCreate.value=false;surface.value='control';tab.value='files'
    await createConversation(data.server.id,'服务器初始化')
    await loadFiles();await openFile('server.properties');flash('服务器工作区已创建，首次启动任务已加入队列')
  }catch(error){flash('创建失败：'+String(error))}finally{creating.value=false}
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
function openDeleteServer(item:ServerItem){deleteServerTarget.value=item;deleteServerFiles.value=false;deleteServerStep.value=1;deleteServerConfirmation.value=''}
async function advanceDeleteServer(){if(!deleteServerTarget.value)return;if(deleteServerFiles.value){deleteServerStep.value=2;return}await executeDeleteServer()}
async function executeDeleteServer(){
  const target=deleteServerTarget.value;if(!target)return
  if(deleteServerFiles.value&&deleteServerConfirmation.value!=='delete all')return
  busy.value=true
  try{
    await api('/api/servers/'+target.id,{method:'DELETE',body:JSON.stringify({delete_files:deleteServerFiles.value,confirmation:deleteServerFiles.value?deleteServerConfirmation.value:null})})
    servers.value=servers.value.filter(item=>item.id!==target.id)
    const next={...conversationsByServer.value};delete next[target.id];conversationsByServer.value=next
    deleteServerTarget.value=null
    if(selectedId.value===target.id){selectedId.value=servers.value[0]?.id??'';selectedConversationId.value='';messages.value=[];if(selectedId.value)await selectServer(selectedId.value)}
    flash(deleteServerFiles.value?'服务器与磁盘文件已删除':'服务器已从项目列表移除，磁盘文件已保留')
  }catch(error){flash('删除服务器失败：'+String(error))}finally{busy.value=false}
}

function flash(message:string){notice.value=message;window.setTimeout(()=>{if(notice.value===message)notice.value=''},2600)}
async function loadLogs(){try{const data=await api('/api/servers/'+selectedId.value+'/logs');terminal.value=data.lines}catch{}}
const logSocket=ref<WebSocket|null>(null),logStreamLive=ref(false),termScroller=ref<HTMLElement|null>(null)
function scrollTerminal(){nextTick(()=>{termScroller.value?.scrollTo({top:termScroller.value.scrollHeight})})}
function disconnectLogStream(){logStreamLive.value=false;if(logSocket.value){logSocket.value.onclose=null;logSocket.value.close();logSocket.value=null}}
function connectLogStream(){
  disconnectLogStream()
  const id=selectedId.value
  try{
    const socket=new WebSocket(API_BASE.replace(/^http/,'ws')+'/api/servers/'+id+'/ws/logs')
    socket.onopen=()=>{if(selectedId.value!==id){socket.close();return}terminal.value=[];logStreamLive.value=true}
    socket.onmessage=event=>{terminal.value.push(String(event.data));if(terminal.value.length>1200)terminal.value.splice(0,terminal.value.length-1200);scrollTerminal()}
    socket.onclose=()=>{if(logSocket.value===socket){logStreamLive.value=false;logSocket.value=null;loadLogs()}}
    socket.onerror=()=>{socket.close()}
    logSocket.value=socket
  }catch{logStreamLive.value=false;loadLogs()}
}
async function loadFiles(path=''){try{const data=await api('/api/servers/'+selectedId.value+'/files?path='+encodeURIComponent(path));fileEntries.value=data.entries;currentPath.value=data.path;parentPath.value=data.parent}catch(error){flash('目录读取失败：'+String(error))}}
async function openFile(path:string){try{const data=await api('/api/servers/'+selectedId.value+'/file?path='+encodeURIComponent(path));activeFile.value=data.path;fileContent.value=data.content;fileReadonly.value=data.readonly}catch(error){flash('文件读取失败：'+String(error))}}
async function openEntry(entry:FileEntry){if(entry.kind==='folder'){activeFile.value='';fileContent.value='';await loadFiles(entry.path)}else await openFile(entry.path)}
async function saveCurrentFile(){if(!activeFile.value||fileReadonly.value)return;busy.value=true;try{await api('/api/servers/'+selectedId.value+'/file',{method:'PUT',body:JSON.stringify({path:activeFile.value,content:fileContent.value})});flash(activeFile.value+' 已保存')}catch(error){flash('保存失败：'+String(error))}finally{busy.value=false}}
async function createFolder(){const name=newFolderName.value.trim();if(!name)return;const path=[currentPath.value,name].filter(Boolean).join('/');try{await api('/api/servers/'+selectedId.value+'/directory',{method:'POST',body:JSON.stringify({path})});newFolderName.value='';showNewFolder.value=false;await loadFiles(currentPath.value);flash('目录已创建')}catch(error){flash('创建失败：'+String(error))}}
async function openProperties(){tab.value='files';await loadFiles();await openFile('server.properties')}
async function selectServer(id:string){
  selectedId.value=id;surface.value='control';tab.value='overview';activeFile.value='';mirrorPanel.value=false;previewCandidates.value=[];downloadStatus.value=null;stopDownloadPolling()
  if(!conversationsByServer.value[id])await loadConversationSummaries(id,false)
  const items=conversationsByServer.value[id]??[]
  const remembered=selectedConversationByServer.value[id]
  const preferred=items.find(item=>item.id===remembered&&!item.archived)??items.find(item=>!item.archived)??items[0]
  if(preferred)await selectConversation(preferred);else{selectedConversationId.value='';messages.value=[]}
  if(server.value.status!=='planning')await Promise.all([loadLogs(),fetchDownloadStatus()])
}
async function openMirrorPanel(){mirrorPanel.value=!mirrorPanel.value;if(!mirrorPanel.value)return;try{mirrors.value=await api('/api/download/mirrors');selectedMirrorIds.value=mirrors.value.filter(mirror=>mirror.enabled).map(mirror=>mirror.id);previewCandidates.value=[]}catch(error){flash('镜像列表加载失败：'+String(error))}}
const downloadStatus=ref<DownloadStatus|null>(null)
let downloadTimer:number|undefined
const downloadActive=computed(()=>!!downloadStatus.value&&['resolving','downloading','verifying'].includes(downloadStatus.value.phase))
const downloadPhaseLabels:Record<string,string>={resolving:'解析下载源',downloading:'下载中',verifying:'校验中',completed:'下载完成',failed:'下载失败',cancelled:'已取消'}
const downloadPhaseLabel=computed(()=>downloadStatus.value?(downloadPhaseLabels[downloadStatus.value.phase]??downloadStatus.value.phase):'')
const formatMb=(n:number)=>(n/1048576).toFixed(1)+' MB'
const downloadTraffic=computed(()=>{const s=downloadStatus.value;if(!s)return'';const parts=[formatMb(s.received)+(s.total?' / '+formatMb(s.total):'')];if(s.message)parts.push(s.message);return parts.join(' · ')})
const downloadSummary=computed(()=>{if(downloadStatus.value)return downloadPhaseLabel.value+(downloadStatus.value.source?' · '+downloadStatus.value.source:'');return bootstrapTask.value?'等待下载核心文件':'资源目录优先，镜像与官方源自动回退'})
function stopDownloadPolling(){if(downloadTimer){window.clearInterval(downloadTimer);downloadTimer=undefined}}
function startDownloadPolling(){if(downloadTimer)return;downloadTimer=window.setInterval(async()=>{await fetchDownloadStatus(false);if(!downloadActive.value){stopDownloadPolling();await loadDashboard(false)}},1000)}
async function fetchDownloadStatus(schedule=true){try{const data=await api('/api/servers/'+selectedId.value+'/download/status');downloadStatus.value=data.status??null;if(schedule&&downloadActive.value)startDownloadPolling()}catch{}}
async function startCoreDownload(){if(downloadActive.value)return;busy.value=true;try{await api('/api/servers/'+selectedId.value+'/download/core',{method:'POST',body:JSON.stringify({mirror_ids:selectedMirrorIds.value})});flash('核心下载任务已启动（资源目录优先）');await fetchDownloadStatus()}catch(error){flash('下载启动失败：'+String(error))}finally{busy.value=false}}
async function cancelCoreDownload(){try{await api('/api/servers/'+selectedId.value+'/download/cancel',{method:'POST'});flash('已请求取消下载')}catch(error){flash('取消失败：'+String(error))}}
async function previewDownloads(){if(!selectedMirrorIds.value.length)return;busy.value=true;try{const data=await api('/api/download/preview',{method:'POST',body:JSON.stringify({core:server.value.core,version:server.value.version,mirror_ids:selectedMirrorIds.value})});previewCandidates.value=data.candidates;flash('已生成 '+data.candidates.length+' 个下载候选地址')}catch(error){flash('预览失败：'+String(error))}finally{busy.value=false}}
async function toggleServer(){if(busy.value)return;busy.value=true;try{const action=server.value.status==='online'?'stop':'start';const data=await api('/api/servers/'+selectedId.value+'/action',{method:'POST',body:JSON.stringify({action})});const index=servers.value.findIndex(item=>item.id===selectedId.value);if(index>=0)servers.value[index]=data.server;terminal.value.push(data.log);flash(action==='start'?'服务器已启动':'服务器已安全停止')}catch(error){flash('操作失败：'+String(error))}finally{busy.value=false}}
async function runCommand(){const value=command.value.trim();if(!value||busy.value)return;command.value='';busy.value=true;try{const data=await api('/api/servers/'+selectedId.value+'/command',{method:'POST',body:JSON.stringify({command:value})});if(!logStreamLive.value){terminal.value.push(...data.lines);scrollTerminal()}}catch(error){terminal.value.push('[ERROR]: '+String(error))}finally{busy.value=false}}
async function loadDashboard(loadRelated=true){try{const data=await api('/api/dashboard');servers.value=data.servers;tasks.value=data.tasks;if(!servers.value.some(item=>item.id===selectedId.value))selectedId.value=servers.value[0]?.id??'';if(loadRelated&&selectedId.value&&server.value.status!=='planning')await loadLogs()}catch{flash('Rust 后端未连接，当前使用展示数据')}}
watch(selectedId,()=>{if(tab.value==='files')loadFiles();else if(tab.value==='terminal')connectLogStream();else loadLogs()})
watch(tab,(next,prev)=>{if(next==='terminal')connectLogStream();else if(prev==='terminal')disconnectLogStream()})
watch(surface,(_next,prev)=>{if(prev==='settings')loadAiSettings()})
let refreshTimer:number|undefined
onMounted(async()=>{document.addEventListener('click',closeMenus);loadUi().catch(()=>{});await Promise.all([loadDashboard(),loadAiSettings()]);await loadAllConversationSummaries();if(selectedId.value)await selectServer(selectedId.value);if(server.value.status!=='planning')await fetchDownloadStatus();refreshTimer=window.setInterval(async()=>{if(!showCreate.value&&!creating.value&&!(surface.value==='control'&&tab.value==='files')){await loadDashboard(false);if(tab.value==='terminal'&&!logStreamLive.value&&server.value.status!=='planning')connectLogStream()}},2000)})
onUnmounted(()=>{document.removeEventListener('click',closeMenus);if(refreshTimer)window.clearInterval(refreshTimer);disconnectLogStream();stopDownloadPolling()})
</script>

<template>
  <main class="app" :class="{collapsed,'mirror-mode':surface==='mirror'||surface==='settings'}">
    <aside class="sidebar">
      <div class="brand"><span class="logo"><Box :size="17"/></span><div v-if="!collapsed"><b>Sculk Catalyst</b><small>AI Server Studio</small></div><button @click="collapsed=!collapsed"><PanelLeftClose v-if="!collapsed" :size="16"/><ChevronRight v-else :size="16"/></button></div>
      <button class="create" @click="openCreate"><Plus :size="16"/><span v-if="!collapsed">创建服务器</span></button>
      <nav><button aria-label="控制中心" :class="{active:surface==='control'}" @click="surface='control'"><LayoutDashboard/><span v-if="!collapsed">控制中心</span></button><button aria-label="镜像仓库" title="镜像仓库" :class="{active:surface==='mirror'}" @click="surface='mirror'"><Archive/><span v-if="!collapsed">镜像仓库</span></button><button aria-label="AI 自动化" :class="{active:surface==='automation'}" @click="surface='automation'"><Sparkles/><span v-if="!collapsed">AI 自动化</span><i v-if="!collapsed">{{tasks.filter(task=>task.status==='queued').length}}</i></button><button aria-label="玩家社区" :class="{active:surface==='community'}" @click="surface='community'"><Vote/><span v-if="!collapsed">玩家社区</span></button><button aria-label="Skills & MCP" :class="{active:surface==='integrations'}" @click="surface='integrations'"><PlugZap/><span v-if="!collapsed">Skills & MCP</span></button></nav>
      <div v-if="!collapsed" class="label">服务器 <MoreHorizontal :size="15"/></div>
      <ConversationTree
        :servers="servers"
        :conversations="conversationsByServer"
        :selected-server-id="selectedId"
        :selected-conversation-id="selectedConversationId"
        :collapsed="collapsed"
        @select-server="selectServer"
        @select-conversation="selectConversation"
        @new-conversation="createConversation"
        @conversation-action="handleConversationAction"
        @delete-server="openDeleteServer"
      />
      <div class="spacer"/><div v-if="!collapsed" class="codex"><GitBranch :size="17"/><span><b>Codex 已连接</b><small>MCP 通道可用</small></span><i class="dot online"/></div><button class="settings" :class="{active:surface==='settings'}" @click="surface='settings'"><Settings/><span v-if="!collapsed">设置</span></button>
    </aside>

    <section v-if="surface!=='mirror'&&surface!=='settings'" class="chat-panel">
      <header><div><small>{{server.name}}</small><h1>{{selectedConversation?.title || '新对话任务'}}</h1><em>{{server.task}}</em></div><span><button><Search/></button><button @click="selectedConversation&&handleConversationAction('rename',selectedConversation)"><MoreHorizontal/></button></span></header>
      <div ref="scroller" class="chat-scroll">
        <section v-if="server.status==='planning'" class="mission planning-mission"><div><span class="agent"><BrainCircuit :size="20"/></span><p><small>智能创建 · 规划阶段</small><b>通过对话确定核心、版本与部署方案</b></p><em><i/>未创建文件</em></div><footer><span>先描述玩法、人数与版本偏好</span><span>方案确认后再创建工作区</span></footer></section>
        <section v-else-if="!selectedConversationId" class="mission empty-mission"><div><span class="agent"><MessageSquareText :size="20"/></span><p><small>服务器对话任务</small><b>新建一个独立对话开始工作</b></p></div><footer><span>每个任务拥有独立历史与上下文</span><button @click="createConversation(selectedId)"><Plus/>新建对话</button></footer></section>
        <div v-if="messages.length" class="day">今天</div>
        <article v-for="message in messages" :key="message.id" class="message" :class="message.role">
          <span v-if="message.role==='assistant'" class="avatar bot"><Bot :size="17"/></span><div><header><b>{{message.role==='assistant'?'Sculk Agent':'你'}}</b><time>{{message.time}}</time><em v-if="message.role==='assistant'&&message.fallback" class="fallback-tag">本地规则</em></header><p>{{message.content}}<i v-if="message.streaming" class="stream-cursor"/></p><footer v-if="message.actions"><button v-for="action in message.actions" :key="action" @click="send(action)">{{action}}<ChevronRight :size="13"/></button></footer></div><span v-if="message.role==='user'" class="avatar user">A</span>
        </article>
        <article v-if="thinking" class="message assistant"><span class="avatar bot"><Bot :size="17"/></span><div><header><b>Sculk Agent</b><time>正在分析</time></header><span class="typing"><i/><i/><i/></span></div></article>
      </div>
      <div class="compose-wrap"><div class="prompts"><button @click="send('分析最新报错并自动修复')"><Wrench/>修复报错</button><button @click="send('为新玩法发起玩家投票')"><Vote/>发起投票</button><button @click="send('生成本周服务器宣传文案')"><MessageSquareText/>宣传文案</button></div><div class="composer"><textarea v-model="input" placeholder="描述你想完成的开服任务…" @keydown.enter.exact.prevent="send()"/><footer><span>
        <button><Plus/></button>
        <span class="composer-menu-anchor">
          <button class="model" :class="{active:showAgentMenu}" @click.stop="showAgentMenu=!showAgentMenu;showModelMenu=false;showReviewMenu=false"><Sparkles/>{{chatAgentLabel}}<ChevronDown/></button>
          <div v-if="showAgentMenu" class="composer-menu">
            <small>本次对话的 Agent</small>
            <button :class="{picked:!activeAgentId}" @click="pickChatAgent(null)"><span><b>Sculk Agent（内置）</b><em>直连模型提供商，可搭配右侧模型选择</em></span><Check v-if="!activeAgentId"/></button>
            <template v-if="agentMenuItems.length">
              <small class="group">ACP Agent</small>
              <button v-for="agent in agentMenuItems" :key="agent.id" :class="{picked:activeAgentId===agent.id}" @click="pickChatAgent(agent.id)"><span><b>{{agent.name}}</b><em>{{agent.kind}} · ACP 协议接入</em></span><Check v-if="activeAgentId===agent.id"/></button>
            </template>
            <small v-else class="empty-hint">尚未接入外部 Agent，可到「设置」通过 ACP 协议接入</small>
          </div>
        </span>
        <span v-if="!activeAgentId" class="composer-menu-anchor">
          <button class="model" :class="{active:showModelMenu}" @click.stop="showModelMenu=!showModelMenu;showAgentMenu=false;showReviewMenu=false"><Cpu/>{{chatModelLabel}}<ChevronDown/></button>
          <div v-if="showModelMenu" class="composer-menu">
            <small>本次对话使用的模型</small>
            <button :class="{picked:!chatModelOverride}" @click="pickChatModel(null)"><span><b>自动</b><em>按情景绑定选择模型</em></span><Check v-if="!chatModelOverride"/></button>
            <template v-for="group in modelMenuGroups" :key="group.id">
              <small class="group">{{group.name}}</small>
              <button v-for="model in group.models" :key="group.id+'::'+model.id" :class="{picked:chatModelOverride?.provider_id===group.id&&chatModelOverride?.model_id===model.id}" @click="pickChatModel({provider_id:group.id,model_id:model.id})"><span><b>{{model.id}}</b></span><Check v-if="chatModelOverride?.provider_id===group.id&&chatModelOverride?.model_id===model.id"/></button>
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
      </span><button class="send" :disabled="!input.trim()" @click="send()"><Send/></button></footer></div><p v-if="safeNotice" class="safe" :class="{warn:reviewMode==='full'}"><ShieldCheck/>{{safeNotice}}</p></div>
    </section>

    <section class="work-panel" :class="{'mirror-work':surface==='mirror'||surface==='settings'}">
      <template v-if="surface==='control'">
      <header class="work-header"><nav><button :class="{active:tab==='overview'}" @click="tab='overview'"><Gauge/>总览</button><button :disabled="server.status==='planning'" :class="{active:tab==='files'}" @click="tab='files';loadFiles()"><Files/>文件</button><button :disabled="server.status==='planning'" :class="{active:tab==='terminal'}" @click="tab='terminal'"><SquareTerminal/>终端</button></nav><span v-if="notice" class="notice">{{notice}}</span><button><MoreHorizontal/></button></header>
      <div v-if="tab==='overview'" class="work-scroll">
        <section v-if="server.status==='planning'" class="planning-workspace"><span><BrainCircuit/></span><small>PLANNING WORKSPACE</small><h2>服务器尚在规划阶段</h2><p>这里还没有核心、配置或文件。继续在左侧当前对话中描述玩法、预计人数与版本偏好，Sculk Agent 会先给出可审阅方案。</p><button @click="send('请根据我的需求推荐合适的服务端核心，并说明取舍')"><Sparkles/>开始核心选型</button></section>
        <section v-else class="server-hero"><div><span class="big-icon"><Server/></span><p><b>{{server.name}} <em :class="server.status">{{server.status==='online'?'运行中':serverTransitioning?'处理中':server.status==='warning'?'需关注':'已停止'}}</em></b><small>{{server.core}} {{server.version}} · Java 21 · 内存 {{serverMemoryLimit}} GB · 端口 {{server.port}}</small></p></div><aside><button :class="{active:mirrorPanel}" @click="openMirrorPanel"><Download/>核心</button><button @click="openProperties"><Settings/>配置</button><button :disabled="busy||serverTransitioning" :class="server.status==='online'?'stop':'start'" @click="toggleServer"><LoaderCircle v-if="serverTransitioning" class="spin"/><CircleStop v-else-if="server.status==='online'"/><Play v-else/>{{serverTransitioning?'处理中':server.status==='online'?'停止服务器':'启动服务器'}}</button></aside></section>
        <section v-if="server.status!=='planning'" class="metrics"><div><span class="cyan"><Users/></span><p><small>在线玩家</small><b>{{server.players}}</b><em>峰值 34</em></p></div><div><span class="purple"><Cpu/></span><p><small>CPU 使用率</small><b>{{server.cpu}}%</b><em>运行平稳</em></p></div><div><span class="green"><Database/></span><p><small>内存</small><b>{{server.memory}}%</b><em>上限 {{serverMemoryLimit}} GB</em></p></div><div><span class="orange"><Activity/></span><p><small>TPS</small><b>19.98</b><em>延迟 31 ms</em></p></div></section>
        <section v-if="server.status!=='planning'&&(bootstrapTask||mirrorPanel||downloadStatus)" class="card bootstrap-card" :class="downloadStatus?(downloadActive?'running':downloadStatus.phase):bootstrapTask?.status">
          <header><p><small>核心下载</small><b>{{server.core}} {{server.version}} · server.jar</b></p><span>{{downloadStatus?downloadStatus.percent+'%':(bootstrapTask?bootstrapTask.progress+'%':'')}}</span></header>
          <div class="bootstrap-progress"><i :style="{width:(downloadStatus?downloadStatus.percent:bootstrapTask?.progress??0)+'%'}"/></div>
          <footer><p>{{downloadSummary}}</p><button :class="{active:mirrorPanel}" @click="openMirrorPanel"><Download/>{{mirrorPanel?'收起回退源':'选择回退源'}}</button></footer>
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
        <section v-if="server.status!=='planning'" class="card workflow"><header><p><small>AI 开服流水线</small><b>从构建到实装</b></p><button>查看全部<ChevronRight/></button></header><div v-for="(step,index) in workflow" :key="step[0]" class="step" :class="step[2]"><span><i>{{step[2]==='done'?'✓':index+1}}</i><u v-if="index<workflow.length-1"/></span><p><b>{{step[0]}}</b><small>{{step[1]}}</small></p><em>{{step[2]==='done'?'已完成':step[2]==='active'?'执行中':'待处理'}}</em></div></section>
        <section v-if="server.status!=='planning'" class="card activity"><header><p><small>实时动态</small><b>AI 运行日志</b></p><MoreHorizontal/></header><div><span class="success"><Zap/></span><p><b>自动修复完成</b><small>已调整 Chunky 参数，内存峰值下降 22%</small></p><time>8 分钟前</time></div><div><span class="info"><GitBranch/></span><p><b>Codex 提交插件构建</b><small>ruins-bounty v0.3.2 已部署至镜像服</small></p><time>21 分钟前</time></div><div><span class="warn"><Users/></span><p><b>玩家意见聚类完成</b><small>从 86 条反馈中整理出 5 个主要诉求</small></p><time>1 小时前</time></div></section>
      </div>
      <div v-else-if="tab==='files'" class="files-view"><aside><header><span>{{currentPath||'服务器文件'}}</span><button @click="showNewFolder=!showNewFolder"><Plus/></button></header><form v-if="showNewFolder" class="new-folder" @submit.prevent="createFolder"><input v-model="newFolderName" placeholder="新目录名称"/><button>创建</button></form><button v-if="parentPath!==null" @click="loadFiles(parentPath||'')"><Folder/><span>..</span></button><button v-for="entry in fileEntries" :key="entry.path" :class="{active:activeFile===entry.path}" @click="openEntry(entry)"><Folder v-if="entry.kind==='folder'"/><FileCode2 v-else/><span>{{entry.name}}</span><small v-if="entry.kind==='file'">{{entry.size<1024?entry.size+' B':Math.ceil(entry.size/1024)+' KB'}}</small></button></aside><section><header><FileCode2/>{{activeFile||'未选择文件'}}</header><textarea v-if="activeFile" v-model="fileContent" class="config-editor" :readonly="fileReadonly" spellcheck="false"/><div v-else class="empty"><Files/><b>{{currentPath||'服务器工作区'}}</b><small>选择文本文件进行安全编辑</small></div><footer><span>UTF-8　LF　{{fileReadonly?'只读文件':'路径保护已开启'}}</span><button v-if="activeFile&&!fileReadonly" :disabled="busy" @click="saveCurrentFile">保存文件</button></footer></section></div>
      <div v-else class="terminal-view"><header><span><i :class="{live:logStreamLive}"/>{{server.name}} / 控制台 · {{logStreamLive?'实时连接':'轮询模式'}}</span><Search/></header><main ref="termScroller"><p v-for="(line,index) in terminal" :key="index" :class="{ai:line.includes('AI]')}">{{line}}</p></main><form @submit.prevent="runCommand"><ChevronRight/><input v-model="command" placeholder="输入服务器命令，例如 list"/><button>执行</button></form></div>
      </template>
      <AutomationView v-else-if="surface==='automation'" :server-id="selectedId"/>
      <CommunityView v-else-if="surface==='community'" :server-id="selectedId"/>
       <IntegrationsView v-else-if="surface==='integrations'"/>
       <SettingsView v-else-if="surface==='settings'"/>
       <MirrorCenterView v-else-if="surface==='mirror'" :initial-core="server.core" :initial-minecraft="server.version"/>
    </section>
    <div v-if="showCreate" class="modal-backdrop" @click.self="showCreate=false">
      <section class="create-modal">
        <header><div><small>NEW SERVER WORKSPACE</small><h2>创建 Minecraft 服务器</h2></div><button @click="showCreate=false"><X/></button></header>
        <nav class="wizard-steps"><span v-for="index in 4" :key="index" :class="{active:createStep===index,done:createStep>index}"><i><Check v-if="createStep>index"/><template v-else>{{index}}</template></i>{{['名称与位置','服务器参数','环境检查','确认创建'][index-1]}}</span></nav>
        <main v-if="createStep===1" class="wizard-page location-page">
          <div class="field wide"><label>服务器项目名称</label><input v-model="createForm.name" placeholder="例如：深暗生存服"/></div>
          <div class="field wide"><label>服务器位置</label><select v-model="createForm.location"><option value="local">本机 · 默认数据目录</option><option v-for="connection in remoteConnections" :key="connection.id" :value="'remote:'+connection.id" disabled>{{connection.name}} · {{connection.host}}（暂未支持）</option><option v-if="!remoteConnections.length" value="remote:placeholder" disabled>远程服务器（接口已预留，暂未支持）</option></select></div>
          <div class="location-preview wide"><MapPin/><p><b>{{selectedLocationLabel}}</b><small>服务器项目会使用独立目录；智能创建阶段不会写入任何文件。</small></p></div>
          <div class="creation-mode wide"><article><span><FolderTree/></span><p><b>普通创建</b><small>继续选择核心、版本、内存和端口，完成环境检查后创建工作区。</small></p><button :disabled="!createForm.name.trim()" @click="createStep=2">继续配置<ChevronRight/></button></article><article class="smart"><span><BrainCircuit/></span><p><b>智能创建</b><small>先把项目加入列表，不预设核心、不创建文件。随后通过独立对话完成选型与部署方案。</small></p><button :disabled="!createForm.name.trim()||creating" @click="createSmartServer"><LoaderCircle v-if="creating" class="spin"/><Sparkles v-else/>进入智能规划</button></article></div>
        </main>
        <main v-else-if="createStep===2" class="wizard-page">
          <div class="field"><label>服务端核心</label><select v-model="createForm.core"><option v-for="core in (catalogCores.length?catalogCores.map(item=>item.name):(systemInfo?.cores || ['Paper','Purpur','Fabric','Velocity']))" :key="core">{{core}}</option></select></div>
          <div class="field"><label>Minecraft 版本</label><select v-model="createForm.version"><option v-for="version in createMinecraftVersions" :key="version">{{version}}</option></select></div>
          <div class="field"><label>最大内存</label><div class="input-unit"><input v-model.number="createForm.memory_gb" type="number" min="2" max="64"/><span>GB</span></div></div>
          <div class="field"><label>服务器端口</label><input v-model.number="createForm.port" type="number" min="1024" max="65535"/></div>
          <div class="core-note wide"><Sparkles/><p><b>参数只用于普通创建</b><small>核心选择不再硬编码推荐。智能创建会在对话中结合玩法、插件生态和维护成本给出选型建议。</small></p></div>
        </main>
        <main v-else-if="createStep===3" class="wizard-page environment-page">
          <div class="environment-card" :class="{ok:systemInfo?.java_installed}"><span><Coffee/></span><p><b>Java 运行环境</b><small>{{systemInfo?.java_installed ? systemInfo.java_version : '未检测到 Java，将创建安装任务'}}</small></p><em>{{systemInfo?.java_installed?'可用':'待安装'}}</em></div>
          <div class="environment-card ok"><span><HardDrive/></span><p><b>服务器工作区</b><small>{{systemInfo?.data_dir || 'backend/data/servers'}}</small></p><em>可写入</em></div>
          <div class="environment-card ok"><span><Cpu/></span><p><b>系统架构</b><small>{{systemInfo?.os || 'Windows'}} · {{systemInfo?.arch || 'x86_64'}}</small></p><em>兼容</em></div>
          <div class="environment-summary"><ShieldCheck/><p><b>隔离与安全策略已启用</b><small>每个服务器使用独立目录。首次启动前不会下载或执行任何未知文件。</small></p></div>
        </main>
        <main v-else class="wizard-page review-page">
          <div class="review-server"><span><Server/></span><p><b>{{createForm.name}}</b><small>{{createForm.core}} {{createForm.version}} · Java {{systemInfo?.recommended_java || 21}}</small></p></div>
          <dl><div><dt>服务器位置</dt><dd>本机默认数据目录</dd></div><div><dt>内存限制</dt><dd>{{createForm.memory_gb}} GB</dd></div><div><dt>监听端口</dt><dd>{{createForm.port}}</dd></div><div><dt>初始状态</dt><dd>停止 · 等待核心下载</dd></div></dl>
          <label class="eula-check"><input v-model="createForm.eula_accepted" type="checkbox"/><span>我已阅读并同意 Minecraft EULA，允许工具生成 <code>eula=true</code></span></label>
        </main>
        <footer><button class="back" :disabled="createStep===1||creating" @click="createStep--">上一步</button><button v-if="createStep===1" class="next" :disabled="!createForm.name.trim()" @click="createStep=2">普通创建<ChevronRight/></button><button v-else-if="createStep<4" class="next" @click="createStep++">继续<ChevronRight/></button><button v-else class="next" :disabled="!createForm.eula_accepted||creating" @click="createNewServer"><LoaderCircle v-if="creating" class="spin"/><Plus v-else/>创建服务器</button></footer>
      </section>
    </div>

    <div v-if="conversationDialog" class="modal-backdrop" @click.self="conversationDialog=null">
      <section class="action-modal">
        <header><div><small>CONVERSATION TASK</small><h2>{{conversationDialog.kind==='rename'?'重命名对话':conversationDialog.kind==='group'?'移动到组':'删除对话'}}</h2></div><button @click="conversationDialog=null"><X/></button></header>
        <main v-if="conversationDialog.kind==='delete'" class="danger-copy"><Trash2/><p><b>删除「{{conversationDialog.conversation.title}}」？</b><small>该对话的全部历史消息会被永久删除，服务器项目本身不受影响。</small></p></main>
        <main v-else><div class="field"><label>{{conversationDialog.kind==='rename'?'新名称':'分组名称'}}</label><input v-model="conversationDialogValue" :placeholder="conversationDialog.kind==='rename'?'输入对话任务名称':'输入分组名称；留空移出分组'" @keydown.enter.prevent="submitConversationDialog"/></div></main>
        <footer><button class="back" @click="conversationDialog=null">取消</button><button class="next" :class="{danger:conversationDialog.kind==='delete'}" :disabled="conversationDialog.kind==='rename'&&!conversationDialogValue.trim()" @click="submitConversationDialog">{{conversationDialog.kind==='delete'?'删除对话':'保存'}}</button></footer>
      </section>
    </div>

    <div v-if="deleteServerTarget" class="modal-backdrop" @click.self="deleteServerTarget=null">
      <section class="action-modal delete-server-modal">
        <header><div><small>DELETE SERVER PROJECT</small><h2>{{deleteServerStep===1?'删除服务器项目':'确认删除磁盘文件'}}</h2></div><button @click="deleteServerTarget=null"><X/></button></header>
        <main v-if="deleteServerStep===1">
          <div class="danger-copy"><Trash2/><p><b>从项目列表删除「{{deleteServerTarget.name}}」？</b><small>关联的对话任务、自动化任务和运行状态会一并移除。</small></p></div>
          <label class="delete-files-check"><input v-model="deleteServerFiles" type="checkbox"/><span><b>同时删除磁盘上的服务器文件</b><small>包括地图、插件、配置和日志。勾选后还需要第二次确认。</small></span></label>
        </main>
        <main v-else>
          <div class="danger-copy critical"><ShieldCheck/><p><b>这是不可恢复的磁盘删除</b><small>将永久删除「{{deleteServerTarget.name}}」的完整服务器目录。请手动输入 <code>delete all</code>。</small></p></div>
          <div class="field"><label>确认文本</label><input v-model="deleteServerConfirmation" autocomplete="off" placeholder="delete all" @keydown.enter.prevent="executeDeleteServer"/></div>
        </main>
        <footer><button class="back" @click="deleteServerStep===2?deleteServerStep=1:deleteServerTarget=null">{{deleteServerStep===2?'上一步':'取消'}}</button><button class="next danger" :disabled="busy||(deleteServerStep===2&&deleteServerConfirmation!=='delete all')" @click="deleteServerStep===1?advanceDeleteServer():executeDeleteServer()"><LoaderCircle v-if="busy" class="spin"/><Trash2 v-else/>{{deleteServerStep===1?(deleteServerFiles?'继续确认':'移除项目'):'永久删除文件'}}</button></footer>
      </section>
    </div>

  </main>
</template>

<style scoped>
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
.composer :deep(button.model.review.warn){color:#f3a75c}
.composer :deep(button.model.active){color:#e8edf2}
.safe.warn{color:#f3a75c}
.safe.warn :deep(svg){color:#f3a75c}
.fallback-tag{padding:2px 5px;border-radius:4px;color:#8f84d8;background:rgba(156,140,255,.1);font:normal 6px Inter}
.stream-cursor{display:inline-block;width:6px;height:11px;margin-left:2px;vertical-align:-1px;background:#32d5b0;animation:blink 1s steps(2) infinite}
.sidebar :deep(.conversation-tree){flex:1}.sidebar>.spacer{display:none}
.wizard-steps{grid-template-columns:repeat(4,1fr)}
.location-page{grid-template-columns:1fr}.location-preview{display:flex;align-items:center;gap:11px;padding:13px;border:1px solid rgba(156,140,255,.16);border-radius:9px;color:#9c8cff;background:rgba(156,140,255,.055)}.location-preview>svg{width:18px}.location-preview p{display:flex;flex-direction:column;margin:0}.location-preview b{color:#d6d0ff;font-size:10px}.location-preview small{margin-top:4px;color:#6e7985;font-size:8px}.creation-mode{display:grid;grid-template-columns:1fr 1fr;gap:10px}.creation-mode article{display:grid;grid-template-columns:34px 1fr;gap:10px;padding:13px;border:1px solid rgba(255,255,255,.075);border-radius:10px;background:#0e1318}.creation-mode article.smart{border-color:rgba(50,213,176,.16);background:linear-gradient(135deg,rgba(50,213,176,.06),#0e1318 62%)}.creation-mode article>span{width:34px;height:34px;display:grid;place-items:center;border-radius:8px;color:#7d8995;background:#171d24}.creation-mode article.smart>span{color:#32d5b0;background:rgba(50,213,176,.09)}.creation-mode article>span svg{width:17px}.creation-mode p{display:flex;flex-direction:column;margin:0}.creation-mode b{font-size:10px}.creation-mode small{margin-top:5px;color:#687481;font-size:8px;line-height:1.55}.creation-mode button{grid-column:1/-1;height:31px;display:flex;align-items:center;justify-content:center;gap:5px;border:1px solid rgba(255,255,255,.08);border-radius:7px;color:#9aa5b0;background:#171d24;font-size:8px}.creation-mode article.smart button{border:0;color:#06251e;background:#32d5b0;font-weight:700}.creation-mode button:disabled{opacity:.38}.creation-mode button svg{width:13px}
.planning-mission{border-color:rgba(156,140,255,.17);background:linear-gradient(120deg,rgba(156,140,255,.09),rgba(50,213,176,.035))}.planning-mission .agent{color:#b4a9ff;background:rgba(156,140,255,.12)}.empty-mission{border-style:dashed}.empty-mission footer button{height:26px;display:flex;align-items:center;gap:5px;padding:0 9px;border:1px solid rgba(50,213,176,.18);border-radius:6px;color:#83ddc7;background:rgba(50,213,176,.07);font-size:8px}.empty-mission footer button svg{width:12px}
.planning-workspace{min-height:300px;display:flex;align-items:center;justify-content:center;flex-direction:column;padding:40px;border:1px solid rgba(156,140,255,.15);border-radius:12px;background:radial-gradient(circle at 50% 0,rgba(156,140,255,.1),transparent 52%),#11161c;text-align:center}.planning-workspace>span{width:54px;height:54px;display:grid;place-items:center;border-radius:15px;color:#b1a5ff;background:rgba(156,140,255,.1);box-shadow:0 0 30px rgba(156,140,255,.1)}.planning-workspace>span svg{width:26px}.planning-workspace>small{margin-top:17px;color:#746ba8;font-size:7px;font-weight:700;letter-spacing:.15em}.planning-workspace h2{margin:8px 0 0;font-size:16px}.planning-workspace p{max-width:420px;margin:10px 0 0;color:#6e7985;font-size:9px;line-height:1.75}.planning-workspace button{height:33px;display:flex;align-items:center;gap:6px;margin-top:18px;padding:0 13px;border:0;border-radius:7px;color:#09241e;background:#32d5b0;font-size:8px;font-weight:700}.planning-workspace button svg{width:14px}.work-header nav button:disabled{opacity:.3;cursor:not-allowed}
.action-modal{width:min(440px,calc(100vw - 40px));overflow:hidden;border:1px solid rgba(255,255,255,.1);border-radius:13px;background:#12171d;box-shadow:0 28px 80px rgba(0,0,0,.55)}.action-modal>header{height:66px;display:flex;align-items:center;justify-content:space-between;padding:0 20px;border-bottom:1px solid rgba(255,255,255,.07)}.action-modal>header small{color:#5f6b77;font-size:7px;font-weight:700;letter-spacing:.13em}.action-modal>header h2{margin:5px 0 0;font-size:15px}.action-modal>header>button{width:28px;height:28px;display:grid;place-items:center;border:0;border-radius:6px;color:#77838f;background:transparent}.action-modal>header svg{width:15px}.action-modal>main{display:flex;flex-direction:column;gap:13px;padding:20px}.action-modal>footer{height:58px;display:flex;align-items:center;justify-content:flex-end;gap:8px;padding:0 20px;border-top:1px solid rgba(255,255,255,.07);background:#10151b}.action-modal>footer button{height:32px;padding:0 12px;border-radius:7px;font-size:8px;font-weight:650}.action-modal .back{border:1px solid rgba(255,255,255,.08);color:#84909b;background:#161c23}.action-modal .next{border:0;color:#07251e;background:#32d5b0}.action-modal .next.danger{color:#fff;background:#c9575e}.action-modal button:disabled{opacity:.38}.danger-copy{display:flex;align-items:flex-start;gap:12px;padding:13px;border:1px solid rgba(226,92,101,.16);border-radius:9px;color:#e37178;background:rgba(226,92,101,.055)}.danger-copy>svg{width:19px;flex:none}.danger-copy p{display:flex;flex-direction:column;margin:0}.danger-copy b{color:#e6b7ba;font-size:10px}.danger-copy small{margin-top:5px;color:#7f6c70;font-size:8px;line-height:1.6}.danger-copy code{color:#ffb3b7}.danger-copy.critical{border-color:rgba(226,92,101,.28);background:rgba(226,92,101,.08)}.delete-files-check{display:flex;align-items:flex-start;gap:9px;padding:12px;border-radius:9px;background:#0e1318;color:#7c8793}.delete-files-check input{margin-top:2px;accent-color:#d75b63}.delete-files-check span{display:flex;flex-direction:column}.delete-files-check b{font-size:9px}.delete-files-check small{margin-top:4px;color:#606b76;font-size:7px;line-height:1.55}
@media(max-width:700px){.creation-mode{grid-template-columns:1fr}.wizard-steps{grid-template-columns:repeat(4,1fr)}}
@keyframes blink{50%{opacity:0}}
</style>
