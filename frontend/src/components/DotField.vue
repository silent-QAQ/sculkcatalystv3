<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'

interface Dot {
  anchorX: number
  anchorY: number
  softX: number
  softY: number
  velocityX: number
  velocityY: number
  x: number
  y: number
}

const props = withDefaults(defineProps<{
  dotRadius?: number
  dotSpacing?: number
  cursorRadius?: number
  cursorForce?: number
  bulgeOnly?: boolean
  bulgeStrength?: number
  glowRadius?: number
  sparkle?: boolean
  waveAmplitude?: number
  gradientFrom?: string
  gradientTo?: string
  glowColor?: string
}>(), {
  dotRadius: 1.4,
  dotSpacing: 17,
  cursorRadius: 420,
  cursorForce: 0.1,
  bulgeOnly: true,
  bulgeStrength: 30,
  glowRadius: 170,
  sparkle: false,
  waveAmplitude: 0,
  gradientFrom: 'rgba(105, 238, 199, 0.24)',
  gradientTo: 'rgba(174, 155, 255, 0.17)',
  glowColor: '#07100f',
})

const rootRef = ref<HTMLElement | null>(null)
const canvasRef = ref<HTMLCanvasElement | null>(null)
const glowRef = ref<SVGCircleElement | null>(null)
const glowId = `dot-field-glow-${Math.random().toString(36).slice(2, 9)}`

const dots: Dot[] = []
const mouse = { x: -9_999, y: -9_999, previousX: -9_999, previousY: -9_999, speed: 0 }
let width = 0
let height = 0
let context: CanvasRenderingContext2D | null = null
let animationFrame = 0
let resizeObserver: ResizeObserver | null = null
let frame = 0
let engagement = 0
let glowOpacity = 0
let reducedMotion = false

function buildDots(nextWidth: number, nextHeight: number) {
  const step = props.dotRadius + props.dotSpacing
  const columns = Math.floor(nextWidth / step)
  const rows = Math.floor(nextHeight / step)
  const paddingX = (nextWidth % step) / 2
  const paddingY = (nextHeight % step) / 2

  dots.length = 0
  for (let row = 0; row < rows; row += 1) {
    for (let column = 0; column < columns; column += 1) {
      const anchorX = paddingX + column * step + step / 2
      const anchorY = paddingY + row * step + step / 2
      dots.push({
        anchorX,
        anchorY,
        softX: anchorX,
        softY: anchorY,
        velocityX: 0,
        velocityY: 0,
        x: anchorX,
        y: anchorY,
      })
    }
  }
}

function resize() {
  const root = rootRef.value
  const canvas = canvasRef.value
  if (!root || !canvas) return

  const bounds = root.getBoundingClientRect()
  width = Math.max(1, Math.round(bounds.width))
  height = Math.max(1, Math.round(bounds.height))
  const pixelRatio = Math.min(window.devicePixelRatio || 1, 2)
  canvas.width = Math.round(width * pixelRatio)
  canvas.height = Math.round(height * pixelRatio)
  context = canvas.getContext('2d', { alpha: true })
  context?.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0)
  buildDots(width, height)
  render()
}

function updatePointer(event: PointerEvent) {
  const bounds = rootRef.value?.getBoundingClientRect()
  if (!bounds) return
  mouse.x = event.clientX - bounds.left
  mouse.y = event.clientY - bounds.top
}

