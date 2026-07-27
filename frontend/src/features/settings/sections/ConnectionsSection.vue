<script setup lang="ts">
import { computed, ref } from 'vue'
import { FlaskConical, Globe, LoaderCircle, Pencil, Plus, Trash2, X } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import type { RemoteConnection, TestResult, UiSettings } from '../types'
import { flash, friendly, uiSettings } from '../store'

const connections = computed(() => uiSettings.value?.connections ?? [])

const modal = ref(false)
const modalMode = ref<'create' | 'edit'>('create')
const editingId = ref('')
const saving = ref(false)
const formError = ref('')
const form = ref({ name: '', protocol: 'ssh' as 'ssh' | 'sftp', host: '', port: 22, username: '', root_path: '', enabled: true })
const testingId = ref('')
const testResults = ref<Record<string, TestResult>>({})
const confirmOpen = ref(false)
const confirmTarget = ref<RemoteConnection | null>(null)
const deleting = ref(false)

function openCreate() {
  modalMode.value = 'create'; editingId.value = ''
  form.value = { name: '', protocol: 'ssh', host: '', port: 22, username: '', root_path: '', enabled: true }
  formError.value = ''; modal.value = true
}
function openEdit(connection: RemoteConnection) {
  modalMode.value = 'edit'; editingId.value = connection.id
  form.value = { name: connection.name, protocol: connection.protocol, host: connection.host, port: connection.port, username: connection.username, root_path: connection.root_path, enabled: connection.enabled }
  formError.value = ''; modal.value = true
}
async function save() {
  if (!form.value.name.trim() || !form.value.host.trim()) { formError.value = '请填写连接名称和主机地址'; return }
  saving.value = true; formError.value = ''
  try {
    const path = modalMode.value === 'create' ? '/api/ui/connections' : '/api/ui/connections/' + editingId.value
    uiSettings.value = await apiRequest<UiSettings>(path, {
      method: modalMode.value === 'create' ? 'POST' : 'PUT',
      body: JSON.stringify({ ...form.value, name: form.value.name.trim(), host: form.value.host.trim(), username: form.value.username.trim(), root_path: form.value.root_path.trim() }),
    })
    modal.value = false
    flash(modalMode.value === 'create' ? '连接已添加，可点击「测试」验证连通性' : '连接已更新')
  } catch (error) { formError.value = friendly(error) }
  finally { saving.value = false }
}
async function toggle(connection: RemoteConnection) {
  try {
    uiSettings.value = await apiRequest<UiSettings>('/api/ui/connections/' + connection.id, {
      method: 'PUT',
      body: JSON.stringify({ name: connection.name, protocol: connection.protocol, host: connection.host, port: connection.port, username: connection.username, root_path: connection.root_path, enabled: !connection.enabled }),
    })
  } catch (error) { flash('操作失败：' + friendly(error)) }
}
function askDelete(connection: RemoteConnection) { confirmTarget.value = connection; confirmOpen.value = true }
async function doDelete() {
  if (!confirmTarget.value) return
  deleting.value = true
  try {
    uiSettings.value = await apiRequest<UiSettings>('/api/ui/connections/' + confirmTarget.value.id, { method: 'DELETE' })
    confirmOpen.value = false
    flash('连接已删除')
  } catch (error) { flash('删除失败：' + friendly(error)) }
  finally { deleting.value = false }
}
async function test(connection: RemoteConnection) {
  testingId.value = connection.id
  try {
    testResults.value = { ...testResults.value, [connection.id]: await apiRequest<TestResult>('/api/ui/connections/' + connection.id + '/test', { method: 'POST' }) }
  } catch (error) {
    testResults.value = { ...testResults.value, [connection.id]: { ok: false, latency_ms: 0, error: friendly(error) } }
  } finally { testingId.value = '' }
}
</script>

