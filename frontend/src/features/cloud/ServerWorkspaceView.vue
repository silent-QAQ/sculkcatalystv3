<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { Download, ExternalLink, FileJson, FileUp, Pencil, Plus, Save, Server, Trash2, X } from 'lucide-vue-next'
import {
  MAX_SERVER_TEMPLATE_FILE_BYTES, createServerTemplate, exportServerTemplateManifest,
  parseServerTemplateManifest, type ServerTemplate,
} from '../portable/server-manifest'
import { cloudServerTemplates, saveCloudServerTemplates } from './workspace'
import './server-workspace.css'

const props = withDefaults(defineProps<{ context?: 'desktop' | 'web'; busy?: boolean }>(), {
  context: 'desktop',
  busy: false,
})
const emit = defineEmits<{
  changed: []
  apply: [template: ServerTemplate]
}>()

const emptyDraft = () => ({
  title: '', description: '', name: '', core: 'Paper', minecraft_version: '1.21.4', memory_gb: 8, port: 25565,
})
const draft = ref(emptyDraft())
const editingId = ref('')
const importInput = ref<HTMLInputElement | null>(null)
const localMessage = ref('')
const localError = ref('')

const localConsoleUrl = computed(() => {
  const configured = import.meta.env.VITE_LOCAL_CONSOLE_URL || 'http://127.0.0.1:8787/'
  try {
    const url = new URL(configured)
    if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)) throw new Error()
    url.username = ''
    url.password = ''
    url.search = ''
    url.hash = ''
    url.searchParams.set('cloud', 'workspace')
    return url.toString()
  } catch {
    return 'http://127.0.0.1:8787/'
  }
})

function clearFeedback() {
  localMessage.value = ''
  localError.value = ''
}

function resetDraft() {
  editingId.value = ''
  draft.value = emptyDraft()
  clearFeedback()
}

function editTemplate(template: ServerTemplate) {
  editingId.value = template.id
  draft.value = {
    title: template.title,
    description: template.description,
    name: template.server.name,
    core: template.server.core,
    minecraft_version: template.server.minecraft_version,
    memory_gb: template.server.memory_gb,
    port: template.server.port,
  }
  clearFeedback()
}

function saveTemplate() {
  clearFeedback()
  try {
    const next = createServerTemplate({
      title: draft.value.title,
      description: draft.value.description,
      server: {
        name: draft.value.name,
        core: draft.value.core,
        minecraft_version: draft.value.minecraft_version,
        memory_gb: Number(draft.value.memory_gb),
        port: Number(draft.value.port),
      },
    }, editingId.value || crypto.randomUUID())
    const existing = cloudServerTemplates.value.find(item => item.id === editingId.value)
    const saved = existing ? { ...next, created_at: existing.created_at } : next
    saveCloudServerTemplates(existing
      ? cloudServerTemplates.value.map(item => item.id === existing.id ? saved : item)
      : [saved, ...cloudServerTemplates.value])
    localMessage.value = existing ? '模板已更新，正在同步云端。' : '模板已创建，正在同步云端。'
    editingId.value = ''
    draft.value = emptyDraft()
    emit('changed')
  } catch (error) {
    localError.value = error instanceof Error ? error.message : String(error)
  }
}

function removeTemplate(id: string) {
  saveCloudServerTemplates(cloudServerTemplates.value.filter(item => item.id !== id))
  if (editingId.value === id) resetDraft()
  localMessage.value = '模板已移除，正在同步云端。'
  emit('changed')
}

