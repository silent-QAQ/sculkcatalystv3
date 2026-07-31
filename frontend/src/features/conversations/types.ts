import type { ModelBinding, ReasoningEffort } from '../settings/types'

export interface ConversationMessage {
  id: string
  role: 'assistant' | 'user'
  content: string
  time: string
  actions?: string[]
  task_id?: string
}

export interface ConversationSummary {
  id: string
  server_id: string
  title: string
  group?: string | null
  pinned: boolean
  archived: boolean
  unread: boolean
  model_binding?: ModelBinding | null
  agent_override?: string | null
  reasoning_effort?: ReasoningEffort | null
  created_at: string
  updated_at: string
  message_count: number
}

export interface Conversation extends Omit<ConversationSummary, 'message_count'> {
  messages: ConversationMessage[]
}

export type ConversationAction =
  | 'rename'
  | 'group'
  | 'pin'
  | 'archive'
  | 'unread'
  | 'fork'
  | 'delete'
