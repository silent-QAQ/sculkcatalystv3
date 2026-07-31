<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import {
  Archive,
  ArrowUpRight,
  CircleAlert,
  Database,
  KeyRound,
  LoaderCircle,
  LockKeyhole,
  LogOut,
  ShieldCheck,
  UploadCloud,
} from 'lucide-vue-next'
import MirrorCenterView from './features/mirror/MirrorCenterView.vue'
import type { CatalogSummary } from './features/mirror/types'
import {
  RESOURCE_API_BASE,
  clearResourceAdminCredentials,
  createResourceAdminAuthorization,
  hasResourceAdminCredentials,
  resourceApiRequest,
  setResourceAdminCredentials,
} from './lib/resource-api'

interface AdminVerifyResponse {
  authorized: boolean
  protected: boolean
  upload_max_bytes: number
}

const username = ref('')
const password = ref('')
const authorized = ref(false)
const protectedMode = ref(true)
const checking = ref(false)
const error = ref('')
const summary = ref<CatalogSummary | null>(null)
const uploadMaxBytes = ref(0)
const apiLabel = computed(() => RESOURCE_API_BASE || window.location.origin)
const uploadLimit = computed(() => uploadMaxBytes.value
  ? `${Math.round(uploadMaxBytes.value / 1024 / 1024)} MB`
  : '读取中')

function friendlyError(value: unknown) {
  const raw = value instanceof Error ? value.message : String(value)
  try {
    const parsed = JSON.parse(raw)
    return parsed.error || parsed.message || raw
  } catch {
    return raw
  }
}

async function loadSummary() {
  try {
    summary.value = await resourceApiRequest<CatalogSummary>('/api/catalog/summary')
  } catch (value) {
    error.value = `资源站连接失败：${friendlyError(value)}`
  }
}

async function unlock() {
  checking.value = true
  error.value = ''
  try {
    const authorization = createResourceAdminAuthorization(username.value, password.value)
    await verifyAccess(authorization)
    setResourceAdminCredentials(username.value, password.value)
    password.value = ''
  } catch (value) {
    handleAuthenticationFailure(value)
  } finally {
    checking.value = false
  }
}

async function verifyAccess(authorization = '') {
  const result = await resourceApiRequest<AdminVerifyResponse>('/api/catalog/admin/verify', {
    method: 'POST',
    headers: authorization ? { Authorization: authorization } : undefined,
  })
  if (!result.authorized) throw new Error('invalid credentials')
  authorized.value = true
  protectedMode.value = result.protected
  uploadMaxBytes.value = result.upload_max_bytes
}

function handleAuthenticationFailure(value: unknown) {
  authorized.value = false
  clearResourceAdminCredentials()
  const detail = friendlyError(value)
  error.value = /invalid|unauthorized|forbidden|401|403/i.test(detail)
    ? '账号或密码错误，请重新输入。'
    : `无法验证管理权限：${detail}`
}

async function restoreSession() {
  checking.value = true
  error.value = ''
  try {
    await verifyAccess()
  } catch (value) {
    handleAuthenticationFailure(value)
  } finally {
    checking.value = false
  }
}

function lock() {
  clearResourceAdminCredentials()
  username.value = ''
  password.value = ''
  authorized.value = false
  error.value = ''
}

onMounted(async () => {
  await loadSummary()
  if (hasResourceAdminCredentials()) await restoreSession()
})
</script>

<template>
  <main class="resource-admin" :class="{ unlocked: authorized }">
    <header class="resource-admin__bar">
      <a class="resource-admin__brand" href="/resource-admin" aria-label="资源站管理首页">
        <span><Archive /></span>
        <p><b>Sculk Resource</b><small>独立资源站管理控制台</small></p>
      </a>
      <div class="resource-admin__endpoint"><i/><span><small>RESOURCE API</small><b>{{ apiLabel }}</b></span></div>
      <a :href="`${apiLabel}/api/openapi.json`" target="_blank" rel="noreferrer">接口文档<ArrowUpRight/></a>
      <button v-if="authorized" @click="lock"><LogOut/>锁定管理页</button>
    </header>

    <section v-if="!authorized" class="resource-login">
      <div class="resource-login__intro">
        <span class="resource-login__icon"><LockKeyhole/></span>
        <small>REMOTE RESOURCE OPERATIONS</small>
        <h1>远程维护你的<br/><em>资源流通中心</em></h1>
        <p>在浏览器中直接创建资源项目、上传制品并发布版本。上传文件由资源站计算大小与 SHA-256，开服器总站通过统一解析接口安全拉取。</p>
        <div class="resource-login__flow">
          <span><UploadCloud/><b>上传对象</b><small>浏览器直传源站</small></span>
          <i>→</i>
          <span><Database/><b>发布目录</b><small>版本与兼容性</small></span>
          <i>→</i>
          <span><ShieldCheck/><b>总站校验</b><small>解析、下载、SHA</small></span>
        </div>
      </div>

      <form class="resource-login__card" @submit.prevent="unlock">
        <header><span><KeyRound/></span><div><small>ADMIN ACCESS</small><h2>资源站管理认证</h2></div></header>
        <p>使用管理账号登录。登录信息仅保存在当前标签页，关闭后自动清除。</p>
        <label><span>管理账号</span><input v-model="username" name="username" type="text" autocomplete="username" autofocus placeholder="输入管理账号"/></label>
        <label><span>管理密码</span><input v-model="password" name="password" type="password" autocomplete="current-password" placeholder="输入管理密码"/></label>
        <button :disabled="checking || !username.trim() || !password"><LoaderCircle v-if="checking" class="spin"/><ShieldCheck v-else/>登录并进入</button>
        <div v-if="error" class="resource-login__error"><CircleAlert/>{{ error }}</div>
        <footer>
          <span><i :class="{ online: summary }"/>{{ summary ? '只读 API 在线' : '等待资源站' }}</span>
          <span>{{ summary?.versions ?? '—' }} 个版本</span>
        </footer>
      </form>
    </section>

    <section v-else class="resource-admin__workspace">
      <div class="resource-admin__status">
        <span><ShieldCheck/><p><b>管理权限已验证</b><small>{{ protectedMode ? '写接口受账号认证保护' : '开发模式：服务端未启用管理认证' }}</small></p></span>
        <span><UploadCloud/><p><b>单文件上限 {{ uploadLimit }}</b><small>上传后自动填写 URL、大小与 SHA-256</small></p></span>
        <span><Database/><p><b>{{ summary?.versions ?? 0 }} 个目录版本</b><small>{{ summary?.published_versions ?? 0 }} 个已发布</small></p></span>
      </div>
      <MirrorCenterView admin-mode @catalog-updated="summary = $event" />
    </section>
  </main>
</template>
