import type { AppearanceSettings, BackgroundSettings } from '../features/settings/types'
import { resolveUiScale } from './ui-scale'

export interface PresetBundle {
  key: string
  label: string
  hint: string
  accent: string
  bg: string
  panel: string
  background: BackgroundSettings
  font_family: string
  font_size: number
  font_color: string
  card_blur: number
}

/** 预设 = 强调色 + 背景 + 字体的完整组合；选择预设会整体应用。 */
export const PRESETS: PresetBundle[] = [
  {
    key: 'sculk', label: '星雾蓝', hint: '网格渐变 · Inter 130% · 冰蓝',
    accent: '#5cb3ff', bg: '#0b0e12', panel: '#10141a',
    background: { mode: 'gradient', solid: '#e4c8e1', gradient: 'mesh', gradient_colors: ['#f9a9d0', '#a8e5ff', '#5cb3ff'], image_url: '', image_opacity: 72, image_position_x: 50, image_position_y: 50, image_scale: 100 },
    font_family: 'default', font_size: 130, font_color: '#85baff', card_blur: 6,
  },
  {
    key: 'amethyst', label: '紫水晶', hint: '星云渐变 · 衬线 105% · 紫罗兰',
    accent: '#9c8cff', bg: '#0d0c14', panel: '#151222',
    background: { mode: 'gradient', solid: '#0d0c14', gradient: 'mesh', gradient_colors: ['#2b1752', '#0b0e12', '#123d36', '#17102c'], image_url: '', image_opacity: 72, image_position_x: 50, image_position_y: 50, image_scale: 100 },
    font_family: 'serif', font_size: 105, font_color: '#eae8f4', card_blur: 18,
  },
  {
    key: 'ember', label: '余烬橙', hint: '熔岩渐变 · 系统字体 100% · 暖橙',
    accent: '#f3a75c', bg: '#120e0b', panel: '#1a140f',
    background: { mode: 'gradient', solid: '#120e0b', gradient: 'diagonal', gradient_colors: ['#3b1609', '#130d13', '#321027'], image_url: '', image_opacity: 72, image_position_x: 50, image_position_y: 50, image_scale: 100 },
    font_family: 'system', font_size: 100, font_color: '#f0ebe6', card_blur: 18,
  },
  {
    key: 'azure', label: '冰川蓝', hint: '深海渐变 · 等宽 95% · 冷蓝',
    accent: '#5cb3ff', bg: '#0a0e15', panel: '#0f1620',
    background: { mode: 'gradient', solid: '#0a0e15', gradient: 'radial', gradient_colors: ['#07345c', '#09131f', '#06353d'], image_url: '', image_opacity: 72, image_position_x: 50, image_position_y: 50, image_scale: 100 },
    font_family: 'mono', font_size: 95, font_color: '#e7edf5', card_blur: 18,
  },
]

export const CUSTOM_PRESET_KEY = 'custom'
export const MENU_FONT_INHERIT_KEY = 'inherit'
export const DEFAULT_MENU_FONT_COLOR = '#929ca9'

export const GRADIENTS: { key: string; label: string; hint: string }[] = [
  { key: 'diagonal', label: '对角流光', hint: '颜色沿 135° 平滑过渡' },
  { key: 'vertical', label: '垂直沉浸', hint: '从顶部向底部依次融合' },
  { key: 'radial', label: '聚光晕染', hint: '颜色从中心向四周扩散' },
  { key: 'mesh', label: '网格融合', hint: '多个彩色光团交叠混合' },
  { key: 'conic', label: '环流渐变', hint: '颜色围绕中心旋转衔接' },
]

