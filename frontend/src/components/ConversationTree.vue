<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import {
  Archive,
  ArchiveRestore,
  BellDot,
  ChevronDown,
  ChevronRight,
  Copy,
  FolderInput,
  MessageSquareText,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Server,
  Trash2,
} from 'lucide-vue-next'
import type { ConversationAction, ConversationSummary } from '../features/conversations/types'

interface ServerItem {
  id: string
  name: string
  core: string
  version: string
  status: 'online' | 'stopped' | 'warning' | 'planning'
  players: string
  memory: number
  memory_gb?: number
  cpu: number
  port: number
  task: string
  location?: string
}

const props = defineProps<{
  servers: ServerItem[]
  conversations: Record<string, ConversationSummary[]>
  selectedServerId: string
  selectedConversationId: string
  collapsed: boolean
}>()

const emit = defineEmits<{
  selectServer: [id: string]
  selectConversation: [conversation: ConversationSummary]
  newConversation: [serverId: string]
  conversationAction: [action: ConversationAction, conversation: ConversationSummary]
  deleteServer: [server: ServerItem]
}>()

const expanded = ref<Record<string, boolean>>({})
const openMenu = ref('')
const archivedOpen = ref<Record<string, boolean>>({})

function isExpanded(serverId: string) {
  return expanded.value[serverId] ?? serverId === props.selectedServerId
}

function toggleServer(serverId: string) {
  expanded.value[serverId] = !isExpanded(serverId)
}

function rows(serverId: string, archived: boolean) {
  return (props.conversations[serverId] ?? [])
    .filter(item => item.archived === archived)
    .sort((left, right) => Number(right.pinned) - Number(left.pinned) || right.updated_at.localeCompare(left.updated_at))
}

function groups(serverId: string) {
  const grouped = new Map<string, ConversationSummary[]>()
  for (const item of rows(serverId, false)) {
    const key = item.group?.trim() || ''
    const items = grouped.get(key) ?? []
    items.push(item)
    grouped.set(key, items)
  }
  return [...grouped.entries()].map(([name, items]) => ({ name, items }))
}

function serverSubtitle(server: ServerItem) {
  if (server.status === 'planning') return '规划中 · 尚未创建文件'
  return [server.core, server.version].filter(Boolean).join(' ') + ' · ' + server.players
}

function conversationRuntime(conversation: ConversationSummary) {
  if (conversation.agent_override && conversation.agent_override !== 'default') return 'ACP Agent'
  if (conversation.model_binding) return conversation.model_binding.model_id
  return '自动模型'
}

function closeMenus() { openMenu.value = '' }
onMounted(() => document.addEventListener('click', closeMenus))
onUnmounted(() => document.removeEventListener('click', closeMenus))

function chooseConversation(item: ConversationSummary) {
  openMenu.value = ''
  emit('selectConversation', item)
}

function runAction(action: ConversationAction, item: ConversationSummary) {
  openMenu.value = ''
  emit('conversationAction', action, item)
}
</script>

