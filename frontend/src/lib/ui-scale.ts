export interface UiScaleMetrics {
  scale: number
  inversePercent: string
}

const MIN_UI_SCALE = 0.7
const MAX_UI_SCALE = 1.5

/**
 * Keep the scaled application canvas inside the physical viewport. The canvas
 * gets an inverse logical size before it is visually enlarged.
 */
export function resolveUiScale(fontSize = 100): UiScaleMetrics {
  const requestedScale = Number.isFinite(fontSize) ? fontSize / 100 : 1
  const scale = Math.min(MAX_UI_SCALE, Math.max(MIN_UI_SCALE, requestedScale))

  return {
    scale,
    inversePercent: `${100 / scale}%`,
  }
}
