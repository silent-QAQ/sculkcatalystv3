<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

const STORAGE_KEY = 'sculk-chat-work-split-v1'
const DEFAULT_SPLIT_RATIO = 1 / 2.28
const MIN_CHAT_WIDTH = 320
const MIN_WORK_WIDTH = 400

interface SplitBounds {
  available: number
  min: number
  max: number
}

const grip = ref<HTMLElement | null>(null)
const app = ref<HTMLElement | null>(null)
const ratio = ref(DEFAULT_SPLIT_RATIO)
const activePointerId = ref<number | null>(null)
const dragging = ref(false)
const currentWidth = ref(0)
const currentBounds = ref<SplitBounds>({
  available: 0,
  min: MIN_CHAT_WIDTH,
  max: MIN_CHAT_WIDTH,
})

let observer: ResizeObserver | null = null

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max)
}

function layoutScale(root: HTMLElement, rect = root.getBoundingClientRect()) {
  return root.clientWidth > 0 ? rect.width / root.clientWidth : 1
}

function measureBounds(): SplitBounds | null {
  const root = app.value
  const chatPanel = root?.querySelector<HTMLElement>('.chat-panel')
  if (!root || !chatPanel) return null

  const rootRect = root.getBoundingClientRect()
  const chatRect = chatPanel.getBoundingClientRect()
  const available = Math.max(0, (rootRect.right - chatRect.left) / layoutScale(root, rootRect))
  const max = Math.max(0, available - MIN_WORK_WIDTH)

  return {
    available,
    min: Math.min(MIN_CHAT_WIDTH, max),
    max,
  }
}

function applyWidth(width: number, bounds: SplitBounds) {
  const nextWidth = Math.round(clamp(width, bounds.min, bounds.max))
  currentBounds.value = bounds
  currentWidth.value = nextWidth
  app.value?.style.setProperty('--chat-panel-width', `${nextWidth}px`)
}

function applyRatio() {
  const bounds = measureBounds()
  if (!bounds) return
  applyWidth(bounds.available * ratio.value, bounds)
}

function persistRatio() {
  try {
    window.localStorage.setItem(STORAGE_KEY, ratio.value.toFixed(5))
  } catch {
    // 浏览器拒绝本地存储时，宽度仍仅在当前页面有效。
  }
}

function setWidth(width: number, persist = false) {
  const bounds = measureBounds()
  if (!bounds || bounds.available <= 0) return
  const nextWidth = clamp(width, bounds.min, bounds.max)
  ratio.value = nextWidth / bounds.available
  applyWidth(nextWidth, bounds)
  if (persist) persistRatio()
}

function updateFromPointer(clientX: number) {
  const root = app.value
  const chatPanel = root?.querySelector<HTMLElement>('.chat-panel')
  if (!root || !chatPanel) return
  const rootRect = root.getBoundingClientRect()
  setWidth((clientX - chatPanel.getBoundingClientRect().left) / layoutScale(root, rootRect))
}

function startResize(event: PointerEvent) {
  if (event.button !== 0) return
  const control = grip.value
  if (!control) return

  event.preventDefault()
  activePointerId.value = event.pointerId
  dragging.value = true
  document.body.classList.add('workspace-resize-active')
  control.setPointerCapture(event.pointerId)
  updateFromPointer(event.clientX)
}

function moveResize(event: PointerEvent) {
  if (!dragging.value || event.pointerId !== activePointerId.value) return
  event.preventDefault()
  updateFromPointer(event.clientX)
}

function finishResize(event?: PointerEvent) {
  if (event && event.pointerId !== activePointerId.value) return
  // Some pointer implementations coalesce the final move into pointerup.
  // Apply that coordinate as well so the divider lands where it was released.
  if (event && dragging.value) updateFromPointer(event.clientX)
  const pointerId = activePointerId.value
  if (pointerId !== null && grip.value?.hasPointerCapture(pointerId)) {
    grip.value.releasePointerCapture(pointerId)
  }
  if (dragging.value) persistRatio()
  activePointerId.value = null
  dragging.value = false
  document.body.classList.remove('workspace-resize-active')
}

function resizeFromKeyboard(event: KeyboardEvent) {
  const bounds = measureBounds()
  if (!bounds) return
  const step = event.shiftKey ? 64 : 16
  let target: number | null = null

  if (event.key === 'ArrowLeft') target = currentWidth.value - step
  if (event.key === 'ArrowRight') target = currentWidth.value + step
  if (event.key === 'Home') target = bounds.min
  if (event.key === 'End') target = bounds.max
  if (target === null) return

  event.preventDefault()
  setWidth(target, true)
}

function resetWidth() {
  ratio.value = DEFAULT_SPLIT_RATIO
  try {
    window.localStorage.removeItem(STORAGE_KEY)
  } catch {
    // 本地存储不可用时无需额外处理。
  }
  applyRatio()
}

function restoreRatio() {
  try {
    const stored = Number(window.localStorage.getItem(STORAGE_KEY))
    if (Number.isFinite(stored) && stored > 0 && stored < 1) ratio.value = stored
  } catch {
    // 使用默认比例。
  }
}

onMounted(() => {
  app.value = grip.value?.closest<HTMLElement>('.app') ?? null
  if (!app.value) return

  app.value.classList.add('resizable-layout')
  restoreRatio()
  requestAnimationFrame(applyRatio)

  const sidebar = app.value.querySelector<HTMLElement>('.sidebar')
  observer = new ResizeObserver(applyRatio)
  observer.observe(app.value)
  if (sidebar) observer.observe(sidebar)
  window.addEventListener('resize', applyRatio)
})

onUnmounted(() => {
  finishResize()
  observer?.disconnect()
  window.removeEventListener('resize', applyRatio)
})
</script>

<template>
  <div
    ref="grip"
    class="workspace-resizer"
    :class="{ dragging }"
    role="separator"
    aria-orientation="vertical"
    aria-label="调整对话区与工作区宽度"
    :aria-valuemin="Math.round(currentBounds.min)"
    :aria-valuemax="Math.round(currentBounds.max)"
    :aria-valuenow="currentWidth"
    tabindex="0"
    title="拖动调整对话区与工作区宽度，双击恢复默认"
    @pointerdown="startResize"
    @pointermove="moveResize"
    @pointerup="finishResize"
    @pointercancel="finishResize"
    @lostpointercapture="finishResize"
    @keydown="resizeFromKeyboard"
    @dblclick.prevent="resetWidth"
  />
</template>