<template>
  <div class="conversation-tree" :class="{ compact: collapsed }">
    <article v-for="item in servers" :key="item.id" class="server-node">
      <div class="server-row" :class="{ selected: selectedServerId === item.id }">
        <button class="server-main" @click="emit('selectServer', item.id)">
          <span class="server-icon"><Server :size="16"/></span>
          <span v-if="!collapsed" class="server-copy">
            <b>{{ item.name }}</b>
            <small>{{ serverSubtitle(item) }}</small>
          </span>
          <i class="dot" :class="item.status"/>
        </button>
        <template v-if="!collapsed">
          <button class="tree-icon expand" :aria-label="isExpanded(item.id) ? '收起对话' : '展开对话'" @click.stop="toggleServer(item.id)">
            <ChevronDown v-if="isExpanded(item.id)"/>
            <ChevronRight v-else/>
          </button>
          <span class="tree-menu-anchor">
            <button class="tree-icon" aria-label="服务器菜单" @click.stop="openMenu = openMenu === 'server:'+item.id ? '' : 'server:'+item.id"><MoreHorizontal/></button>
            <div v-if="openMenu === 'server:'+item.id" class="tree-menu server-menu">
              <button @click="openMenu='';emit('newConversation', item.id)"><Plus/>新建对话任务</button>
              <button class="danger" @click="openMenu='';emit('deleteServer', item)"><Trash2/>删除服务器</button>
            </div>
          </span>
        </template>
      </div>

      <div v-if="!collapsed && isExpanded(item.id)" class="conversation-children">
        <button class="new-chat" @click="emit('newConversation', item.id)"><Plus/>新建对话任务</button>

        <template v-for="group in groups(item.id)" :key="group.name || '__root'">
          <small v-if="group.name" class="group-label"><FolderInput/>{{ group.name }}</small>
          <div v-for="conversation in group.items" :key="conversation.id" class="conversation-row" :class="{ active:selectedConversationId===conversation.id, unread:conversation.unread }">
            <button class="conversation-main" @click="chooseConversation(conversation)">
              <MessageSquareText/>
              <span><b>{{ conversation.title }}</b><small>{{ conversation.message_count }} 条 · {{ conversationRuntime(conversation) }}</small></span>
              <Pin v-if="conversation.pinned" class="pin-mark"/>
              <i v-if="conversation.unread" class="unread-dot"/>
            </button>
            <span class="tree-menu-anchor">
              <button class="conversation-more" aria-label="对话菜单" @click.stop="openMenu = openMenu === conversation.id ? '' : conversation.id"><MoreHorizontal/></button>
              <div v-if="openMenu===conversation.id" class="tree-menu">
                <button @click="runAction('rename',conversation)"><Pencil/>重命名</button>
                <button @click="runAction('group',conversation)"><FolderInput/>移动到组</button>
                <button @click="runAction('pin',conversation)"><PinOff v-if="conversation.pinned"/><Pin v-else/>{{conversation.pinned?'取消固定':'固定'}}</button>
                <button @click="runAction('fork',conversation)"><Copy/>分叉</button>
                <button @click="runAction('unread',conversation)"><BellDot/>{{conversation.unread?'标记已读':'标记未读'}}</button>
                <button @click="runAction('archive',conversation)"><Archive/>归档</button>
                <button class="danger" @click="runAction('delete',conversation)"><Trash2/>删除</button>
              </div>
            </span>
          </div>
        </template>

        <div v-if="!rows(item.id,false).length" class="conversation-empty">还没有对话任务</div>

        <button v-if="rows(item.id,true).length" class="archive-toggle" @click="archivedOpen[item.id]=!archivedOpen[item.id]">
          <Archive/>已归档 {{ rows(item.id,true).length }}<ChevronDown v-if="archivedOpen[item.id]"/><ChevronRight v-else/>
        </button>
        <div v-if="archivedOpen[item.id]" class="archived-list">
          <div v-for="conversation in rows(item.id,true)" :key="conversation.id" class="conversation-row archived">
            <button class="conversation-main" @click="chooseConversation(conversation)"><MessageSquareText/><span><b>{{conversation.title}}</b><small>{{conversation.message_count}} 条 · {{ conversationRuntime(conversation) }}</small></span></button>
            <span class="tree-menu-anchor">
              <button class="conversation-more" aria-label="归档对话菜单" @click.stop="openMenu = openMenu === conversation.id ? '' : conversation.id"><MoreHorizontal/></button>
              <div v-if="openMenu===conversation.id" class="tree-menu">
                <button @click="runAction('archive',conversation)"><ArchiveRestore/>移出归档</button>
                <button @click="runAction('fork',conversation)"><Copy/>分叉</button>
                <button class="danger" @click="runAction('delete',conversation)"><Trash2/>删除</button>
              </div>
            </span>
          </div>
        </div>
      </div>
    </article>
    <div v-if="!collapsed && !servers.length" class="tree-empty"><Server/><span>还没有服务器项目</span></div>
  </div>
</template>

