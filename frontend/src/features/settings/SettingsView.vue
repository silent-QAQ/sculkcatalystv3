<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Bot, Cloud, GitBranch, Globe, Mic, Palette, PlugZap, Search, Settings, Sparkles, BrainCircuit } from 'lucide-vue-next'
import { loadAll, notice } from './store'
import GeneralSection from './sections/GeneralSection.vue'
import AppearanceSection from './sections/AppearanceSection.vue'
import ModelsSection from './sections/ModelsSection.vue'
import SpeechRecognitionSection from './sections/SpeechRecognitionSection.vue'
import AgentsSection from './sections/AgentsSection.vue'
import PersonalizationSection from './sections/PersonalizationSection.vue'
import AccountSection from './sections/AccountSection.vue'
import PluginsSection from './sections/PluginsSection.vue'
import GitSection from './sections/GitSection.vue'
import ConnectionsSection from './sections/ConnectionsSection.vue'
import type { ServerTemplate } from '../portable/server-manifest'

type SectionKey = 'general' | 'appearance' | 'models' | 'speech' | 'agents' | 'personalization' | 'account' | 'plugins' | 'git' | 'connections'

const props = withDefaults(defineProps<{
  initialSection?: SectionKey
  initialCloudTab?: 'overview' | 'agents' | 'workspace'
}>(), { initialSection: 'general', initialCloudTab: 'overview' })
const emit = defineEmits<{ applyServerTemplate: [template: ServerTemplate] }>()

const SECTIONS: { key: SectionKey; label: string; icon: any; group: string; keywords: string }[] = [
  { key: 'general', label: '常规', icon: Settings, group: '个人', keywords: '常规 语言 language 审批 权限 审核 review approval' },
  { key: 'appearance', label: '外观', icon: Palette, group: '个人', keywords: '外观 主题 背景 颜色 字体 透明 theme appearance font color background' },
  { key: 'models', label: '模型', icon: BrainCircuit, group: '个人', keywords: '模型 提供商 api key 默认模型 语音 开服 报错 修复 管理 model provider' },
  { key: 'speech', label: '语音识别', icon: Mic, group: '个人', keywords: '语音 录音 麦克风 识别 转写 asr whisper speech recognition microphone' },
  { key: 'agents', label: '智能体管理', icon: Bot, group: '个人', keywords: '智能体 agent acp codex claude openclaw hermes sculkagent' },
  { key: 'personalization', label: '个性化', icon: Sparkles, group: '个人', keywords: '个性化 风格 语气 上下文 style context persona' },
  { key: 'account', label: 'Sculk Cloud', icon: Cloud, group: '云服务', keywords: '云账号 账户 account 同步 团队 审批 token api cloud' },
  { key: 'plugins', label: '开服器插件', icon: PlugZap, group: '集成', keywords: '插件 技能 skills mcp 扩展 集成' },
  { key: 'git', label: 'Git', icon: GitBranch, group: '集成', keywords: 'git 版本 分支 提交 仓库 branch commit' },
  { key: 'connections', label: '连接', icon: Globe, group: '集成', keywords: '连接 远程 服务器 ssh sftp remote connection' },
]

const active = ref<SectionKey>(props.initialSection)
const query = ref('')

const filtered = computed(() => {
  const text = query.value.trim().toLowerCase()
  if (!text) return SECTIONS
  return SECTIONS.filter(section => (section.label + ' ' + section.keywords).toLowerCase().includes(text))
})
const groups = computed(() => {
  const map: { name: string; items: typeof SECTIONS }[] = []
  for (const section of filtered.value) {
    let group = map.find(item => item.name === section.group)
    if (!group) { group = { name: section.group, items: [] }; map.push(group) }
    group.items.push(section)
  }
  return map
})
const activeMeta = computed(() => SECTIONS.find(section => section.key === active.value)!)

const COMPONENTS: Record<SectionKey, any> = {
  general: GeneralSection,
  appearance: AppearanceSection,
  models: ModelsSection,
  speech: SpeechRecognitionSection,
  agents: AgentsSection,
  personalization: PersonalizationSection,
  account: AccountSection,
  plugins: PluginsSection,
  git: GitSection,
  connections: ConnectionsSection,
}

onMounted(loadAll)
</script>

<template>
  <div class="settings-layout">
    <aside class="settings-nav">
      <div class="search"><Search/><input v-model="query" placeholder="搜索设置…"/></div>
      <template v-for="group in groups" :key="group.name">
        <div class="group-label">{{ group.name }}</div>
        <nav>
          <button v-for="section in group.items" :key="section.key" :class="{active:active===section.key}" @click="active=section.key">
            <component :is="section.icon"/>{{ section.label }}
          </button>
        </nav>
      </template>
      <p v-if="!filtered.length" class="no-result">没有匹配「{{ query }}」的设置项</p>
    </aside>
    <section class="settings-content">
      <h1>{{ activeMeta.label }}</h1>
      <AccountSection
        v-if="active === 'account'"
        :initial-tab="initialCloudTab"
        @apply-server-template="template => emit('applyServerTemplate', template)"
      />
      <component :is="COMPONENTS[active]" v-else/>
      <div v-if="notice" class="s-notice">{{ notice }}</div>
    </section>
  </div>
</template>
