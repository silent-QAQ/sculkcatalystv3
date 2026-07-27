import { apiUrl } from './api'

export interface ChatMetaPayload {
  provider: string | null
  model: string | null
  fallback: boolean
}

export interface ChatDonePayload {
  id: string
  time: string
  actions: string[]
  task: unknown | null
}

export interface SseHandlers {
  onMeta?: (meta: ChatMetaPayload) => void
  onDelta: (text: string) => void
  onDone: (done: ChatDonePayload) => void
  onError?: (message: string) => void
}

/** POST 一个 JSON body 并消费 SSE 响应流。EventSource 仅支持 GET，故用 fetch + ReadableStream。 */
export async function postSse(path: string, body: unknown, handlers: SseHandlers, signal?: AbortSignal): Promise<void> {
  const response = await fetch(apiUrl(path), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  })
  if (!response.ok || !response.body) throw new Error(await response.text().catch(() => '') || `请求失败（HTTP ${response.status}）`)
  const reader = response.body.getReader()
  const decoder = new TextDecoder('utf-8')
  let buffer = ''
  const dispatch = (block: string) => {
    let event = 'message'
    const dataLines: string[] = []
    for (const line of block.split('\n')) {
      if (line.startsWith('event:')) event = line.slice(6).trim()
      else if (line.startsWith('data:')) dataLines.push(line.slice(5).trimStart())
    }
    if (!dataLines.length) return
    let payload: any
    try { payload = JSON.parse(dataLines.join('\n')) } catch { return }
    if (event === 'meta') handlers.onMeta?.(payload)
    else if (event === 'delta') handlers.onDelta(String(payload.content ?? ''))
    else if (event === 'done') handlers.onDone(payload)
    else if (event === 'error') handlers.onError?.(String(payload.message ?? '模型响应中断'))
  }
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })
    let cut: number
    while ((cut = buffer.indexOf('\n\n')) >= 0) {
      const block = buffer.slice(0, cut)
      buffer = buffer.slice(cut + 2)
      if (block.trim()) dispatch(block)
    }
  }
  if (buffer.trim()) dispatch(buffer)
}
