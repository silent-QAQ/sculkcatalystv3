// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

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

export interface SyncedSettings {
  version: number
  payload: Record<string, unknown>
  updated_at: string
  updated_by_device?: string | null
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
  team_name: string
  requested_by: string
  requester_name: string
  title: string
  summary: string
  risk: 'low' | 'medium' | 'high'
  status: 'pending' | 'approved' | 'rejected' | 'cancelled'
  payload: Record<string, unknown>
  decision_comment: string
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
