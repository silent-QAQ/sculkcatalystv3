export type TaskStatus =
  | 'awaiting_approval'
  | 'queued'
  | 'running'
  | 'cancelling'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'interrupted'
  | 'rollback_failed'

export type TaskRisk = 'low' | 'medium' | 'high'

export type TaskKind =
  | 'server_provision'
  | 'bootstrap'
  | 'download'
  | 'core_download'
  | 'diagnostic'
  | 'server_start'
  | 'server_stop'
  | 'rollback_server_state'

export interface TaskEvent {
  at: string
  level: 'info' | 'warn' | 'error' | string
  message: string
}

export interface TaskArtifact {
  id: string
  name: string
  kind: TaskKind | string
  size: number
  created_at: string
  relative_path: string
}

export interface TaskRollback {
  status: 'prepared' | 'available' | 'scheduled' | 'planned' | 'completed' | 'failed' | string
  previous_server_status: string
  summary?: string | null
}

export interface TaskInfo {
  id: string
  server_id: string
  title: string
  kind: string
  status: TaskStatus | string
  progress: number
  created_at: string
  updated_at?: string
  risk: TaskRisk | string
  approved_by?: string | null
  started_at?: string | null
  finished_at?: string | null
  summary?: string | null
  error?: string | null
  events?: TaskEvent[]
  artifacts?: TaskArtifact[]
  rollback?: TaskRollback | null
  parent_task_id?: string | null
}

export const TASK_STATUS_LABELS: Record<string, string> = {
  awaiting_approval: '等待批准',
  queued: '等待执行器',
  running: '执行中',
  cancelling: '正在取消',
  completed: '已完成',
  failed: '执行失败',
  cancelled: '已取消',
  interrupted: '执行中断',
  rollback_failed: '回滚失败',
  pending: '待处理',
}

export const TERMINAL_TASK_STATUSES = new Set(['completed', 'failed', 'cancelled', 'interrupted', 'rollback_failed'])

export function taskCanApprove(task: TaskInfo) {
  return task.status === 'awaiting_approval'
}

export function taskCanCancel(task: TaskInfo) {
  return ['awaiting_approval', 'queued', 'running'].includes(task.status)
}

export function taskCanRollback(task: TaskInfo) {
  return task.status === 'completed' && task.rollback?.status === 'available'
}
