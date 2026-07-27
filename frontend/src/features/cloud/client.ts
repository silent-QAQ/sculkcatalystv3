import { apiUrl } from '../../lib/api'

const TOKEN_KEY = 'sculk-cloud-session'

export function cloudSession() {
  return window.localStorage.getItem(TOKEN_KEY) || ''
}

export function setCloudSession(token: string) {
  if (token) window.localStorage.setItem(TOKEN_KEY, token)
  else window.localStorage.removeItem(TOKEN_KEY)
}

export async function cloudRequest<T>(path: string, options: RequestInit = {}, authenticated = true): Promise<T> {
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
    try {
      const parsed = JSON.parse(raw) as { error?: { message?: string } }
      message = parsed.error?.message || message
    } catch {}
    if (response.status === 401 && authenticated) setCloudSession('')
    throw new Error(message)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}
