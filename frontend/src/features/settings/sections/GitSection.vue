<script setup lang="ts">
import { ref, watch } from 'vue'
import type { GitSettings } from '../types'
import { saveUi, uiSettings } from '../store'

const DEFAULT_GIT: GitSettings = { username: '', email: '', default_branch: 'main', remote_url: '', auto_commit: false }
const draft = ref<GitSettings>({ ...DEFAULT_GIT })
watch(uiSettings, value => { if (value) draft.value = { ...DEFAULT_GIT, ...value.git } }, { immediate: true })

async function commit(message = 'Git 设置已保存') {
  await saveUi({ git: { ...draft.value, username: draft.value.username.trim(), email: draft.value.email.trim(), default_branch: draft.value.default_branch.trim() || 'main', remote_url: draft.value.remote_url.trim() } }, message)
}
function toggleAutoCommit() {
  draft.value.auto_commit = !draft.value.auto_commit
  commit(draft.value.auto_commit ? '已开启配置自动提交' : '已关闭配置自动提交')
}
</script>

<template>
  <div class="s-group">
    <h2>提交身份</h2>
    <p class="desc">AI 修改服务器配置、插件方案时使用的 Git 提交身份。</p>
    <div class="s-card">
      <div class="s-row">
        <p><b>用户名</b><small>对应 git config user.name</small></p>
        <input class="s-input" style="width:220px" v-model="draft.username" placeholder="例如 sculk-admin" @change="commit()"/>
      </div>
      <div class="s-row">
        <p><b>邮箱</b><small>对应 git config user.email</small></p>
        <input class="s-input" style="width:220px" v-model="draft.email" placeholder="you@example.com" @change="commit()"/>
      </div>
    </div>
  </div>

  <div class="s-group">
    <h2>仓库</h2>
    <div class="s-card">
      <div class="s-row">
        <p><b>默认分支</b><small>AI 生成的配置变更会先提交到此分支</small></p>
        <input class="s-input" style="width:160px" v-model="draft.default_branch" placeholder="main" @change="commit()"/>
      </div>
      <div class="s-row">
        <p><b>远程仓库地址</b><small>用于备份服务器配置与插件清单（可留空）</small></p>
        <input class="s-input" style="width:300px" v-model="draft.remote_url" placeholder="git@github.com:you/server-config.git" @change="commit()"/>
      </div>
      <div class="s-row">
        <p><b>配置变更自动提交</b><small>AI 每次修改 server.properties 或插件配置后自动生成一次提交，便于回滚</small></p>
        <button class="s-switch" :class="{on:draft.auto_commit}" @click="toggleAutoCommit"><i/></button>
      </div>
    </div>
  </div>
</template>
