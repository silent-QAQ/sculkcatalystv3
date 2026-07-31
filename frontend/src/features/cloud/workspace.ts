// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

import { ref } from 'vue'
import type { AiSettingsView, UiSettings } from '../settings/types'
import type { CloudPrompt, CloudSkillLink, CloudWorkspacePayload } from './types'
import { normalizeStoredServerTemplate, type ServerTemplate } from '../portable/server-manifest'

const WORKSPACE_CACHE_KEY = 'sculk-cloud-workspace-v3'
const LEGACY_WORKSPACE_CACHE_KEY = 'sculk-cloud-workspace-v2'

const defaultPrompts: CloudPrompt[] = [
  { id: 'repair', title: '修复报错', content: '分析最新报错并自动修复' },
  { id: 'vote', title: '发起投票', content: '为新玩法发起玩家投票' },
  { id: 'promotion', title: '宣传文案', content: '生成本周服务器宣传文案' },
]

export const cloudPrompts = ref<CloudPrompt[]>([...defaultPrompts])
export const cloudSkillLinks = ref<CloudSkillLink[]>([])
export const cloudServerTemplates = ref<ServerTemplate[]>([])
let cachedWorkspace: CloudWorkspacePayload | null = null

function validPrompt(value: unknown): value is CloudPrompt {
  const item = value as Partial<CloudPrompt>
  return !!item && typeof item.id === 'string' && typeof item.title === 'string' && typeof item.content === 'string'
}

function validSkillLink(value: unknown): value is CloudSkillLink {
  const item = value as Partial<CloudSkillLink>
  return !!item && typeof item.id === 'string' && typeof item.name === 'string' && typeof item.url === 'string' && typeof item.enabled === 'boolean'
}

function cacheWorkspace(payload: CloudWorkspacePayload) {
  localStorage.setItem(WORKSPACE_CACHE_KEY, JSON.stringify(payload))
}

export function loadWorkspaceCache() {
  try {
    const raw = localStorage.getItem(WORKSPACE_CACHE_KEY) || localStorage.getItem(LEGACY_WORKSPACE_CACHE_KEY)
    if (!raw) return
    const payload = JSON.parse(raw) as Partial<CloudWorkspacePayload>
    if (Array.isArray(payload.prompts)) cloudPrompts.value = payload.prompts.filter(validPrompt)
    if (Array.isArray(payload.skill_links)) cloudSkillLinks.value = payload.skill_links.filter(validSkillLink)
    if (Array.isArray(payload.server_templates)) {
      cloudServerTemplates.value = payload.server_templates.map(normalizeStoredServerTemplate).filter(item => item !== null)
    }
    if (payload.schema_version === 2 || payload.schema_version === 3) cachedWorkspace = payload as CloudWorkspacePayload
  } catch {
    localStorage.removeItem(WORKSPACE_CACHE_KEY)
  }
}

export function isCloudWorkspace(value: Record<string, unknown>): value is CloudWorkspacePayload {
  return (value.schema_version === 2 || value.schema_version === 3) && Array.isArray(value.prompts) && Array.isArray(value.skill_links)
}

export function snapshotCloudWorkspace(ui: UiSettings | null, ai: AiSettingsView | null): CloudWorkspacePayload {
  return {
    schema_version: 3,
    ...(ui ? { ui } : {}),
    prompts: cloudPrompts.value,
    skill_links: cloudSkillLinks.value,
    server_templates: cloudServerTemplates.value,
    ...(ai ? {
      ai: {
        providers: ai.providers.map(provider => ({
          id: provider.id,
          name: provider.name,
          base_url: endpointWithoutCredentials(provider.base_url),
          enabled: provider.enabled,
          models: provider.models,
          models_synced_at: provider.models_synced_at,
          has_credential: provider.has_key,
        })),
        scenarios: ai.scenarios,
        default_binding: ai.default_binding,
        review_mode: ai.review_mode,
        agents: ai.agents.map(agent => ({ id: agent.id, name: agent.name, kind: agent.kind, enabled: agent.enabled })),
        active_agent: ai.active_agent,
      },
    } : {}),
  }
}

function endpointWithoutCredentials(value: string) {
  try {
    const endpoint = new URL(value)
    endpoint.username = ''
    endpoint.password = ''
    return endpoint.toString().replace(/\/$/, '')
  } catch {
    return ''
  }
}

export function applyCloudWorkspace(payload: CloudWorkspacePayload): UiSettings | undefined {
  cloudPrompts.value = payload.prompts.filter(validPrompt)
  cloudSkillLinks.value = payload.skill_links.filter(validSkillLink)
  cloudServerTemplates.value = (payload.server_templates || []).map(normalizeStoredServerTemplate).filter(item => item !== null)
  cachedWorkspace = payload
  cacheWorkspace(payload)
  return payload.ui
}

export function saveCloudPrompts(items: CloudPrompt[]) {
  cloudPrompts.value = items
  persistWorkspaceLists()
}

export function saveCloudSkillLinks(items: CloudSkillLink[]) {
  cloudSkillLinks.value = items
  persistWorkspaceLists()
}

export function saveCloudServerTemplates(items: ServerTemplate[]) {
  cloudServerTemplates.value = items.slice(0, 50)
  persistWorkspaceLists()
}

function persistWorkspaceLists() {
  cachedWorkspace = {
    ...(cachedWorkspace || {}),
    schema_version: 3,
    prompts: cloudPrompts.value,
    skill_links: cloudSkillLinks.value,
    server_templates: cloudServerTemplates.value,
  }
  cacheWorkspace(cachedWorkspace)
}

loadWorkspaceCache()
