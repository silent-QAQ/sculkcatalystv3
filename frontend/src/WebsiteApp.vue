<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  Box,
  Check,
  ChevronRight,
  Cloud,
  Cpu,
  Download,
  FileCode2,
  FolderTree,
  HardDrive,
  Menu,
  Monitor,
  RadioTower,
  Server,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  Users,
  X,
} from 'lucide-vue-next'
import DotField from './components/DotField.vue'
import './website.css'

type DownloadPlatform = 'windows' | 'linux'

const menuOpen = ref(false)
const selectedPlatform = ref<DownloadPlatform>('windows')
const cloudPortalUrl = String(import.meta.env.VITE_CLOUD_PORTAL_URL || 'https://sculk.mcmy.love').replace(/\/$/, '')
const cloudRegisterUrl = `${cloudPortalUrl}/?mode=register`
const cloudLoginUrl = `${cloudPortalUrl}/`
const githubUrl = 'https://github.com/silent-QAQ/sculkcatalystv3'

const capabilities = [
  {
    index: '01',
    icon: Sparkles,
    title: 'AI 开服协作',
    body: '把服务器构想、配置取舍和创建步骤放进同一条对话与任务流中。',
    tone: 'mint',
  },
  {
    index: '02',
    icon: Server,
    title: '可视化服务器控制',
    body: '从 Java 环境与核心下载，到启动、停止、日志和终端，都能在工作台中完成。',
    tone: 'violet',
  },
  {
    index: '03',
    icon: FolderTree,
    title: '资源与文件管理',
    body: '安全浏览工作区、编辑常用配置，并从资源目录查找适配的服务端核心与插件。',
    tone: 'amber',
  },
  {
    index: '04',
    icon: Cloud,
    title: '云端协作连接',
    body: '云账号连接设备、团队、审批与配置同步；本地主机仍由你自己掌控。',
    tone: 'blue',
  },
]

const downloadOptions: Array<{
  id: DownloadPlatform
  platform: string
  architecture: string
  href: string
  description: string
}> = [
  {
    id: 'windows',
    platform: 'Windows',
    architecture: 'Windows x64',
    href: '/downloads/sculk-agent-windows-x86_64.exe',
    description: '适用于 Windows x64 主机，一次下载即可开始配对。',
  },
  {
    id: 'linux',
    platform: 'Linux',
    architecture: 'Linux x64',
    href: '/downloads/sculk-agent-linux-x86_64',
    description: '适用于 Linux x64 主机，适合长期运行的服务端环境。',
  },
]

const selectedDownload = computed(() => downloadOptions.find(item => item.id === selectedPlatform.value) || downloadOptions[0])

function closeMenu() {
  menuOpen.value = false
}

function selectPlatform(platform: DownloadPlatform) {
  selectedPlatform.value = platform
  document.querySelector('#downloads')?.scrollIntoView({ behavior: 'smooth', block: 'start' })
}

document.title = 'Sculk Catalyst — AI Minecraft Server Studio'
document.querySelector('meta[name="description"]')?.setAttribute(
  'content',
  'Sculk Catalyst：面向 Minecraft 服务端的 AI 工作台，连接本地控制、资源管理与云端协作。',
)
document.querySelector('meta[name="theme-color"]')?.setAttribute('content', '#07100f')

onMounted(() => {
  document.documentElement.dataset.sculkSurface = 'website'
  document.body.dataset.sculkSurface = 'website'
})

onUnmounted(() => {
  delete document.documentElement.dataset.sculkSurface
  delete document.body.dataset.sculkSurface
})
</script>

