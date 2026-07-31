// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

import { apiUrl } from '../../lib/api'

const TOKEN_KEY = 'sculk-cloud-session'
export const CLOUD_SESSION_EXPIRED_EVENT = 'sculk-cloud-session-expired'
export type AgentBootstrapPlatform = 'windows' | 'linux'
export const AGENT_RELEASE_VERSION = '20260731-running-cancel-v1'

export function agentDownloadPath(platform: AgentBootstrapPlatform) {
  const filename = platform === 'windows'
    ? 'sculk-agent-windows-x86_64.exe'
    : 'sculk-agent-linux-x86_64'
  return `/downloads/${filename}?v=${encodeURIComponent(AGENT_RELEASE_VERSION)}`
}

export function agentChecksumsPath() {
  return `/downloads/sculk-agent-SHA256SUMS.txt?v=${encodeURIComponent(AGENT_RELEASE_VERSION)}`
}

export interface AgentBootstrapDownload {
  platform: AgentBootstrapPlatform
  downloadUrl: string
  configFilename: string | null
  configJson: unknown | null
  expiresAt: string | null
}

export interface AgentBootstrapInput {
  platform: AgentBootstrapPlatform
  name: string
  workspaceLabel: string
  workspaceRoot: string
}

export class CloudApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(message: string, status: number, code: string) {
    super(message)
    this.name = 'CloudApiError'
    this.status = status
    this.code = code
  }
}

export function cloudSession() {
  return window.localStorage.getItem(TOKEN_KEY) || ''
}

export function setCloudSession(token: string) {
  if (token) window.localStorage.setItem(TOKEN_KEY, token)
  else window.localStorage.removeItem(TOKEN_KEY)
}

export async function cloudRequest<T>(path: string, options: RequestInit = {}, authenticated = true): Promise<T> {
  if (!path.startsWith('/api/cloud/')) throw new Error('Cloud 客户端仅允许访问 /api/cloud/*')
  const headers = new Headers(options.headers)
  if (options.body && !headers.has('Content-Type')) headers.set('Content-Type', 'application/json')
  if (authenticated) {
    const token = cloudSession()
    if (token) headers.set('Authorization', `Bearer ${token}`)
  }
  const response = await fetch(apiUrl(path), { ...options, headers })
  if (!response.ok) {
    const raw = await response.text()
    let message = raw || `请求失败（HTTP ${response.status}）`
    let code = 'http_error'
    try {
      const parsed = JSON.parse(raw) as { error?: { code?: string; message?: string } }
      message = parsed.error?.message || message
      code = parsed.error?.code || code
    } catch {}
    if (response.status === 401 && authenticated) {
      setCloudSession('')
      window.dispatchEvent(new Event(CLOUD_SESSION_EXPIRED_EVENT))
    }
    throw new CloudApiError(message, response.status, code)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

/**
 * 生成仅用于首次启动 Agent 的短期下载信息。不要将返回的配置写入本地存储：
 * 它可能包含只能使用一次的配对凭据。
 */
export async function createAgentBootstrap(input: AgentBootstrapInput): Promise<AgentBootstrapDownload> {
  const platform = input.platform
  const response = await cloudRequest<Record<string, unknown>>('/api/cloud/agent-bootstrap', {
    method: 'POST',
    // 当前服务端会直接返回短期 bootstrap JSON；新版服务端可返回下载包 URL。
    body: JSON.stringify({
      platform: platform === 'windows' ? 'windows-x86_64' : 'linux-x86_64',
      name: input.name,
      workspace_label: input.workspaceLabel,
      workspace_root: input.workspaceRoot,
    }),
  })
  const value = (response.bootstrap && typeof response.bootstrap === 'object'
    ? response.bootstrap
    : response) as Record<string, unknown>
  const rawUrl = value.download_url ?? value.downloadUrl ?? value.bundle_url ?? value.bundleUrl ?? value.url
  let downloadUrl: string
  if (typeof rawUrl === 'string' && rawUrl.trim()) {
    try {
      const parsed = new URL(rawUrl, window.location.origin)
      if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password) throw new Error()
      downloadUrl = parsed.toString()
    } catch {
      throw new Error('服务端返回的 Agent 下载地址无效，请重新生成启动包')
    }
  } else {
    // 兼容先返回配置 JSON 的接口：Agent 二进制仍由云端静态下载地址提供。
    downloadUrl = new URL(
      agentDownloadPath(platform),
      window.location.origin,
    ).toString()
  }
  const responsePlatform = value.platform ?? value.target_platform ?? platform
  const normalizedPlatform = typeof responsePlatform === 'string' && responsePlatform.toLowerCase().startsWith('win')
    ? 'windows'
    : typeof responsePlatform === 'string' && responsePlatform.toLowerCase().startsWith('linux')
      ? 'linux'
      : responsePlatform
  if (normalizedPlatform !== 'windows' && normalizedPlatform !== 'linux') {
    throw new Error('服务端返回的 Agent 平台无效，请重新生成启动包')
  }
  const configFilename = value.config_filename ?? value.configFilename ?? value.filename
  const expiresAt = value.expires_at ?? value.expiresAt
  return {
    platform: normalizedPlatform,
    downloadUrl,
    configFilename: typeof configFilename === 'string'
      ? configFilename
      : platform === 'windows' ? 'sculk-agent-windows-x86_64.json' : 'sculk-agent-linux-x86_64.json',
    configJson: value.config_json ?? value.configJson ?? bootstrapConfigFromResponse(value),
    expiresAt: typeof expiresAt === 'string' ? expiresAt : null,
  }
}

function bootstrapConfigFromResponse(value: Record<string, unknown>) {
  if (typeof value.cloud_url !== 'string' || typeof value.pairing_code !== 'string') return null
  // Agent 的 BootstrapConfig 使用 deny_unknown_fields；只下发它可读取的字段，
  // 不把配对记录 ID、有效期等控制台元数据写入配置文件。
  return {
    cloud_url: value.cloud_url,
    pairing_code: value.pairing_code,
    name: typeof value.name === 'string' ? value.name : 'mc-host',
    workspace_label: typeof value.workspace_label === 'string' ? value.workspace_label : 'minecraft',
    permissions: Array.isArray(value.permissions) ? value.permissions : [],
    capabilities: Array.isArray(value.capabilities) ? value.capabilities : [],
    workspace_root: typeof value.workspace_root === 'string' ? value.workspace_root : undefined,
    platform: typeof value.platform === 'string' ? value.platform : undefined,
    version: typeof value.version === 'string' ? value.version : undefined,
  }
}