<template>
  <div class="s-group">
    <h2 style="display:flex;align-items:center;justify-content:space-between">远程服务器连接<button class="s-btn primary" @click="openCreate"><Plus/>添加连接</button></h2>
    <p class="desc">连接远程服务器中的开服项目（SSH / SFTP）。凭据不落库，测试仅验证 TCP 连通性；协议级认证在实际连接时进行。</p>
    <div v-if="!connections.length" class="s-empty">还没有远程连接。点击「添加连接」接入运行在云主机或家用服务器上的 Minecraft 项目。</div>
    <div v-else class="s-card">
      <div v-for="connection in connections" :key="connection.id" class="s-row" :style="{opacity:connection.enabled?1:.55}">
        <span style="display:grid;place-items:center;width:29px;height:29px;border-radius:7px;color:var(--accent);background:color-mix(in srgb,var(--accent) 10%,transparent);flex:none"><Globe style="width:14px"/></span>
        <p>
          <b>{{ connection.name }}<em style="margin-left:6px;padding:2px 5px;border-radius:4px;color:#8f84d8;background:rgba(156,140,255,.1);font:normal 6px Inter">{{ connection.protocol.toUpperCase() }}</em></b>
          <small><code>{{ connection.username ? connection.username + '@' : '' }}{{ connection.host }}:{{ connection.port }}</code><template v-if="connection.root_path"> · {{ connection.root_path }}</template></small>
        </p>
        <span v-if="testResults[connection.id]" class="s-test" :class="{ok:testResults[connection.id].ok}">
          <template v-if="testResults[connection.id].ok">✓ 可达 · {{ testResults[connection.id].latency_ms }} ms</template>
          <template v-else>✗ {{ testResults[connection.id].error }}</template>
        </span>
        <button class="s-btn small" :disabled="testingId===connection.id" @click="test(connection)"><LoaderCircle v-if="testingId===connection.id" class="s-spin"/><FlaskConical v-else/>测试</button>
        <button class="s-btn small" @click="openEdit(connection)"><Pencil/></button>
        <button class="s-btn small danger" @click="askDelete(connection)"><Trash2/></button>
        <button class="s-switch" :class="{on:connection.enabled}" @click="toggle(connection)"><i/></button>
      </div>
    </div>
  </div>

  <div v-if="modal" class="s-modal-backdrop" @click.self="modal=false">
    <section class="s-modal">
      <header><b>{{ modalMode==='create' ? '添加远程连接' : '编辑远程连接' }}</b><button @click="modal=false"><X/></button></header>
      <div class="field"><label>连接名称</label><input class="s-input" v-model="form.name" placeholder="例如：阿里云生存服"/></div>
      <div class="field"><label>协议</label>
        <span class="s-seg">
          <button :class="{active:form.protocol==='ssh'}" @click="form.protocol='ssh'">SSH</button>
          <button :class="{active:form.protocol==='sftp'}" @click="form.protocol='sftp'">SFTP</button>
        </span>
      </div>
      <div class="field"><label>主机地址</label><input class="s-input" v-model="form.host" placeholder="1.2.3.4 或 mc.example.com"/></div>
      <div class="field"><label>端口</label><input class="s-input" style="width:120px" v-model.number="form.port" type="number" min="1" max="65535"/></div>
      <div class="field"><label>用户名</label><input class="s-input" v-model="form.username" placeholder="root（可留空）"/></div>
      <div class="field"><label>项目根目录</label><input class="s-input" v-model="form.root_path" placeholder="/opt/minecraft（可留空）"/><small>远程开服项目所在路径</small></div>
      <label class="check"><input v-model="form.enabled" type="checkbox"/><span>启用该连接</span></label>
      <p v-if="formError" class="s-error">{{ formError }}</p>
      <footer><button class="s-btn" @click="modal=false">取消</button><button class="s-btn primary" :disabled="saving" @click="save"><LoaderCircle v-if="saving" class="s-spin"/>{{ modalMode==='create' ? '添加' : '保存' }}</button></footer>
    </section>
  </div>

  <div v-if="confirmOpen" class="s-modal-backdrop" @click.self="confirmOpen=false">
    <section class="s-modal">
      <header><b>删除 {{ confirmTarget?.name }}？</b><button @click="confirmOpen=false"><X/></button></header>
      <p class="confirm-body">该远程连接配置会被移除。此操作不影响远程服务器本身。</p>
      <footer><button class="s-btn" @click="confirmOpen=false">取消</button><button class="s-btn danger-solid" :disabled="deleting" @click="doDelete"><LoaderCircle v-if="deleting" class="s-spin"/>确认删除</button></footer>
    </section>
  </div>
</template>
