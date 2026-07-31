export interface AiModel {
  id: string
  enabled: boolean
}

export interface AiProviderView {
  id: string
  name: string
  base_url: string
  enabled: boolean
  api_key_masked: string
  has_key: boolean
  models: AiModel[]
  models_synced_at?: string | null
}

export interface ModelBinding {
  provider_id: string
  model_id: string
  reasoning_effort?: ReasoningEffort | null
}

export type ReviewMode = 'approval' | 'auto' | 'full'
export type ReasoningEffort = 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max'
export type AgentTransport = 'acp' | 'cli'

export interface AiAgent {
  id: string
  name: string
  kind: string
  command: string
  args: string[]
  enabled: boolean
  transport?: AgentTransport
  reasoning_effort?: ReasoningEffort | null
}

export interface DetectedAgent {
  kind: 'codex' | 'claude-code'
  name: string
  installed: boolean
  available: boolean
  command: string
  path?: string | null
  version?: string | null
  transport: 'cli'
  capabilities: {
    reasoning_effort: {
      supported: boolean
      values: ReasoningEffort[]
    }
    acp: boolean
  }
  reason?: string | null
}

export interface AiSettingsView {
  providers: AiProviderView[]
  scenarios: Record<string, ModelBinding>
  default_binding?: ModelBinding | null
  review_mode: ReviewMode
  agents: AiAgent[]
  active_agent?: string | null
  reasoning_effort?: ReasoningEffort | null
  reasoning_effort_values?: ReasoningEffort[]
  detected_agents?: DetectedAgent[]
}

export interface TestResult {
  ok: boolean
  latency_ms: number
  reply?: string
  error?: string
}

export const SCENARIOS: { key: string; label: string; hint: string }[] = [
  { key: 'chat', label: '对话模型', hint: '日常 AI 对话与问题解答' },
  { key: 'speech', label: '语音合成模型', hint: '语音播报与 TTS 内容生成' },
  { key: 'setup', label: '服务器开服模型', hint: '核心选择、环境检查与部署建议' },
  { key: 'repair', label: '服务器报错修复模型', hint: '日志诊断、崩溃分析与自动修复' },
  { key: 'automation', label: '服务器管理模型', hint: '任务分析、巡检与自动化执行' },
  { key: 'config', label: '配置编写模型', hint: 'server.properties 与插件配置生成' },
  { key: 'community', label: '社区分析模型', hint: '玩家反馈聚类与运营内容' },
]

export const REVIEW_MODES: { key: ReviewMode; label: string; hint: string }[] = [
  { key: 'approval', label: '请求批准', hint: '中高风险任务都需要你人工批准后才会执行' },
  { key: 'auto', label: '替我审核', hint: 'AI 自动批准中风险任务，高风险仍需你确认' },
  { key: 'full', label: '完全访问权限', hint: '所有任务自动执行，不做审批拦截' },
]

export const REASONING_EFFORTS: { key: ReasoningEffort; label: string; hint: string }[] = [
  { key: 'minimal', label: '最小', hint: '优先响应速度，仅部分 Codex 模型支持' },
  { key: 'low', label: '低', hint: '适合简单问答和小范围修改' },
  { key: 'medium', label: '中', hint: '在质量、延迟与成本之间平衡' },
  { key: 'high', label: '高', hint: '适合复杂开发、诊断与多步任务' },
  { key: 'xhigh', label: '超高', hint: '用于更难的推理与长流程任务' },
  { key: 'max', label: '最高', hint: '仅支持该级别的模型或 Claude CLI 可用' },
]

export const AGENT_KINDS: { key: string; label: string; commandHint: string }[] = [
  { key: 'codex', label: 'Codex CLI', commandHint: '原生 CLI 通常填写 codex；如使用 ACP，请填写对应适配器命令与参数' },
  { key: 'claude-code', label: 'Claude Code CLI', commandHint: '原生 CLI 通常填写 claude；如使用 ACP，请填写对应适配器命令与参数' },
  { key: 'openclaw', label: 'OpenClaw', commandHint: 'openclaw（以 ACP stdio 模式启动的命令）' },
  { key: 'hermes', label: 'Hermes', commandHint: 'hermes（以 ACP stdio 模式启动的命令）' },
  { key: 'custom', label: '自定义 Agent', commandHint: '任何支持 ACP 协议（stdio JSON-RPC）的可执行命令' },
]

// ---------- UI 偏好（/api/ui/settings） ----------

export type BackgroundMode = 'solid' | 'gradient' | 'image'

export interface BackgroundSettings {
  mode: BackgroundMode
  solid: string
  gradient: string
  gradient_colors: string[]
  image_url: string
  image_opacity: number
}

export interface AppearanceSettings {
  preset: string
  accent: string
  background: BackgroundSettings
  font_family: string
  font_size: number
  font_color: string
}

export interface PersonalizationSettings {
  chat_style: string
  extra_context: string
}

export interface GitSettings {
  username: string
  email: string
  default_branch: string
  remote_url: string
  auto_commit: boolean
}

export interface AccountSettings {
  nickname: string
  email: string
}

export interface RemoteConnection {
  id: string
  name: string
  protocol: 'ssh' | 'sftp'
  host: string
  port: number
  username: string
  root_path: string
  enabled: boolean
}

export interface UiSettings {
  language: string
  appearance: AppearanceSettings
  personalization: PersonalizationSettings
  git: GitSettings
  account: AccountSettings
  connections: RemoteConnection[]
}

export const LANGUAGES: { key: string; label: string }[] = [
  { key: 'auto', label: '自动检测' },
  { key: 'zh-CN', label: '简体中文' },
  { key: 'en-US', label: 'English' },
]

export const CHAT_STYLES: { key: string; label: string; hint: string }[] = [
  { key: 'default', label: '默认', hint: '平衡的专业语气' },
  { key: 'concise', label: '简洁', hint: '只给结论与关键步骤' },
  { key: 'detailed', label: '详尽', hint: '补充原理与可选方案' },
  { key: 'humorous', label: '幽默', hint: '轻松语气，信息依然准确' },
  { key: 'formal', label: '正式', hint: '专业书面语气' },
  { key: 'custom', label: '自定义', hint: '用自己的话描述期望的风格' },
]

export interface IntegrationItem {
  id: string
  name: string
  kind: string
  status: string
  enabled: boolean
  endpoint: string
  latency_ms?: number
  capabilities: string[]
}

export interface SkillItem {
  id: string
  name: string
  description: string
  source: string
  enabled: boolean
  version: string
}
