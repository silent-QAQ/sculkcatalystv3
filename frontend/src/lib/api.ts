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

  let response: Response
  try {
    response = await fetch(apiUrl(path), { ...options, headers })
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') throw error
    throw new ApiError(0, '无法连接到后端服务，请确认服务正在运行后重试')
  }
  if (!response.ok) {
    const message = await response.text()
    throw new ApiError(response.status, message || `请求失败（HTTP ${response.status}）`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}
