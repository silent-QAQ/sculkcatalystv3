<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, Plus, SlidersHorizontal, Trash2 } from 'lucide-vue-next'
import { CUSTOM_PRESET_KEY, FONTS, GRADIENTS, PRESETS, applyAppearance, buildGradient, presetAppearance } from '../../../lib/appearance'
import type { AppearanceSettings, BackgroundMode } from '../types'
import { saveUi, uiSettings } from '../store'

const draft = ref<AppearanceSettings>(presetAppearance('sculk'))
watch(uiSettings, value => {
  if (!value) return
  const base = presetAppearance('sculk')
  draft.value = { ...base, ...value.appearance, background: { ...base.background, ...value.appearance.background } }
}, { immediate: true })

const mode = computed(() => draft.value.background.mode)
const isCustom = computed(() => draft.value.preset === CUSTOM_PRESET_KEY)

async function commit(message = '外观已更新') {
  applyAppearance(draft.value)
  await saveUi({ appearance: draft.value }, message)
}
/** 选择预设 = 整套应用（强调色 + 背景 + 字体）。 */
function setPreset(key: string) {
  draft.value = presetAppearance(key)
  commit('预设风格已应用（含背景与字体）')
}
/** 进入自定义：以当前值为起点，仅改标记。 */
function pickCustom() {
  if (isCustom.value) return
  draft.value.preset = CUSTOM_PRESET_KEY
  commit('已切换到自定义预设，可自由调整下方选项')
}
/** 手动改动任何外观项后自动脱离命名预设。 */
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
    draft.value.background.gradient_colors = ['#071a17', '#21183d']
  }
  touch('背景方案已切换')
}
function setGradient(value: string) { draft.value.background.gradient = value; touch('融合方案已切换') }
function addGradientColor() {
  if (draft.value.background.gradient_colors.length >= 5) return
  draft.value.background.gradient_colors.push(draft.value.accent || '#32d5b0')
  touch('已添加融合颜色')
}
function removeGradientColor(index: number) {
  if (draft.value.background.gradient_colors.length <= 2) return
  draft.value.background.gradient_colors.splice(index, 1)
  touch('已移除融合颜色')
}
function setFont(value: string) { draft.value.font_family = value; touch('字体风格已更新') }
async function reset() {
  draft.value = presetAppearance('sculk')
  await commit('外观已恢复默认')
}
</script>

<template>
  <div class="s-group">
    <h2>预设风格</h2>
    <p class="desc">预设是强调色、背景与字体的完整组合，选择后整套应用；手动调整下方任意选项会自动切换为「自定义」。</p>
    <div class="s-cards">
      <button v-for="preset in PRESETS" :key="preset.key" class="s-pick-card" :class="{active:draft.preset===preset.key}" @click="setPreset(preset.key)">
        <span class="dots"><i :style="{background:preset.accent}"/><i :style="{background:preset.panel}"/><i :style="{background:preset.bg,border:'1px solid rgba(255,255,255,.15)'}"/></span>
        <b>{{ preset.label }}</b><small>{{ preset.hint }}</small>
        <span v-if="draft.preset===preset.key" class="check"><Check/></span>
      </button>
      <button class="s-pick-card" :class="{active:isCustom}" @click="pickCustom">
        <span class="dots" style="align-items:center;color:#8b96a2"><SlidersHorizontal style="width:14px"/></span>
        <b>自定义</b><small>以当前外观为起点，自由组合强调色、背景与字体</small>
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
    <h2>UI 背景</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>背景类型</b><small>单色 / 多色融合 / 自定义图片</small></p>
        <span class="s-seg">
          <button :class="{active:mode==='solid'}" @click="setMode('solid')">单色</button>
          <button :class="{active:mode==='gradient'}" @click="setMode('gradient')">多色融合</button>
          <button :class="{active:mode==='image'}" @click="setMode('image')">图片</button>
        </span>
      </div>
      <div v-if="mode==='solid'" class="s-row">
        <p><b>背景颜色</b><small>应用于整个工作台底色，面板会半透明透出背景色</small></p>
        <input class="s-color" type="color" v-model="draft.background.solid" @input="previewCustom" @change="touch('背景颜色已更新')"/>
      </div>
      <div v-if="mode==='gradient'" class="s-row" style="align-items:flex-start">
        <p><b>融合颜色</b><small>设置 2–5 个颜色，颜色顺序会影响渐变走向</small></p>
        <div class="gradient-palette">
          <div v-for="(_, index) in draft.background.gradient_colors" :key="index" class="gradient-color">
            <input type="color" v-model="draft.background.gradient_colors[index]" @input="previewCustom" @change="touch('融合颜色已更新')"/>
            <code>{{ draft.background.gradient_colors[index] }}</code>
            <button :disabled="draft.background.gradient_colors.length<=2" :aria-label="`移除颜色 ${index+1}`" @click="removeGradientColor(index)"><Trash2/></button>
          </div>
          <button class="gradient-add" :disabled="draft.background.gradient_colors.length>=5" @click="addGradientColor"><Plus/>添加颜色</button>
        </div>
      </div>
      <div v-if="mode==='gradient'" class="s-row" style="align-items:flex-start">
        <p><b>融合方案</b><small>只改变颜色如何铺开，不会替换你的调色板</small></p>
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
          <p><b>遮罩不透明度</b><small>值越大界面越沉、图片越淡（当前 {{ draft.background.image_opacity }}%）</small></p>
          <input type="range" min="0" max="95" v-model.number="draft.background.image_opacity" @change="touch('透明度已更新')" :style="{accentColor:'var(--accent)',width:'180px'}"/>
        </div>
      </template>
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
        <p><b>字体颜色</b><small>正文默认颜色，仅建议微调亮度</small></p>
        <input class="s-color" type="color" v-model="draft.font_color" @input="previewCustom" @change="touch('字体颜色已更新')"/>
      </div>
    </div>
  </div>

  <div class="s-group">
    <button class="s-btn" @click="reset">恢复默认外观</button>
  </div>
</template>
