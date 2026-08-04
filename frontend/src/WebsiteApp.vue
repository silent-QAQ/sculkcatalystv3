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
type DeploymentMode = 'local' | 'cloud'

const menuOpen = ref(false)
const isScrolled = ref(false)
const selectedDeploymentMode = ref<DeploymentMode>('local')
const selectedPlatform = ref<DownloadPlatform>('windows')
const cloudPortalUrl = String(import.meta.env.VITE_CLOUD_PORTAL_URL || 'https://sculk.mcmy.love').replace(/\/$/, '')
const cloudRegisterUrl = `${cloudPortalUrl}/?mode=register`
const cloudLoginUrl = `${cloudPortalUrl}/`
const githubUrl = 'https://github.com/silent-QAQ/sculkcatalystv3'
const localGuideUrl = `${githubUrl}/blob/main/README.md#本地开发`
const agentDocsUrl = `${githubUrl}/blob/main/docs/SCULK_AGENT.md`

const capabilities = [
  {
    index: '01',
    icon: Sparkles,
    title: 'AI 开服协作',
    body: '把服务器构想、配置取舍和创建步骤整理成可审查的任务与建议。',
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
  description: string
}> = [
  {
    id: 'windows',
    platform: 'Windows',
    architecture: 'Windows x64',
    description: '适用于 Windows x64 主机。启动包开放后，先在 Cloud 生成一次性配对码，再运行 Agent 完成连接。',
  },
  {
    id: 'linux',
    platform: 'Linux',
    architecture: 'Linux x64',
    description: '适用于 Linux x64 主机。启动包开放后，赋予执行权限，再按 Cloud 指引完成配对。',
  },
]

const selectedDownload = computed(() => downloadOptions.find(item => item.id === selectedPlatform.value) || downloadOptions[0])

function closeMenu() {
  menuOpen.value = false
}

function handleScroll() {
  isScrolled.value = window.scrollY > 24
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
  handleScroll()
  window.addEventListener('scroll', handleScroll, { passive: true })
})

onUnmounted(() => {
  window.removeEventListener('scroll', handleScroll)
  delete document.documentElement.dataset.sculkSurface
  delete document.body.dataset.sculkSurface
})
</script>