function normalizedColors(colors: string[] | undefined) {
  const valid = (colors ?? []).filter(color => /^#[0-9a-f]{6}$/i.test(color)).slice(0, 5)
  if (valid.length >= 2) return valid
  return ['#f9a9d0', '#a8e5ff', '#5cb3ff']
}

function colorStops(colors: string[]) {
  return colors.map((color, index) => `${color} ${Math.round(index * 100 / (colors.length - 1))}%`).join(',')
}

/** 根据用户颜色列表和几何方案生成最终背景，不再内置固定灰色滤镜。 */
export function buildGradient(scheme: string, inputColors: string[]) {
  const colors = normalizedColors(inputColors)
  const stops = colorStops(colors)
  if (scheme === 'vertical') return `linear-gradient(180deg,${stops})`
  if (scheme === 'radial') return `radial-gradient(circle at 50% 42%,${stops})`
  if (scheme === 'conic') {
    const loop = [...colors, colors[0]]
      .map((color, index) => `${color} ${Math.round(index * 100 / colors.length)}%`)
      .join(',')
    return `conic-gradient(from 215deg at 50% 48%,${loop})`
  }
  if (scheme === 'mesh') {
    const positions = ['16% 18%', '84% 20%', '76% 84%', '22% 78%', '50% 48%']
    const lights = colors.map((color, index) => `radial-gradient(circle at ${positions[index]},${color} 0%,transparent 58%)`)
    return [...lights, `linear-gradient(${colors[colors.length - 1]},${colors[colors.length - 1]})`].join(',')
  }
  return `linear-gradient(135deg,${stops})`
}

export const FONTS: { key: string; label: string; css: string }[] = [
  { key: 'default', label: '默认（Inter / 思源黑体）', css: "Inter,'Noto Sans SC',system-ui,sans-serif" },
  { key: 'system', label: '系统字体', css: "system-ui,'Segoe UI','Microsoft YaHei',sans-serif" },
  { key: 'serif', label: '衬线', css: "Georgia,'Noto Serif SC','Songti SC',serif" },
  { key: 'mono', label: '等宽（Cascadia Code）', css: "'Cascadia Code',Consolas,'Noto Sans SC',monospace" },
]

/** 预设的完整外观值（用于「选择预设 = 应用整套组合」）。 */
export function presetAppearance(key: string): AppearanceSettings {
  const preset = PRESETS.find(item => item.key === key) ?? PRESETS[0]
  return {
    preset: preset.key,
    accent: preset.accent,
    background: { ...preset.background, gradient_colors: [...preset.background.gradient_colors] },
    font_family: preset.font_family,
    font_size: preset.font_size,
    font_color: preset.font_color,
    menu_font_family: MENU_FONT_INHERIT_KEY,
    menu_font_color: '',
    card_blur: preset.card_blur,
  }
}

/** 补齐旧版云端状态与本地缓存中缺少的外观字段。 */
export function normalizeAppearance(input: Partial<AppearanceSettings> | null | undefined): AppearanceSettings {
  const base = presetAppearance('sculk')
  return {
    ...base,
    ...(input ?? {}),
    background: { ...base.background, ...(input?.background ?? {}) },
    menu_font_family: input?.menu_font_family || MENU_FONT_INHERIT_KEY,
    menu_font_color: input?.menu_font_color || '',
  }
}

/** 将外观设置落到 CSS 变量与根元素样式上，立即生效。 */
export function applyAppearance(appearance: AppearanceSettings) {
  const root = document.documentElement
  const preset = PRESETS.find(item => item.key === appearance.preset) ?? PRESETS[0]
  root.style.setProperty('--accent', appearance.accent || preset.accent)
  root.style.setProperty('--bg', preset.bg)
  root.style.setProperty('--panel', preset.panel)

  const background = appearance.background
  let value = ''
  root.style.removeProperty('--app-bg-image')
  root.style.removeProperty('--app-bg-image-overlay')
  root.style.removeProperty('--app-bg-image-position')
  root.style.removeProperty('--app-bg-image-scale')
  if (background.mode === 'solid' && background.solid) {
    value = background.solid
  } else if (background.mode === 'gradient') {
    value = buildGradient(background.gradient, background.gradient_colors)
  } else if (background.mode === 'image' && background.image_url.trim()) {
    const opacity = Math.min(95, Math.max(0, background.image_opacity)) / 100
    const positionX = Math.min(100, Math.max(0, background.image_position_x ?? 50))
    const positionY = Math.min(100, Math.max(0, background.image_position_y ?? 50))
    const scale = Math.min(200, Math.max(75, background.image_scale ?? 100)) / 100
    const safeUrl = background.image_url.trim().replace(/["\\()]/g, character => encodeURIComponent(character))
    root.style.setProperty('--app-bg-image', `url("${safeUrl}")`)
    root.style.setProperty('--app-bg-image-overlay', `rgba(8,10,14,${opacity})`)
    root.style.setProperty('--app-bg-image-position', `${positionX}% ${positionY}%`)
    root.style.setProperty('--app-bg-image-scale', String(scale))
    value = background.solid || preset.bg
  }
  if (value) root.style.setProperty('--app-bg', value)
  else root.style.removeProperty('--app-bg')
  root.style.setProperty('--app-bg-color', background.solid || preset.bg)
  root.style.setProperty('--card-blur', `${Math.min(40, Math.max(0, appearance.card_blur ?? 18))}px`)
  root.dataset.backgroundMode = background.mode
  // 面板半透明让背景透出：渐变/图片背景，或单色但改成了非预设默认色时开启。
  const richSolid = background.mode === 'solid'
    && background.solid.trim().toLowerCase() !== preset.bg.toLowerCase()
  root.classList.toggle('bg-rich', !!value && (background.mode !== 'solid' || richSolid))

  const font = FONTS.find(item => item.key === appearance.font_family) ?? FONTS[0]
  root.style.fontFamily = font.css
  root.style.setProperty('--text-primary', appearance.font_color || preset.font_color)
  root.style.color = appearance.font_color || preset.font_color
  const menuFontKey = appearance.menu_font_family || MENU_FONT_INHERIT_KEY
  const menuFont = menuFontKey === MENU_FONT_INHERIT_KEY
    ? font
    : FONTS.find(item => item.key === menuFontKey) ?? font
  root.style.setProperty('--menu-font-family', menuFont.css)
  const menuFontColor = appearance.menu_font_color?.trim() ?? ''
  if (/^#[0-9a-f]{6}$/i.test(menuFontColor)) root.style.setProperty('--menu-font-color', menuFontColor)
  else root.style.removeProperty('--menu-font-color')
  // 字体大小 = 整体 UI 缩放；反向画布尺寸保证放大后仍完整落在视口内。
  const { scale, inversePercent } = resolveUiScale(appearance.font_size ?? 100)
  root.style.setProperty('--ui-scale', String(scale))
  root.style.setProperty('--ui-scale-inverse', inversePercent)
  // Clear the legacy root zoom during hot reloads and persisted appearance restores.
  ;(root.style as CSSStyleDeclaration & { zoom: string }).zoom = ''
}