function downloadTemplate(template: ServerTemplate) {
  const blob = new Blob([exportServerTemplateManifest(template)], { type: 'application/json;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `${template.title.replace(/[^\p{L}\p{N}._-]+/gu, '-') || 'sculk-server-template'}.json`
  anchor.click()
  URL.revokeObjectURL(url)
  localMessage.value = '配置文件已导出；其中不包含密钥、路径、运行状态或 EULA 确认。'
}

async function importTemplate(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return
  clearFeedback()
  try {
    if (file.size > MAX_SERVER_TEMPLATE_FILE_BYTES) throw new Error('配置文件不能超过 64 KiB')
    if (cloudServerTemplates.value.length >= 50) throw new Error('最多保存 50 个开服参数模板')
    const imported = parseServerTemplateManifest(await file.text())
    saveCloudServerTemplates([imported, ...cloudServerTemplates.value])
    localMessage.value = `已导入“${imported.title}”，正在同步云端。`
    emit('changed')
  } catch (error) {
    localError.value = error instanceof Error ? error.message : String(error)
  }
}

function openLocalConsole() {
  window.open(localConsoleUrl.value, '_blank', 'noopener,noreferrer')
}
</script>

<template>
  <div class="server-workspace-view">
    <section class="workspace-intro">
      <div><Server/><p><small>PORTABLE SERVER TEMPLATE</small><b>开服参数工作区</b><span>把常用开服参数保存到账号，在任意本机控制台载入后继续环境检查。</span></p></div>
      <aside>
        <button @click="importInput?.click()"><FileUp/>导入配置</button>
        <button v-if="context === 'web'" class="primary" @click="openLocalConsole"><ExternalLink/>打开本机控制台</button>
      </aside>
      <input ref="importInput" class="workspace-file-input" type="file" accept="application/json,.json" @change="importTemplate"/>
    </section>

    <p class="workspace-boundary">模板只保存创建参数，不会创建服务器、下载核心、访问本机文件或启动进程。载入后仍需确认环境与 Minecraft EULA。</p>
    <p v-if="localMessage" class="workspace-feedback ok">{{ localMessage }}</p>
    <p v-if="localError" class="workspace-feedback error">{{ localError }}</p>

    <section class="workspace-editor cloud-panel">
      <header><div><h3>{{ editingId ? '编辑参数模板' : '新建参数模板' }}</h3><p>请勿在名称或说明中填写密钥、本机路径或私人信息</p></div><button v-if="editingId" class="cloud-icon-btn" title="取消编辑" @click="resetDraft"><X/></button></header>
      <form @submit.prevent="saveTemplate">
        <label>模板名称<input v-model="draft.title" maxlength="64" placeholder="Paper 生存服" required/></label>
        <label>服务器名称<input v-model="draft.name" maxlength="64" placeholder="深暗生存服" required/></label>
        <label class="wide">模板说明<input v-model="draft.description" maxlength="500" placeholder="适合中小型纯生存服务器"/></label>
        <label>服务端核心<input v-model="draft.core" maxlength="64" placeholder="Paper" required/></label>
        <label>Minecraft 版本<input v-model="draft.minecraft_version" maxlength="32" placeholder="1.21.4" required/></label>
        <label>最大内存（GB）<input v-model.number="draft.memory_gb" type="number" min="2" max="64" step="1" required/></label>
        <label>首选端口<input v-model.number="draft.port" type="number" min="1024" max="65535" step="1" required/></label>
        <button class="cloud-primary compact" :disabled="busy"><Save v-if="editingId"/><Plus v-else/>{{ editingId ? '保存修改' : '创建模板' }}</button>
      </form>
    </section>

    <section class="workspace-template-list">
      <article v-for="template in cloudServerTemplates" :key="template.id" class="cloud-panel template-card">
        <header><span><Server/></span><div><h3>{{ template.title }}</h3><p>{{ template.description || '未填写模板说明' }}</p></div><time>{{ new Date(template.updated_at).toLocaleDateString('zh-CN') }}</time></header>
        <dl>
          <div><dt>服务器</dt><dd>{{ template.server.name }}</dd></div>
          <div><dt>核心</dt><dd>{{ template.server.core }} {{ template.server.minecraft_version }}</dd></div>
          <div><dt>资源</dt><dd>{{ template.server.memory_gb }} GB · {{ template.server.port }}</dd></div>
        </dl>
        <footer>
          <button title="编辑模板" @click="editTemplate(template)"><Pencil/>编辑</button>
          <button title="导出开放配置" @click="downloadTemplate(template)"><Download/>导出</button>
          <button v-if="context === 'desktop'" class="apply" @click="emit('apply', template)"><FileJson/>载入创建向导</button>
          <button class="danger" title="删除模板" :disabled="busy" @click="removeTemplate(template.id)"><Trash2/></button>
        </footer>
      </article>
      <div v-if="!cloudServerTemplates.length" class="workspace-empty cloud-panel"><FileJson/><b>还没有开服参数模板</b><p>可以创建新模板，或者导入符合开放格式的 JSON 配置文件。</p></div>
    </section>
  </div>
</template>
