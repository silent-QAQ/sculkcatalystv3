import type { ReasoningEffort } from '../features/settings/types'

export interface ReasoningScaleOption {
  key: ReasoningEffort
}

export function reasoningEffortFromScale(index: number, options: readonly ReasoningScaleOption[]): ReasoningEffort | null {
  if (!Number.isInteger(index) || index <= 0) return null
  return options[index - 1]?.key ?? null
}

export function reasoningEffortToScale(effort: ReasoningEffort | null | undefined, options: readonly ReasoningScaleOption[]): number {
  if (!effort) return 0
  const index = options.findIndex(option => option.key === effort)
  return index < 0 ? 0 : index + 1
}
