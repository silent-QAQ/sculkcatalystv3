// 普通控制台经同源只读代理读取远端目录，避免第三方自部署域名被 CORS 拦截。
// 独立管理页默认直连官方源站；开源部署可通过构建变量指向自己的资源中心。
export const OFFICIAL_RESOURCE_API_BASE = 'https://res.mcmy.love'
export const DEFAULT_RESOURCE_API_BASE = '/api/resource-catalog'
const defaultResourceApiBase = typeof window !== 'undefined' && window.location.pathname.startsWith('/resource-admin')
  ? OFFICIAL_RESOURCE_API_BASE
  : DEFAULT_RESOURCE_API_BASE
export const RESOURCE_API_BASE = (
  import.meta.env.VITE_RESOURCE_API_BASE || defaultResourceApiBase
).replace(/\/$/, '')

const ADMIN_AUTH_KEY = 'sculk.resource-admin-authorization'
const LEGACY_ADMIN_TOKEN_KEY = 'sculk.resource-admin-token'
let resourceAdminAuthorization = typeof window === 'undefined'
  ? ''
  : window.sessionStorage.getItem(ADMIN_AUTH_KEY) || ''

if (typeof window !== 'undefined') window.sessionStorage.removeItem(LEGACY_ADMIN_TOKEN_KEY)
if (!resourceAdminAuthorization.startsWith('Basic ')) resourceAdminAuthorization = ''

export function createResourceAdminAuthorization(username: string, password: string) {
  const normalizedUsername = username.trim()
  if (!normalizedUsername || !password) throw new Error('请输入管理账号和密码。')
  if (normalizedUsername.includes(':')) throw new Error('管理账号不能包含冒号。')
  const bytes = new TextEncoder().encode(`${normalizedUsername}:${password}`)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return `Basic ${window.btoa(binary)}`
}

export function hasResourceAdminCredentials() {
  return Boolean(resourceAdminAuthorization)
}

export function setResourceAdminCredentials(username: string, password: string) {
  resourceAdminAuthorization = createResourceAdminAuthorization(username, password)
  if (typeof window === 'undefined') return
  window.sessionStorage.setItem(ADMIN_AUTH_KEY, resourceAdminAuthorization)
  window.sessionStorage.removeItem(LEGACY_ADMIN_TOKEN_KEY)
}

export function clearResourceAdminCredentials() {
  resourceAdminAuthorization = ''
  if (typeof window === 'undefined') return
  window.sessionStorage.removeItem(ADMIN_AUTH_KEY)
  window.sessionStorage.removeItem(LEGACY_ADMIN_TOKEN_KEY)
}

export function resourceApiUrl(path: string) {
  return `${RESOURCE_API_BASE}${path.startsWith('/') ? path : `/${path}`}`
}

export async function resourceApiRequest<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers = new Headers(options.headers)
  if (options.body && !headers.has('Content-Type')) headers.set('Content-Type', 'application/json')
  if (resourceAdminAuthorization && !headers.has('Authorization')) {
    headers.set('Authorization', resourceAdminAuthorization)
  }

  const response = await fetch(resourceApiUrl(path), { ...options, headers })
  if (!response.ok) {
    const message = await response.text()
    throw new Error(message || `资源接口请求失败（HTTP ${response.status}）`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}
