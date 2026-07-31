<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Bot, Cable, ExternalLink, Link2, LoaderCircle, RefreshCw, Sparkles, Unplug } from 'lucide-vue-next'
import { apiRequest } from '../../../lib/api'
import { cloudSkillLinks } from '../../cloud/workspace'
import type { IntegrationItem, SkillItem } from '../types'
import { flash, friendly } from '../store'

const integrations = ref<IntegrationItem[]>([])
const skills = ref<SkillItem[]>([])
const loading = ref('')

async function load() {
  try {
    const data = await apiRequest<{ integrations: IntegrationItem[]; skills: SkillItem[] }>('/api/integrations')
    integrations.value = data.integrations
    skills.value = data.skills
  } catch (error) { flash('加载失败：' + friendly(error)) }
}
async function toggleIntegration(item: IntegrationItem) {
  loading.value = item.id
  try { await apiRequest('/api/integrations/' + item.id + '/toggle', { method: 'POST' }); await load() }
  catch (error) { flash('操作失败：' + friendly(error)) }
  finally { loading.value = '' }
}
async function testIntegration(item: IntegrationItem) {
  loading.value = item.id
  try {
    const result = await apiRequest<IntegrationItem>('/api/integrations/' + item.id + '/test', { method: 'POST' })
    flash(result.name + ' 连接成功，延迟 ' + result.latency_ms + ' ms')
    await load()
  } catch (error) { flash('测试失败：' + friendly(error)) }
  finally { loading.value = '' }
}
async function toggleSkill(skill: SkillItem) {
  loading.value = skill.id
  try { await apiRequest('/api/skills/' + skill.id + '/toggle', { method: 'POST' }); await load() }
  catch (error) { flash('操作失败：' + friendly(error)) }
  finally { loading.value = '' }
}
onMounted(load)
</script>

<template>
  <div class="s-group">
    <h2>MCP 服务连接</h2>
    <p class="desc">开服器通过 MCP 协议连接 Codex、社区渠道、监控系统等外部服务；只可访问显式授权的服务器与目录。</p>
    <div class="s-card">
      <div v-for="item in integrations" :key="item.id" class="s-row" :style="{opacity:item.enabled?1:.55}">
        <span :style="{display:'grid',placeItems:'center',width:'29px',height:'29px',borderRadius:'7px',flex:'none',color:item.enabled?'var(--accent)':'#727d89',background:item.enabled?'color-mix(in srgb,var(--accent) 10%,transparent)':'#1a2027'}">
          <Cable v-if="item.enabled" style="width:14px"/><Unplug v-else style="width:14px"/>
        </span>
        <p>
          <b>{{ item.name }}</b>
          <small>{{ item.kind.toUpperCase() }} · <code>{{ item.endpoint }}</code><template v-if="item.latency_ms"> · {{ item.latency_ms }} ms</template></small>
        </p>
        <button class="s-btn small" :disabled="!item.enabled||loading===item.id" @click="testIntegration(item)"><LoaderCircle v-if="loading===item.id" class="s-spin"/><RefreshCw v-else/>测试</button>
        <button class="s-switch" :class="{on:item.enabled}" @click="toggleIntegration(item)"><i/></button>
      </div>
      <div v-if="!integrations.length" class="s-row"><p><small>暂无 MCP 连接。</small></p></div>
    </div>
  </div>

  <div class="s-group">
    <h2>Agent 技能</h2>
    <p class="desc">为开服器启用或停用 AI 能力包；停用后相关自动化任务不再可用。已启用 {{ skills.filter(skill=>skill.enabled).length }} / {{ skills.length }}。</p>
    <div class="s-card">
      <div v-for="skill in skills" :key="skill.id" class="s-row" :style="{opacity:skill.enabled?1:.55}">
        <span style="display:grid;place-items:center;width:29px;height:29px;border-radius:7px;color:#a99dff;background:rgba(156,140,255,.09);flex:none">
          <Bot v-if="skill.source==='builtin'" style="width:14px"/><Sparkles v-else style="width:14px"/>
        </span>
        <p><b>{{ skill.name }}</b><small>{{ skill.description }} · {{ skill.source }} · v{{ skill.version }}</small></p>
        <button class="s-switch" :class="{on:skill.enabled}" @click="toggleSkill(skill)"><i/></button>
      </div>
      <div v-if="!skills.length" class="s-row"><p><small>暂无技能。</small></p></div>
    </div>
  </div>

  <div class="s-group">
    <h2>云端 Skill 链接</h2>
    <p class="desc">登录云账号后自动同步，可用于保存 Skill 仓库、安装说明或团队共享入口。</p>
    <div class="s-card">
      <a v-for="item in cloudSkillLinks.filter(link=>link.enabled)" :key="item.id" class="s-row" :href="item.url" target="_blank" rel="noopener noreferrer">
        <span style="display:grid;place-items:center;width:29px;height:29px;border-radius:7px;color:var(--accent);background:color-mix(in srgb,var(--accent) 10%,transparent);flex:none"><Link2 style="width:14px"/></span>
        <p><b>{{ item.name }}</b><small>{{ item.url }}</small></p><ExternalLink style="width:14px;color:#727d89"/>
      </a>
      <div v-if="!cloudSkillLinks.filter(link=>link.enabled).length" class="s-row"><p><small>暂无云端 Skill 链接，请在「云账号」中添加。</small></p></div>
    </div>
  </div>
</template>
