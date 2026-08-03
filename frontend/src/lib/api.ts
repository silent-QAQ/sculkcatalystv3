export const API_BASE = (import.meta.env.VITE_API_BASE || '').replace(/\/$/, '')

export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

export function apiUrl(path: string) {
  return `${API_BASE}${path.startsWith('/') ? path : `/${path}`}`
}

export async function apiRequest<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers = new Headers(options.headers)
  if (options.body && !(options.body instanceof FormData) && !headers.has('Content-Type')) headers.set('Content-Type', 'application/json')

  const response = await fetch(apiUrl(path), { ...options, headers })
  if (!response.ok) {
    const message = await response.text()
    throw new ApiError(response.status, message || `请求失败（HTTP ${response.status}）`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}
