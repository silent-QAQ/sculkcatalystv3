<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, Plus, SlidersHorizontal, Trash2 } from 'lucide-vue-next'
import { CUSTOM_PRESET_KEY, DEFAULT_MENU_FONT_COLOR, FONTS, GRADIENTS, MENU_FONT_INHERIT_KEY, PRESETS, applyAppearance, buildGradient, normalizeAppearance, presetAppearance } from '../../../lib/appearance'
import type { AppearanceSettings, BackgroundMode } from '../types'
import { saveUi, uiSettings } from '../store'

const draft = ref<AppearanceSettings>(presetAppearance('sculk'))
watch(uiSettings, value => {
  if (!value) return
  draft.value = normalizeAppearance(value.appearance)
}, { immediate: true })

const mode = computed(() => draft.value.background.mode)
const isCustom = computed(() => draft.value.preset === CUSTOM_PRESET_KEY)
const menuFontColor = computed({
  get: () => draft.value.menu_font_color || DEFAULT_MENU_FONT_COLOR,
  set: (value: string) => { draft.value.menu_font_color = value },
})
const menuFontPreviewCss = computed(() => {
  const key = draft.value.menu_font_family === MENU_FONT_INHERIT_KEY
    ? draft.value.font_family
    : draft.value.menu_font_family
  return FONTS.find(font => font.key === key)?.css ?? FONTS[0].css
})
const menuTypographyIsDefault = computed(() => (
  draft.value.menu_font_family === MENU_FONT_INHERIT_KEY && !draft.value.menu_font_color
))

async function commit(message = '外观已更新') {
  applyAppearance(draft.value)
  await saveUi({ appearance: draft.value }, message)
}

function setPreset(key: string) {
  draft.value = presetAppearance(key)
  commit('预设风格已应用，包含背景与字体')
}

function pickCustom() {
  if (isCustom.value) return
  draft.value.preset = CUSTOM_PRESET_KEY
  commit('已切换到自定义外观')
}

function touch(message: string) {
  draft.value.preset = CUSTOM_PRESET_KEY
  commit(message)
}

function previewCustom() {
  draft.value.preset = CUSTOM_PRESET_KEY
  applyAppearance(draft.value)
}

function setMode(value: BackgroundMode) {
  draft.value.background.mode = value
  if (value === 'gradient' && draft.value.background.gradient_colors.length < 2) {
    draft.value.background.gradient_colors = ['#f9a9d0', '#5cb3ff']
  }
  touch('背景方案已切换')
}

function setGradient(value: string) {
  draft.value.background.gradient = value
  touch('渐变方案已切换')
}

function addGradientColor() {
  if (draft.value.background.gradient_colors.length >= 5) return
  draft.value.background.gradient_colors.push(draft.value.accent || '#32d5b0')
  touch('已添加渐变颜色')
}

function removeGradientColor(index: number) {
  if (draft.value.background.gradient_colors.length <= 2) return
  draft.value.background.gradient_colors.splice(index, 1)
  touch('已移除渐变颜色')
}

function setFont(value: string) {
  draft.value.font_family = value
  touch('字体风格已更新')
}

function setMenuFont(value: string) {
  draft.value.menu_font_family = value
  touch('菜单字体风格已更新')
}

function resetMenuTypography() {
  draft.value.menu_font_family = MENU_FONT_INHERIT_KEY
  draft.value.menu_font_color = ''
  touch('菜单字体已恢复跟随主题')
}

async function reset() {
  draft.value = presetAppearance('sculk')
  await commit('外观已恢复默认')
}
</script>