<style scoped>
.conversation-tree{display:flex;min-height:0;flex-direction:column;gap:3px;overflow:auto;padding:0 5px 12px}.server-node{position:relative}.server-row{position:relative;display:flex;align-items:center;border-radius:8px}.server-row:hover,.server-row.selected{background:rgba(255,255,255,.045)}.server-main{min-width:0;min-height:43px;display:flex;flex:1;align-items:center;gap:9px;padding:7px 6px;border:0;color:#8995a1;background:transparent;text-align:left}.server-row.selected .server-main{color:#d5dce3}.server-icon{width:27px;height:27px;display:grid;place-items:center;flex:none;border-radius:7px;color:#6f7c89;background:#171d24}.server-row.selected .server-icon{color:#53d8b9;background:rgba(50,213,176,.09)}.server-copy{display:flex;min-width:0;flex:1;flex-direction:column;gap:4px}.server-copy b,.server-copy small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.server-copy b{font-size:9px}.server-copy small{color:#596572;font-size:7px}.dot{width:6px;height:6px;flex:none;border-radius:50%;background:#6c7783}.dot.online{background:#32d5b0}.dot.warning{background:#f3a75c}.dot.planning{background:#9c8cff;box-shadow:0 0 7px rgba(156,140,255,.45)}.tree-icon,.conversation-more{width:24px;height:28px;display:grid;place-items:center;flex:none;border:0;border-radius:5px;color:#586470;background:transparent;opacity:0}.server-row:hover .tree-icon,.conversation-row:hover .conversation-more,.tree-icon:focus-visible,.conversation-more:focus-visible{opacity:1}.tree-icon:hover,.conversation-more:hover{color:#b9c2cb;background:rgba(255,255,255,.05)}.tree-icon svg,.conversation-more svg{width:13px}.tree-icon.expand{opacity:.7}.conversation-children{position:relative;margin:2px 0 6px 17px;padding-left:12px;border-left:1px solid rgba(255,255,255,.065)}.new-chat,.archive-toggle{width:100%;height:28px;display:flex;align-items:center;gap:6px;padding:0 7px;border:0;border-radius:6px;color:#667482;background:transparent;font-size:8px}.new-chat:hover,.archive-toggle:hover{color:#aeb7c0;background:rgba(255,255,255,.035)}.new-chat svg,.archive-toggle svg{width:12px}.group-label{height:25px;display:flex;align-items:center;gap:5px;padding:0 8px;color:#586572;font-size:7px;text-transform:uppercase}.group-label svg{width:11px}.conversation-row{position:relative;display:flex;align-items:center;border-radius:6px}.conversation-row:hover,.conversation-row.active{background:rgba(255,255,255,.04)}.conversation-row.active{box-shadow:inset 2px 0 #32d5b0}.conversation-main{min-width:0;height:32px;display:flex;flex:1;align-items:center;gap:7px;padding:0 6px;border:0;color:#788592;background:transparent;text-align:left}.conversation-main>svg{width:12px;flex:none}.conversation-main>span{min-width:0;display:flex;flex:1;flex-direction:column;gap:2px}.conversation-main b,.conversation-main small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.conversation-main b{font-size:8px;font-weight:550}.conversation-main small{color:#505c68;font-size:6px}.conversation-row.active .conversation-main{color:#cbd4dc}.pin-mark{width:10px!important;color:#9c8cff}.unread-dot{width:5px;height:5px;flex:none;border-radius:50%;background:#32d5b0}.tree-menu-anchor{position:relative;display:inline-flex}.tree-menu{position:absolute;top:27px;right:0;z-index:60;width:145px;padding:5px;border:1px solid rgba(255,255,255,.09);border-radius:8px;background:#161c23;box-shadow:0 12px 30px rgba(0,0,0,.48)}.tree-menu.server-menu{top:30px}.tree-menu button{width:100%;height:29px;display:flex;align-items:center;gap:7px;padding:0 8px;border:0;border-radius:5px;color:#a3adb7;background:transparent;font-size:8px;text-align:left}.tree-menu button:hover{color:#e0e5ea;background:rgba(255,255,255,.05)}.tree-menu button.danger{color:#e98b91}.tree-menu svg{width:12px}.conversation-empty,.tree-empty{padding:10px 7px;color:#4f5b67;font-size:7px}.archive-toggle{justify-content:flex-start;margin-top:3px}.archive-toggle svg:last-child{margin-left:auto}.archived-list{opacity:.72}.tree-empty{display:flex;align-items:center;gap:7px;padding:12px}.tree-empty svg{width:14px}.compact{padding-inline:0}.compact .server-row{justify-content:center}.compact .server-main{min-width:40px;max-width:40px;justify-content:center;padding:0}.compact .server-main>.dot{position:absolute;right:2px;bottom:3px;border:2px solid #0d1116}
</style>