<template>
  <div class="website">
    <a class="website-skip-link" href="#main-content">跳到主要内容</a>

    <header class="website-header">
      <a class="website-brand" href="#top" aria-label="Sculk Catalyst 首页" @click="closeMenu">
        <span class="website-brand-mark"><Box/></span>
        <span><b>Sculk Catalyst</b><small>AI Server Studio</small></span>
      </a>

      <button class="website-menu-toggle" type="button" :aria-expanded="menuOpen" aria-label="切换导航菜单" @click="menuOpen = !menuOpen">
        <X v-if="menuOpen"/><Menu v-else/>
      </button>

      <nav class="website-nav" :class="{ open: menuOpen }" aria-label="主导航">
        <a href="#features" @click="closeMenu">功能</a>
        <a href="#workflow" @click="closeMenu">工作方式</a>
        <a href="#downloads" @click="closeMenu">下载</a>
        <a :href="cloudLoginUrl" @click="closeMenu">登录 Cloud</a>
        <a class="website-nav-cta" :href="cloudRegisterUrl" @click="closeMenu">创建云账号 <ChevronRight/></a>
      </nav>
    </header>

    <main id="main-content">
      <section id="top" class="website-hero" aria-labelledby="hero-title">
        <div class="website-orb website-orb-one" aria-hidden="true"/>
        <div class="website-orb website-orb-two" aria-hidden="true"/>
        <DotField
          class="website-dot-field"
          :dot-radius="1.45"
          :dot-spacing="17"
          :cursor-radius="390"
          :bulge-strength="31"
          gradient-from="rgba(105, 238, 199, 0.24)"
          gradient-to="rgba(174, 155, 255, 0.17)"
          glow-color="#07100f"
        />

        <div class="website-container website-hero-layout">
          <div class="website-hero-copy">
            <p class="website-kicker"><i/><span>AI FOR MINECRAFT OPERATIONS</span></p>
            <h1 id="hero-title">把开服的每一步，<strong>变成可掌控的流程。</strong></h1>
            <p class="website-hero-intro">Sculk Catalyst 将服务器创建、运行管理、资源整理与 AI 协作汇聚在一个工作台，让你更专注于服务器本身。</p>

            <div class="website-hero-actions">
              <a class="website-button website-button-primary" href="#downloads"><Download/>下载主机 Agent</a>
              <a class="website-button website-button-secondary" :href="cloudRegisterUrl"><Cloud/>注册云账号</a>
            </div>

            <ul class="website-hero-notes" aria-label="核心优势">
              <li><Check/>本地工作区由你掌控</li>
              <li><Check/>主机 Agent 采用出站连接</li>
              <li><Check/>团队远程操作支持审批</li>
            </ul>
          </div>

          <div class="website-product-frame">
            <div class="website-product-glow" aria-hidden="true"/>
            <div class="website-product-window">
              <div class="website-window-topbar"><span><i/><i/><i/></span><em>Sculk Catalyst · 控制中心</em><b>LIVE WORKSPACE</b></div>
              <img src="/website/sculk-console-v2.png" alt="Sculk Catalyst 服务器控制工作台界面"/>
            </div>
            <div class="website-floating-card website-floating-status"><span><RadioTower/><i/></span><p><small>HOST AGENT</small><b>已连接并待命</b></p></div>
            <div class="website-floating-card website-floating-task"><span><Sparkles/></span><p><small>AI TASK</small><b>将想法拆成可执行步骤</b></p></div>
          </div>
        </div>

        <div class="website-container website-signal-row" aria-label="功能概览">
          <p><Server/><span><b>服务器全流程</b><small>创建、初始化、运行与维护</small></span></p>
          <p><SquareTerminal/><span><b>实时操作面板</b><small>日志、终端和文件在同一处</small></span></p>
          <p><ShieldCheck/><span><b>可审查的协作</b><small>高风险远程操作需要确认</small></span></p>
        </div>
      </section>

      <section id="features" class="website-section website-capabilities" aria-labelledby="features-title">
        <div class="website-container">
          <header class="website-section-heading">
            <p class="website-kicker"><i/><span>ONE STUDIO, FULL CONTEXT</span></p>
            <div><h2 id="features-title">不止是开服面板，<br/>而是你的服务器工作台。</h2><p>从第一份配置到日常运营，关键的操作、信息与协作关系始终保持在可见范围内。</p></div>
          </header>

          <div class="website-capability-grid">
            <article v-for="item in capabilities" :key="item.index" class="website-capability-card" :class="`is-${item.tone}`">
              <header><span><component :is="item.icon"/></span><em>{{ item.index }}</em></header>
              <h3>{{ item.title }}</h3>
              <p>{{ item.body }}</p>
              <a href="#workflow">了解工作方式 <ChevronRight/></a>
            </article>
          </div>
        </div>
      </section>

      <section id="workflow" class="website-section website-workflow" aria-labelledby="workflow-title">
        <div class="website-container website-workflow-layout">
          <div class="website-workflow-copy">
            <p class="website-kicker"><i/><span>FROM IDEA TO RUNTIME</span></p>
            <h2 id="workflow-title">从一个想法，<br/>到稳定运行的服务器。</h2>
            <p>工作台将日常开服拆解成清晰的阶段：先完成环境和方案，再进入构建与运营。每一步都保留可见的状态、上下文和下一步动作。</p>
            <a class="website-text-link" :href="githubUrl" target="_blank" rel="noreferrer">查看项目源码 <ChevronRight/></a>
          </div>

          <ol class="website-flow-list">
            <li><span>01</span><div><i><FileCode2/></i><p><b>确定服务器方案</b><small>在对话与创建向导中整理核心、版本、端口和运行环境。</small></p></div></li>
            <li><span>02</span><div><i><Cpu/></i><p><b>初始化并进入工作区</b><small>检查 Java、准备工作目录、下载并校验服务端核心。</small></p></div></li>
            <li><span>03</span><div><i><Monitor/></i><p><b>持续管理运行状态</b><small>在控制面板查看日志、终端、文件与关键运行指标。</small></p></div></li>
            <li><span>04</span><div><i><Users/></i><p><b>按需连接云端协作</b><small>通过云账号连接团队、设备与远程操作的审批流程。</small></p></div></li>
          </ol>
        </div>
      </section>

      <section class="website-section website-control" aria-labelledby="control-title">
        <div class="website-container website-control-layout">
          <div class="website-control-visual">
            <div class="website-control-screen"><img src="/website/sculk-console-v2.png" alt="Sculk Catalyst 中的服务器状态和 AI 协作界面"/></div>
            <div class="website-control-metric"><span><HardDrive/></span><p><small>WORKSPACE</small><b>文件与资源始终有序</b></p></div>
          </div>
          <div class="website-control-copy">
            <p class="website-kicker"><i/><span>KEEP THE SIGNAL, DROP THE NOISE</span></p>
            <h2 id="control-title">每个决定，都能回到对应的工作上下文。</h2>
            <p>不必在终端、文件管理器、浏览器和聊天窗口之间来回切换。Sculk Catalyst 让状态、任务、文件和对话保持彼此关联。</p>
            <ul>
              <li><Check/><span><b>实时日志与终端</b><small>在运行状态下直接观察服务端输出并发送指令。</small></span></li>
              <li><Check/><span><b>安全文件操作</b><small>仅在服务器工作区内浏览、编辑、上传和下载。</small></span></li>
              <li><Check/><span><b>资源目录</b><small>集中管理服务端核心、插件、皮肤、模型与配置资源。</small></span></li>
            </ul>
          </div>
        </div>
      </section>

      <section class="website-section website-cloud-section" aria-labelledby="cloud-title">
        <div class="website-container website-cloud-panel">
          <div class="website-cloud-lines" aria-hidden="true"/>
          <div class="website-cloud-copy">
            <p class="website-kicker"><i/><span>SCULK CLOUD</span></p>
            <h2 id="cloud-title">把工作区带到协作需要发生的地方。</h2>
            <p>创建 Sculk Cloud 账号后，可在自己的设备与团队之间同步工作区设置，连接出站主机 Agent，并为高风险远程操作建立团队审批。</p>
            <div class="website-cloud-actions">
              <a class="website-button website-button-light" :href="cloudRegisterUrl"><Cloud/>创建云账号</a>
              <a class="website-text-link website-text-link-light" :href="cloudLoginUrl">登录 Cloud <ChevronRight/></a>
            </div>
          </div>
          <div class="website-cloud-map" aria-hidden="true">
            <div class="website-cloud-node website-cloud-node-main"><Box/></div>
            <div class="website-cloud-node website-cloud-node-left"><Monitor/></div>
            <div class="website-cloud-node website-cloud-node-right"><Server/></div>
            <div class="website-cloud-node website-cloud-node-bottom"><Users/></div>
            <span class="website-cloud-ring website-cloud-ring-one"/><span class="website-cloud-ring website-cloud-ring-two"/>
          </div>
        </div>
      </section>

      <section id="downloads" class="website-section website-downloads" aria-labelledby="downloads-title">
        <div class="website-container">
          <header class="website-section-heading website-downloads-heading">
            <p class="website-kicker"><i/><span>GET CONNECTED</span></p>
            <div><h2 id="downloads-title">下载主机 Agent，<br/>连接你的服务器环境。</h2><p>当前提供 Windows x64 与 Linux x64 版本。下载后完成配对，即可让你的主机安全地发起与 Cloud 的连接。</p></div>
          </header>

          <div class="website-download-layout">
            <div class="website-download-platforms" role="tablist" aria-label="选择下载平台">
              <button v-for="item in downloadOptions" :key="item.id" type="button" :class="{ active: selectedPlatform === item.id }" role="tab" :aria-selected="selectedPlatform === item.id" @click="selectPlatform(item.id)">
                <span><Monitor v-if="item.id === 'windows'"/><SquareTerminal v-else/></span><p><b>{{ item.platform }}</b><small>{{ item.architecture }}</small></p><ChevronRight/>
              </button>
            </div>
            <article class="website-download-card">
              <header><span><Download/></span><p><small>HOST AGENT</small><b>{{ selectedDownload.architecture }}</b></p></header>
              <p>{{ selectedDownload.description }}</p>
              <a class="website-button website-button-primary website-button-download" :href="selectedDownload.href" :download="selectedDownload.id === 'windows' ? 'sculk-agent-windows-x86_64.exe' : 'sculk-agent-linux-x86_64'">下载 {{ selectedDownload.platform }} 版本 <Download/></a>
              <footer><a href="/downloads/sculk-agent-SHA256SUMS.txt" target="_blank" rel="noreferrer">查看 SHA-256 校验文件 <ChevronRight/></a><span>仅支持 x64</span></footer>
            </article>
          </div>
        </div>
      </section>

      <section class="website-final-cta" aria-labelledby="cta-title">
        <div class="website-container website-final-cta-inner">
          <div><p class="website-kicker"><i/><span>START WITH YOUR WORLD</span></p><h2 id="cta-title">你的下一个服务器，<br/>从这里开始。</h2></div>
          <div class="website-final-actions"><a class="website-button website-button-primary" href="#downloads"><Download/>下载 Agent</a><a class="website-button website-button-secondary" :href="cloudRegisterUrl"><Cloud/>创建云账号</a></div>
        </div>
      </section>
    </main>

    <footer class="website-footer">
      <div class="website-container website-footer-inner">
        <a class="website-brand" href="#top"><span class="website-brand-mark"><Box/></span><span><b>Sculk Catalyst</b><small>AI Server Studio</small></span></a>
        <p>为 Minecraft 服务端而生的 AI 工作台。</p>
        <div><a :href="githubUrl" target="_blank" rel="noreferrer">GitHub</a><a :href="cloudLoginUrl">Sculk Cloud</a></div>
      </div>
    </footer>
  </div>
</template>