<template>
  <div class="s-group">
    <h2>预设风格</h2>
    <p class="desc">预设会同时应用强调色、背景和字体；手动调整任意选项后会自动切换为自定义。</p>
    <div class="s-cards">
      <button v-for="preset in PRESETS" :key="preset.key" class="s-pick-card" :class="{active:draft.preset===preset.key}" @click="setPreset(preset.key)">
        <span class="dots"><i :style="{background:preset.accent}"/><i :style="{background:preset.panel}"/><i :style="{background:preset.bg,border:'1px solid rgba(255,255,255,.15)'}"/></span>
        <b>{{ preset.label }}</b><small>{{ preset.hint }}</small>
        <span v-if="draft.preset===preset.key" class="check"><Check/></span>
      </button>
      <button class="s-pick-card" :class="{active:isCustom}" @click="pickCustom">
        <span class="dots" style="align-items:center;color:#8b96a2"><SlidersHorizontal style="width:14px"/></span>
        <b>自定义</b><small>自由组合强调色、背景、字体和卡片毛玻璃效果</small>
        <span v-if="isCustom" class="check"><Check/></span>
      </button>
    </div>
  </div>

  <div class="s-group">
    <h2>强调色</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>强调色</b><small>按钮、开关与高亮元素的主色（当前 {{ draft.accent }}）</small></p>
        <input class="s-color" type="color" v-model="draft.accent" @input="previewCustom" @change="touch('强调色已更新')"/>
      </div>
    </div>
  </div>

  <div class="s-group">
    <h2>界面背景</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>背景类型</b><small>单色、多色渐变或自定义图片</small></p>
        <span class="s-seg">
          <button :class="{active:mode==='solid'}" @click="setMode('solid')">单色</button>
          <button :class="{active:mode==='gradient'}" @click="setMode('gradient')">多色渐变</button>
          <button :class="{active:mode==='image'}" @click="setMode('image')">图片</button>
        </span>
      </div>
      <div v-if="mode==='solid'" class="s-row">
        <p><b>背景颜色</b><small>应用于整个工作台底色，面板会透出背景色</small></p>
        <input class="s-color" type="color" v-model="draft.background.solid" @input="previewCustom" @change="touch('背景颜色已更新')"/>
      </div>
      <div v-if="mode==='gradient'" class="s-row" style="align-items:flex-start">
        <p><b>渐变颜色</b><small>设置 2–5 个颜色，顺序会影响渐变走向</small></p>
        <div class="gradient-palette">
          <div v-for="(_, index) in draft.background.gradient_colors" :key="index" class="gradient-color">
            <input type="color" v-model="draft.background.gradient_colors[index]" @input="previewCustom" @change="touch('渐变颜色已更新')"/>
            <code>{{ draft.background.gradient_colors[index] }}</code>
            <button :disabled="draft.background.gradient_colors.length<=2" :aria-label="`移除颜色 ${index+1}`" @click="removeGradientColor(index)"><Trash2/></button>
          </div>
          <button class="gradient-add" :disabled="draft.background.gradient_colors.length>=5" @click="addGradientColor"><Plus/>添加颜色</button>
        </div>
      </div>
      <div v-if="mode==='gradient'" class="s-row" style="align-items:flex-start">
        <p><b>渐变方案</b><small>只改变颜色的铺开方式，不会替换你的调色板</small></p>
        <div class="s-cards gradient-schemes">
          <button v-for="gradient in GRADIENTS" :key="gradient.key" class="s-pick-card" :class="{active:draft.background.gradient===gradient.key}" @click="setGradient(gradient.key)">
            <span class="swatch" :style="{background:buildGradient(gradient.key,draft.background.gradient_colors)}"/>
            <b>{{ gradient.label }}</b><small>{{ gradient.hint }}</small>
            <span v-if="draft.background.gradient===gradient.key" class="check"><Check/></span>
          </button>
        </div>
      </div>
      <template v-if="mode==='image'">
        <div class="s-row">
          <p><b>图片地址</b><small>支持 https:// 图片 URL 或本地静态资源路径</small></p>
          <input class="s-input" style="width:280px" v-model="draft.background.image_url" placeholder="https://…/background.png" @change="touch('背景图片已更新')"/>
        </div>
        <div class="s-row">
          <p><b>图片遮罩</b><small>值越大界面越沉、图片越淡（当前 {{ draft.background.image_opacity }}%）</small></p>
          <input type="range" min="0" max="95" v-model.number="draft.background.image_opacity" @input="previewCustom" @change="touch('图片遮罩已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
        </div>
        <div class="s-row">
          <p><b>图片水平位置</b><small>0% 为左侧，100% 为右侧，当前 {{ draft.background.image_position_x }}%</small></p>
          <input type="range" min="0" max="100" v-model.number="draft.background.image_position_x" @input="previewCustom" @change="touch('图片水平位置已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
        </div>
        <div class="s-row">
          <p><b>图片垂直位置</b><small>0% 为上方，100% 为下方，当前 {{ draft.background.image_position_y }}%</small></p>
          <input type="range" min="0" max="100" v-model.number="draft.background.image_position_y" @input="previewCustom" @change="touch('图片垂直位置已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
        </div>
        <div class="s-row">
          <p><b>图片缩放</b><small>调整背景图的视野大小，当前比例 {{ draft.background.image_scale }}%</small></p>
          <input type="range" min="75" max="200" step="1" v-model.number="draft.background.image_scale" @input="previewCustom" @change="touch('图片缩放已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
        </div>
      </template>
      <div class="s-row">
        <p><b>卡片毛玻璃模糊</b><small>只影响面板与卡片，背景图片本身不会被模糊，当前 {{ draft.card_blur }}px</small></p>
        <input type="range" min="0" max="40" step="1" v-model.number="draft.card_blur" @input="previewCustom" @change="touch('卡片毛玻璃已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
      </div>
    </div>
  </div>

  <div class="s-group">
    <h2>字体</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>字体风格</b><small>影响全局界面文字</small></p>
        <select class="s-select" :value="draft.font_family" @change="setFont(($event.target as HTMLSelectElement).value)">
          <option v-for="font in FONTS" :key="font.key" :value="font.key">{{ font.label }}</option>
        </select>
      </div>
      <div class="s-row">
        <p><b>字体大小</b><small>整体 UI 缩放（当前 {{ draft.font_size }}%，100% 为标准）</small></p>
        <div style="display:flex;align-items:center;gap:8px">
          <input type="range" min="70" max="150" step="5" v-model.number="draft.font_size" @input="previewCustom" @change="touch('字体大小已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
          <button class="s-btn small" :disabled="draft.font_size===100" @click="draft.font_size=100;touch('字体大小已重置')">重置</button>
        </div>
      </div>
      <div class="s-row">
        <p><b>字体颜色</b><small>正文默认颜色，建议只做轻微调整</small></p>
        <input class="s-color" type="color" v-model="draft.font_color" @input="previewCustom" @change="touch('字体颜色已更新')"/>
      </div>
      <div class="s-row">
        <p><b>菜单字体风格</b><small>应用于侧栏、会话树、设置导航、工作区标签和弹出菜单，不影响正文与代码编辑器</small></p>
        <select class="s-select" :value="draft.menu_font_family" @change="setMenuFont(($event.target as HTMLSelectElement).value)">
          <option :value="MENU_FONT_INHERIT_KEY">跟随全局字体</option>
          <option v-for="font in FONTS" :key="font.key" :value="font.key">{{ font.label }}</option>
        </select>
      </div>
      <div class="s-row">
        <p><b>菜单字体颜色</b><small>调整普通菜单文字；选中、高亮、警告和危险操作仍保留状态色</small></p>
        <div class="menu-font-controls">
          <span class="menu-font-preview" :style="{fontFamily:menuFontPreviewCss,color:menuFontColor}">控制中心　任务执行器　设置</span>
          <input class="s-color" type="color" v-model="menuFontColor" @input="previewCustom" @change="touch('菜单字体颜色已更新')"/>
          <button class="s-btn small" :disabled="menuTypographyIsDefault" @click="resetMenuTypography">跟随主题</button>
        </div>
      </div>
    </div>
  </div>

  <div class="s-group">
    <button class="s-btn" @click="reset">恢复默认外观</button>
  </div>
</template>