function render() {
  if (!context || !width || !height) return
  const drawingContext = context
  frame += 1

  const deltaX = mouse.previousX - mouse.x
  const deltaY = mouse.previousY - mouse.y
  const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY)
  mouse.speed += (distance - mouse.speed) * 0.5
  if (mouse.speed < 0.001) mouse.speed = 0
  mouse.previousX = mouse.x
  mouse.previousY = mouse.y

  const targetEngagement = Math.min(mouse.speed / 5, 1)
  engagement += (targetEngagement - engagement) * 0.06
  if (engagement < 0.001) engagement = 0
  glowOpacity += (engagement - glowOpacity) * 0.08

  if (glowRef.value) {
    glowRef.value.setAttribute('cx', String(mouse.x))
    glowRef.value.setAttribute('cy', String(mouse.y))
    glowRef.value.style.opacity = String(glowOpacity)
  }

  drawingContext.clearRect(0, 0, width, height)
  const gradient = drawingContext.createLinearGradient(0, 0, width, height)
  gradient.addColorStop(0, props.gradientFrom)
  gradient.addColorStop(1, props.gradientTo)
  drawingContext.fillStyle = gradient
  drawingContext.beginPath()

  const cursorRadiusSquared = props.cursorRadius * props.cursorRadius
  const radius = props.dotRadius / 2
  const waveTime = frame * 0.02

  dots.forEach((dot, index) => {
    const deltaPointerX = mouse.x - dot.anchorX
    const deltaPointerY = mouse.y - dot.anchorY
    const distanceSquared = deltaPointerX * deltaPointerX + deltaPointerY * deltaPointerY

    if (distanceSquared < cursorRadiusSquared && engagement > 0.01) {
      const distanceToPointer = Math.sqrt(distanceSquared)
      const angle = Math.atan2(deltaPointerY, deltaPointerX)
      if (props.bulgeOnly) {
        const strength = 1 - distanceToPointer / props.cursorRadius
        const push = strength * strength * props.bulgeStrength * engagement
        dot.softX += (dot.anchorX - Math.cos(angle) * push - dot.softX) * 0.15
        dot.softY += (dot.anchorY - Math.sin(angle) * push - dot.softY) * 0.15
      } else {
        const move = (500 / Math.max(distanceToPointer, 1)) * (mouse.speed * props.cursorForce)
        dot.velocityX += Math.cos(angle) * -move
        dot.velocityY += Math.sin(angle) * -move
      }
    } else if (props.bulgeOnly) {
      dot.softX += (dot.anchorX - dot.softX) * 0.1
      dot.softY += (dot.anchorY - dot.softY) * 0.1
    }

    if (!props.bulgeOnly) {
      dot.velocityX *= 0.9
      dot.velocityY *= 0.9
      dot.x = dot.anchorX + dot.velocityX
      dot.y = dot.anchorY + dot.velocityY
      dot.softX += (dot.x - dot.softX) * 0.1
      dot.softY += (dot.y - dot.softY) * 0.1
    }

    let drawX = dot.softX
    let drawY = dot.softY
    if (props.waveAmplitude > 0) {
      drawY += Math.sin(dot.anchorX * 0.03 + waveTime) * props.waveAmplitude
      drawX += Math.cos(dot.anchorY * 0.03 + waveTime * 0.7) * props.waveAmplitude * 0.5
    }

    const sparkle = props.sparkle && (((index * 2_654_435_761) ^ (frame >> 3)) >>> 0) % 100 < 3
    const dotRadius = sparkle ? radius * 1.8 : radius
    drawingContext.moveTo(drawX + dotRadius, drawY)
    drawingContext.arc(drawX, drawY, dotRadius, 0, Math.PI * 2)
  })

  drawingContext.fill()
}

function tick() {
  render()
  if (!reducedMotion) animationFrame = window.requestAnimationFrame(tick)
}

onMounted(() => {
  reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  resizeObserver = new ResizeObserver(resize)
  if (rootRef.value) resizeObserver.observe(rootRef.value)
  window.addEventListener('pointermove', updatePointer, { passive: true })
  resize()
  tick()
})

onUnmounted(() => {
  window.cancelAnimationFrame(animationFrame)
  resizeObserver?.disconnect()
  window.removeEventListener('pointermove', updatePointer)
})

watch(() => [props.dotRadius, props.dotSpacing], resize)
</script>

<template>
  <div ref="rootRef" class="dot-field-container" aria-hidden="true">
    <canvas ref="canvasRef"/>
    <svg>
      <defs>
        <radialGradient :id="glowId">
          <stop offset="0%" :stop-color="glowColor"/>
          <stop offset="100%" stop-color="transparent"/>
        </radialGradient>
      </defs>
      <circle ref="glowRef" cx="-9999" cy="-9999" :r="glowRadius" :fill="`url(#${glowId})`"/>
    </svg>
  </div>
</template>

<style scoped>
.dot-field-container{position:absolute;inset:0;overflow:hidden;pointer-events:none}
.dot-field-container canvas,.dot-field-container svg{position:absolute;inset:0;width:100%;height:100%}
.dot-field-container svg{pointer-events:none}
</style>
