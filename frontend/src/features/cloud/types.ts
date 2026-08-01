// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

import type { AiSettingsView, UiSettings } from '../settings/types'
import type { ServerTemplate } from '../portable/server-manifest'

export interface CloudStatus {
  available: boolean
  message: string
  features: string[]
}

export interface CloudProfile {
  id: string
  email: string
  nickname: string
  avatar_url: string
  role: 'user' | 'admin'
  plan: 'free' | 'pro' | 'team'
  locale: string
  created_at: string
}

export interface AuthResponse {
  access_token: string
  expires_at: string
  profile: CloudProfile
}

export interface CloudDevice {
  id: string
  name: string
  platform: string
  last_seen_at: string
  created_at: string
  current: boolean
}

export interface CloudAgent {
  id: string
  name: string
  platform: string
  version: string
  workspace_label: string
  capabilities: string[]
  permissions: Array<'read' | 'write' | 'process' | 'full'>
  fingerprint: string
  status: 'claimed' | 'active' | 'revoked'
  last_seen_at?: string | null
  online: boolean
  claimed_at: string
  confirmed_at?: string | null
  revoked_at?: string | null
}

export interface AgentPairingCreated {
  id: string
  pairing_code: string
  expires_at: string
  status: 'pending'
}

export type AgentTaskOperation =
  | 'host.inspect'
  | 'workspace.list'
  | 'log.tail'
  | 'workspace.create_directory'
  | 'server.properties.update'
  | 'shell.exec'
  | 'task.rollback'

export type AgentTaskStatus =
  | 'awaiting_approval'
  | 'queued'
  | 'leased'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled'

export interface AgentTaskEvent {
  seq: number
  level: 'info' | 'warn' | 'error'
  message: string
  data?: unknown | null
  created_at: string
}

export interface AgentTaskArtifact {
  name: string
  path: string
  kind: 'file' | 'directory' | 'backup' | 'log'
  size_bytes?: number
  sha256?: string
}

export interface AgentTaskCheckpoint {
  id: string
  seq: number
  kind: 'progress' | 'result'
  resumable: boolean
  created_at: string
}

export interface AgentTaskView {
  id: string
  lineage_id: string
  attempt_no: number
  agent_id: string
  team_id?: string | null
  approval_id?: string | null
  operation: AgentTaskOperation
  required_permission: 'read' | 'write' | 'process' | 'full'
  risk: 'low' | 'high' | 'critical'
  input: Record<string, unknown>
  status: AgentTaskStatus
  idempotency_key?: string | null
  source_task_id?: string | null
  retry_of_task_id?: string | null
  execution_mode: 'original' | 'restart' | 'resume'
  resume_checkpoint_id?: string | null
  rollback_source_task_id?: string | null
  latest_checkpoint?: AgentTaskCheckpoint | null
  can_resume: boolean
  approved_by?: string | null
  approved_at?: string | null
  lease_expires_at?: string | null
  leased_at?: string | null
  started_at?: string | null
  completed_at?: string | null
  cancelled_at?: string | null
  cancel_requested_at?: string | null
  cancel_requested_by?: string | null
  cancel_acknowledged_at?: string | null
  cancel_requested: boolean
  output?: unknown | null
  error?: string | null
  artifacts?: AgentTaskArtifact[] | null
  rollback_available: boolean
  created_at: string
  updated_at: string
  events: AgentTaskEvent[]
}

export interface CloudTerminalSession {
  id: string
  agent_id: string
  team_id?: string | null
  approval_id?: string | null
  title?: string | null
  cwd?: string | null
  cols: number
  rows: number
  status: string
  exit_code?: number | null
  error?: string | null
  created_at: string
  approved_at?: string | null
  started_at?: string | null
  last_seen_at?: string | null
  exited_at?: string | null
  updated_at: string
}

export interface CloudTerminalEvent {
  seq: number
  kind: string
  data_base64?: string | null
  data?: unknown | null
  created_at: string
}

export interface CloudConversation {
  id: string
  title: string
  agent_id?: string | null
  created_at: string
  updated_at: string
}

export interface CloudConversationMessage {
  id: string
  role: 'user' | 'assistant' | 'system'
  content: string
  kind: 'text' | 'plan' | 'system'
  linked_task_id?: string | null
  created_at: string
}

export interface SyncedSettings {
  version: number
  payload: Record<string, unknown>
  updated_at: string
  updated_by_device?: string | null
}

export interface CloudPrompt {
  id: string
  title: string
  content: string
}

export interface CloudSkillLink {
  id: string
  name: string
  url: string
  enabled: boolean
}

export interface CloudAiSettings {
  providers: Array<{
    id: string
    name: string
    base_url: string
    enabled: boolean
    models: AiSettingsView['providers'][number]['models']
    models_synced_at?: string | null
    has_credential: boolean
  }>
  scenarios: AiSettingsView['scenarios']
  default_binding?: AiSettingsView['default_binding']
  review_mode: AiSettingsView['review_mode']
  agents: Array<Pick<AiSettingsView['agents'][number], 'id' | 'name' | 'kind' | 'enabled'>>
  active_agent?: AiSettingsView['active_agent']
}

export interface CloudWorkspacePayload extends Record<string, unknown> {
  schema_version: 2 | 3
  ui?: UiSettings
  prompts: CloudPrompt[]
  skill_links: CloudSkillLink[]
  server_templates?: ServerTemplate[]
  ai?: CloudAiSettings
}

export interface CloudCredential {
  id: string
  name: string
  base_url: string
  api_key_masked: string
  fingerprint: string
  created_at: string
  updated_at: string
}

export interface CloudTeam {
  id: string
  name: string
  slug: string
  role: 'owner' | 'admin' | 'approver' | 'member'
  member_count: number
  pending_approvals: number
  created_at: string
}

export interface TeamMember {
  id: string
  email: string
  nickname: string
  avatar_url: string
  role: CloudTeam['role']
  joined_at: string
}

export interface Invitation {
  id: string
  email: string
  role: string
  invite_code: string
  expires_at: string
}

export interface CloudApproval {
  id: string
  team_id: string
  agent_task_id?: string | null
  terminal_session_id?: string | null
  team_name: string
  requested_by: string
  requester_name: string
  title: string
  summary: string
  risk: 'low' | 'medium' | 'high'
  status: 'pending' | 'approved' | 'rejected' | 'cancelled'
  payload: Record<string, unknown>
  decision_comment: string
  decided_by?: string | null
  decided_by_name?: string | null
  decided_at?: string | null
  created_at: string
}

export interface ApiTokenItem {
  id: string
  label: string
  token_prefix: string
  last_used_at?: string | null
  expires_at?: string | null
  created_at: string
  total_tokens: number
  request_count: number
}

export interface ApiTokenCreated {
  token: string
  item: ApiTokenItem
}

export interface UsageDay {
  day: string
  requests: number
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
}

export interface UsageSummary {
  days: number
  requests: number
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
  daily: UsageDay[]
}

export interface RelayProvider {
  configured: boolean
  name: string
  base_url: string
  api_key_masked: string
  default_model: string
  enabled: boolean
  updated_at?: string | null
}

export interface DeploymentCapability {
  available: boolean
  status: 'planned'
  api_version: string
  reserved_endpoints: string[]
}
