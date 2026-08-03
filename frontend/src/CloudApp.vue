<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->

<script setup lang="ts">
import { Box, ShieldCheck } from 'lucide-vue-next'
import AccountSection from './features/settings/sections/AccountSection.vue'
import './cloud-app.css'

const initialAuthMode = new URLSearchParams(window.location.search).get('mode') === 'register' ? 'register' : 'login'

document.title = 'Sculk Cloud — 云端协作控制台'
document.querySelector('meta[name="description"]')?.setAttribute(
  'content',
  'Sculk Cloud 账号同步、团队审批与 API 中转控制台',
)
</script>

<template>
  <div class="cloud-app">
    <header class="cloud-app-bar">
      <a href="/" class="cloud-app-brand" aria-label="Sculk Cloud 首页">
        <span><Box/></span>
        <p><b>Sculk Catalyst</b><small>Cloud Console</small></p>
      </a>
      <div class="cloud-app-trust"><ShieldCheck/>账号同步、团队审批与 API 中转</div>
    </header>

    <main class="cloud-app-main">
      <AccountSection context="web" :initial-auth-mode="initialAuthMode"/>
    </main>

    <footer class="cloud-app-footer">
      云端控制台不会直接访问你的本地服务器文件或进程。
    </footer>
  </div>
</template>
