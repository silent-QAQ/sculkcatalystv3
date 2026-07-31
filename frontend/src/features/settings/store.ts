import { ref } from 'vue'
import { apiRequest } from '../../lib/api'
import { applyAppearance } from '../../lib/appearance'
import type { AiSettingsView, UiSettings } from './types'

export const aiSettings = ref<AiSettingsView | null>(null)
export const uiSettings = ref<UiSettings | null>(null)
export const notice = ref('')
const UI_CACHE_KEY = 'sculk-cloud-ui-v2'

export function flash(message: string) {
  notice.value = message
  window.setTimeout(() => { if (notice.value === message) notice.value = '' }, 2600)
}

export function friendly(error: unknown) {
  return String(error instanceof Error ? error.message : error).slice(0, 180)
}

export async function loadAi() {
  aiSettings.value = await apiRequest<AiSettingsView>('/api/ai/settings')
}

export async function loadUi() {
  try {
    applyCloudUi(await apiRequest<UiSettings>('/api/ui/settings'))
  } catch (error) {
    const cached = localStorage.getItem(UI_CACHE_KEY)
    if (!cached) throw error
    try {
      applyCloudUi(JSON.parse(cached) as UiSettings)
    } catch {
      localStorage.removeItem(UI_CACHE_KEY)
      throw error
    }
  }
}

export function applyCloudUi(settings: UiSettings) {
  uiSettings.value = settings
  localStorage.setItem(UI_CACHE_KEY, JSON.stringify(settings))
  applyAppearance(settings.appearance)
}

export async function loadAll() {
  const results = await Promise.allSettled([loadAi(), loadUi()])
  const failed = results.find(result => result.status === 'rejected')
  if (failed) flash('设置加载失败：' + friendly((failed as PromiseRejectedResult).reason))
}

/** 部分更新 UI 偏好并同步生效（外观改动立即应用到页面）。 */
export async function saveUi(patch: Partial<UiSettings>, message = '设置已保存') {
  try {
    applyCloudUi(await apiRequest<UiSettings>('/api/ui/settings', { method: 'PUT', body: JSON.stringify(patch) }))
    if (message) flash(message)
    return true
  } catch (error) {
    flash('保存失败：' + friendly(error))
    return false
  }
}