<template>
  <div class="website">
    <a class="website-skip-link" href="#main-content">跳到主要内容</a>

    <header class="website-header" :class="{ 'is-scrolled': isScrolled }" @keydown.esc="closeMenu">
      <a class="website-brand" href="#top" aria-label="Sculk Catalyst 首页" @click="closeMenu">
        <span class="website-brand-mark"><Box/></span>
        <span><b>Sculk Catalyst</b><small>AI Server Studio</small></span>
      </a>

      <button class="website-menu-toggle" type="button" :aria-expanded="menuOpen" aria-controls="website-navigation" aria-label="切换导航菜单" @click="menuOpen = !menuOpen">
        <X v-if="menuOpen"/><Menu v-else/>
      </button>

      <nav id="website-navigation" class="website-nav" :class="{ open: menuOpen }" aria-label="主导航">
        <a href="#features" @click="closeMenu">功能</a>
        <a href="#workflow" @click="closeMenu">工作方式</a>
        <a href="#downloads" @click="closeMenu">部署方式</a>
        <a :href="cloudLoginUrl" @click="closeMenu">登录 Cloud</a>
        <a class="website-nav-cta" :href="cloudRegisterUrl" @click="closeMenu">创建云账号 <ChevronRight/></a>
      </nav>
    </header>

    <main id="main-content">
      <section id="top" class="website-hero" aria-labelledby="hero-title">
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
            <h1 id="hero-title"><span class="website-hero-title-line">把开服的每一步，</span><strong class="website-hero-title-line">变成可掌控的流程。</strong></h1>
            <p class="website-hero-intro">Sculk Catalyst 将服务器创建、运行管理、资源整理与 AI 协作汇聚在一个工作台，让你更专注于服务器本身。</p>

            <div class="website-hero-actions">
              <a class="website-button website-button-primary" href="#downloads"><Download/>选择部署方式</a>
              <a class="website-button website-button-secondary" :href="cloudRegisterUrl"><Cloud/>注册云账号</a>
            </div>
            <a class="website-hero-local-link" :href="localGuideUrl" target="_blank" rel="noreferrer"><HardDrive/>只想本地运行？查看工作台指南 <ChevronRight/></a>

            <ul class="website-hero-notes" aria-label="核心优势">
              <li><Check/>本地工作区由你掌控</li>
              <li><Check/>主机 Agent 采用出站连接</li>
              <li><Check/>团队远程操作支持审批</li>
            </ul>

            <div class="website-hero-proof" role="group" aria-label="产品边界">
              <span><b>LOCAL-FIRST</b><small>工作区先留在你的主机</small></span>
              <span><b>OUTBOUND ONLY</b><small>Agent 不监听入站端口</small></span>
              <span><b>SOURCE AVAILABLE</b><small>源码与构建可以审查</small></span>
            </div>
          </div>

          <div class="website-product-frame">
            <div class="website-product-glow" aria-hidden="true"/>
            <div class="website-product-window">
              <div class="website-window-topbar"><span><i/><i/><i/></span><em>Sculk Catalyst · 控制中心</em><b>PRODUCT PREVIEW</b></div>
              <img src="/website/sculk-console-v2.png" width="1868" height="1290" alt="Sculk Catalyst 服务器控制工作台界面示意" fetchpriority="high"/>
            </div>
            <div class="website-floating-card website-floating-status"><span><RadioTower/><i/></span><p><small>HOST AGENT</small><b>连接状态示意</b></p></div>
            <div class="website-floating-card website-floating-task"><span><Sparkles/></span><p><small>AI WORKFLOW</small><b>任务拆解示意</b></p></div>
          </div>
        </div>

        <div class="website-container website-signal-row" role="group" aria-label="功能概览">
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
            <div class="website-control-screen"><img src="/website/sculk-console-v2.png" width="1868" height="1290" loading="lazy" alt="Sculk Catalyst 中的服务器状态和 AI 协作界面示意"/></div>
            <div class="website-control-metric"><span><HardDrive/></span><p><small>WORKSPACE</small><b>文件与资源始终有序</b></p></div>
          </div>
          <div class="website-control-copy">
            <p class="website-kicker"><i/><span>KEEP THE SIGNAL, DROP THE NOISE</span></p>
            <h2 id="control-title">每个决定，都能回到对应的工作上下文。</h2>
            <p>不必在终端、文件管理器、浏览器和聊天窗口之间来回切换。Sculk Catalyst 让状态、任务、文件和对话保持彼此关联。</p>
            <ul>
              <li><Check/><span><b>实时日志与终端</b><small>在运行状态下直接观察服务端输出并发送指令。</small></span></li>
              <li><Check/><span><b>结构化文件操作</b><small>常规文件浏览、编辑、上传和下载限定在服务器工作区内。</small></span></li>
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
            <p class="website-kicker"><i/><span>CHOOSE YOUR PATH</span></p>
            <div><h2 id="downloads-title">选择部署方式，<br/>从适合你的路径开始。</h2><p>本地部署适合单机管理与开发验证；云端使用适合连接远程主机、团队协作和审批。</p></div>
          </header>

          <div class="website-download-layout">
            <div class="website-download-selector">
              <label class="website-select-field">
                <span>使用方式</span>
                <select v-model="selectedDeploymentMode" class="website-select-control" aria-label="选择使用方式" aria-describedby="deployment-mode-note">
                  <option value="local">本地部署 · 本地工作台</option>
                  <option value="cloud">云端使用 · Cloud + Agent</option>
                </select>
              </label>
              <label v-if="selectedDeploymentMode === 'cloud'" class="website-select-field">
                <span>主机平台</span>
                <select v-model="selectedPlatform" class="website-select-control" aria-label="选择主机平台" aria-describedby="deployment-mode-note">
                  <option v-for="item in downloadOptions" :key="item.id" :value="item.id">{{ item.platform }} · {{ item.architecture }}</option>
                </select>
              </label>
              <p v-if="selectedDeploymentMode === 'local'" id="deployment-mode-note" class="website-selector-note" aria-live="polite"><HardDrive/><span>本地部署不需要下载主机 Agent。按指南启动 Rust 后端和 Vue 工作台，服务器数据保留在本机。</span></p>
              <p v-else id="deployment-mode-note" class="website-selector-note" aria-live="polite"><Cloud/><span>先<a :href="cloudLoginUrl">登录 Cloud</a>或<a :href="cloudRegisterUrl">创建云账号</a>，生成一次性配对码；启动包开放后，在此选择对应平台下载 Agent。</span></p>
            </div>
            <article id="download-panel" class="website-download-card" :class="{ 'is-local': selectedDeploymentMode === 'local' }" aria-live="polite">
              <template v-if="selectedDeploymentMode === 'local'">
                <header><span><HardDrive/></span><p><small>LOCAL WORKSPACE</small><b>本地部署</b></p></header>
                <p>适合单机管理、开发和功能验证。后端默认绑定本机地址，前端工作台直接连接本地服务。</p>
                <a class="website-button website-button-primary website-button-download" :href="localGuideUrl" target="_blank" rel="noreferrer">查看本地部署指南 <ChevronRight/></a>
                <footer><a :href="`${githubUrl}/blob/main/README.md#环境要求`" target="_blank" rel="noreferrer">查看环境要求 <ChevronRight/></a><span>自托管</span></footer>
              </template>
              <template v-else>
                <header><span><Download/></span><p><small>HOST AGENT</small><b>{{ selectedDownload.architecture }}</b></p></header>
                <p>{{ selectedDownload.description }}</p>
                <button class="website-button website-button-primary website-button-download" type="button" disabled aria-disabled="true" aria-describedby="download-unavailable-note">下载 {{ selectedDownload.platform }} 版本 <Download/></button>
                <footer><span id="download-unavailable-note" class="website-download-placeholder">下载入口即将开放</span><span>仅支持 x64</span></footer>
              </template>
            </article>
          </div>

          <div id="install" class="website-install-rail" :class="{ 'is-local': selectedDeploymentMode === 'local' }" aria-live="polite">
            <div class="website-install-intro">
              <p class="website-kicker"><i/><span>{{ selectedDeploymentMode === 'local' ? 'RUN LOCALLY' : 'PAIR IN A FEW STEPS' }}</span></p>
              <h3>{{ selectedDeploymentMode === 'local' ? '本地工作台，三步启动。' : '启动包开放后，三步连接主机。' }}</h3>
              <p>{{ selectedDeploymentMode === 'local' ? '准备运行环境、启动服务，再打开本地工作台管理服务器。' : '配对码只在 Cloud 控制台生成，确认指纹后，主机才会开始接收任务。' }}</p>
              <a class="website-text-link website-install-doc-link" :href="selectedDeploymentMode === 'local' ? localGuideUrl : `${agentDocsUrl}#配对`" target="_blank" rel="noreferrer">{{ selectedDeploymentMode === 'local' ? '查看本地运行文档' : '查看完整配对文档' }} <ChevronRight/></a>
            </div>
            <ol v-if="selectedDeploymentMode === 'local'" class="website-install-steps">
              <li><span>01</span><div><b>准备运行环境</b><small>安装 Rust、Node.js；运行 Minecraft 服务端时准备 Java。</small></div></li>
              <li><span>02</span><div><b>启动本地服务</b><small>按指南启动 Rust 后端和 Vue 前端，使用本机地址访问工作台。</small></div></li>
              <li><span>03</span><div><b>创建并管理服务器</b><small>在工作台完成核心下载、配置、启动、日志和文件操作。</small></div></li>
            </ol>
            <ol v-else class="website-install-steps">
              <li><span>01</span><div><b>创建一次性配对码</b><small>登录 Cloud，进入“主机代理”并连接新主机。</small></div></li>
              <li><span>02</span><div><b>在主机运行 Agent</b><small>填写工作区路径与配对码，Agent 主动发起 HTTPS 连接。</small></div></li>
              <li><span>03</span><div><b>核对指纹并确认</b><small>确认控制台和终端中的指纹一致，再开始远程协作。</small></div></li>
            </ol>
          </div>
        </div>
      </section>

      <section class="website-section website-faq" aria-labelledby="faq-title">
        <div class="website-container website-faq-layout">
          <div>
            <p class="website-kicker"><i/><span>BEFORE YOU CONNECT</span></p>
            <h2 id="faq-title">开始之前，先把边界说清楚。</h2>
            <p class="website-faq-lead">你可以只使用本地工作台，也可以在需要团队协作时接入 Cloud。</p>
          </div>
          <div class="website-faq-list">
            <details open>
              <summary>本地工作台和 Sculk Cloud 如何配合？<ChevronRight/></summary>
              <p>本地工作台负责服务器创建与日常管理；Cloud 用于账号、设备、团队和远程审批。两者可以按需组合。</p>
            </details>
            <details>
              <summary>Agent 会开放公网端口吗？<ChevronRight/></summary>
              <p>不会。Agent 主动发起 HTTPS 连接，不监听入站端口；实际 Shell 权限仍取决于启动它的操作系统账户。</p>
            </details>
            <details>
              <summary>支持哪些主机系统？<ChevronRight/></summary>
              <p>启动包开放后将优先支持 Windows x64 和 Linux x64。Linux 版本计划采用静态链接构建，适合长期运行的服务器主机。</p>
            </details>
          </div>
        </div>
      </section>

      <section class="website-final-cta" aria-labelledby="cta-title">
        <div class="website-container website-final-cta-inner">
          <div><p class="website-kicker"><i/><span>START WITH YOUR WORLD</span></p><h2 id="cta-title">你的下一个服务器，<br/>从这里开始。</h2></div>
          <div class="website-final-actions"><a class="website-button website-button-primary" href="#downloads"><Download/>选择部署方式</a><a class="website-button website-button-secondary" :href="cloudRegisterUrl"><Cloud/>创建云账号</a></div>
        </div>
      </section>
    </main>

    <footer class="website-footer">
      <div class="website-container website-footer-inner">
        <a class="website-brand" href="#top"><span class="website-brand-mark"><Box/></span><span><b>Sculk Catalyst</b><small>AI Server Studio</small></span></a>
        <p>为 Minecraft 服务端而生的 AI 工作台。</p>
        <div><a :href="`${githubUrl}/blob/main/docs/SCULK_AGENT.md`" target="_blank" rel="noreferrer">Agent 文档</a><a :href="`${githubUrl}/blob/main/NOTICE`" target="_blank" rel="noreferrer">许可说明</a><span class="website-footer-pending">启动包即将开放</span><a :href="cloudLoginUrl">Sculk Cloud</a></div>
      </div>
    </footer>
  </div>
</template>
