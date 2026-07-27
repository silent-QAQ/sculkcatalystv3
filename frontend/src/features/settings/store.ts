import { ref } from 'vue'
import { apiRequest } from '../../lib/api'
import { applyAppearance } from '../../lib/appearance'
import type { AiSettingsView, UiSettings } from './types'

export const aiSettings = ref<AiSettingsView | null>(null)
export const uiSettings = ref<UiSettings | null>(null)
export const notice = ref('')

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
  uiSettings.value = await apiRequest<UiSettings>('/api/ui/settings')
  applyAppearance(uiSettings.value.appearance)
}

export async function loadAll() {
  const results = await Promise.allSettled([loadAi(), loadUi()])
  const failed = results.find(result => result.status === 'rejected')
  if (failed) flash('设置加载失败：' + friendly((failed as PromiseRejectedResult).reason))
}

/** 部分更新 UI 偏好并同步生效（外观改动立即应用到页面）。 */
export async function saveUi(patch: Partial<UiSettings>, message = '设置已保存') {
  try {
    uiSettings.value = await apiRequest<UiSettings>('/api/ui/settings', { method: 'PUT', body: JSON.stringify(patch) })
    applyAppearance(uiSettings.value.appearance)
    if (message) flash(message)
    return true
  } catch (error) {
    flash('保存失败：' + friendly(error))
    return false
  }
}
