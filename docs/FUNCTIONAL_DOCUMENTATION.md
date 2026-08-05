# Sculk Catalyst V3 功能文档

> 文档版本：V3.0 功能说明  
> 更新时间：2026-08-01
> 产品阶段：可交互 MVP / 技术验证原型  
> 适用范围：本地开服工作台、资源中心、可选 Sculk Cloud 服务

## 简介

Sculk Catalyst V3 是一套面向 Minecraft 服务器运营者、开发者和社区管理员的 AI 驱动开服管理工作台。它将服务器创建、核心下载、Java 环境检查、配置编辑、进程启停、实时日志、AI 对话、自动化任务、资源目录和社区运营入口集中在一个三栏式界面中。

项目由 Vue 3 + TypeScript + Vite 前端和 Rust + Axum 后端组成。前端负责工作台交互、向导、资源库和设置页面；后端负责服务器工作区、Java 子进程、文件安全边界、下载校验、持久化状态、AI 上游连接以及可选的 Cloud 数据服务。

当前版本适合本机可信环境下的功能验证和单机管理，不应直接视为具备完整身份认证、远程运维、插件自动安装和生产级容灾能力的公有云开服平台。文档中将能力分为三类：

- **已实现**：代码和界面均具备可调用流程。
- **配置依赖**：代码已提供，但必须配置外部服务、运行时或环境变量后才可用。
- **占位/演示**：界面或 API 已预留，但目前只更新本地状态、返回模拟结果或明确返回 planned/501。

## 目录

1. [产品定位与总体架构](#第1章-产品定位与总体架构)
2. [应用入口与工作台导航](#第2章-应用入口与工作台导航)
3. [服务器项目与控制中心](#第3章-服务器项目与控制中心)
4. [首次开服向导与 Java 运行时](#第4章-首次开服向导与-java-运行时)
5. [核心下载、镜像与安装校验](#第5章-核心下载镜像与安装校验)
6. [对话任务与 AI 交互](#第6章-对话任务与-ai-交互)
7. [AI 自动化与审批模式](#第7章-ai-自动化与审批模式)
8. [文件管理、终端与实时日志](#第8章-文件管理终端与实时日志)
9. [资源中心与版本管理](#第9章-资源中心与版本管理)
10. [资源同步与 MSL 核心同步](#第10章-资源同步与-msl-核心同步)
11. [玩家社区与运营功能](#第11章-玩家社区与运营功能)
12. [Skills、MCP 与 ACP Agent](#第12章-skillsmcp-与-acp-agent)
13. [设置中心](#第13章-设置中心)
14. [Sculk Cloud 云服务](#第14章-sculk-cloud-云服务)
15. [数据模型、通信方式与持久化](#第15章-数据模型通信方式与持久化)
16. [API 功能清单](#第16章-api-功能清单)
17. [运行、部署与环境配置](#第17章-运行部署与环境配置)
18. [安全边界、限制与当前缺口](#第18章-安全边界限制与当前缺口)
19. [典型用户流程](#第19章-典型用户流程)
20. [验证基线与后续路线](#第20章-验证基线与后续路线)

---

## 第1章 产品定位与总体架构

### 1.1 产品目标

项目围绕 Minecraft 服务器的完整生命周期提供统一入口：

1. 创建本地服务器项目并选择服务端核心、Minecraft 版本、端口和内存。
2. 检查 Java、系统架构、磁盘空间和工作区可写性。
3. 下载并校验核心文件，生成服务器工作区和启动脚本。
4. 启动、停止、重启服务器并查看实时日志。
5. 浏览和编辑 server.properties、脚本、配置、日志及 Markdown 文件。
6. 通过对话和 SSE 流式输出获取服务器运维建议。
7. 以风险等级管理自动化任务，并为高风险操作保留审批入口。
8. 管理核心、插件、皮肤、BBModel、UI 贴图、Skill 和插件配置资源。
9. 提供玩家、反馈、经济和玩法投票等社区运营面板。
10. 通过 Sculk Cloud、MCP、Skills 和 ACP Agent 扩展协作能力。

### 1.2 系统组成

~~~mermaid
flowchart LR
    UI[Vue 三栏工作台] --> API[Rust Axum API]
    UI --> SSE[SSE 流式对话]
    UI --> WS[WebSocket 日志]
    API --> STATE[state.json / state-cloud.json]
    API --> WORKSPACE[data/servers/{id}]
    API --> JAVA[Java Minecraft 进程]
    API --> CATALOG[七类资源目录]
    API --> DOWNLOAD[核心下载执行器]
    API --> CLOUD[可选 PostgreSQL + Redis]
    CATALOG --> OBJECTS[资源对象目录 / 外部制品]
    DOWNLOAD --> CATALOG
    DOWNLOAD --> UPSTREAM[镜像或官方源]
~~~

### 1.3 技术栈

| 层级 | 技术 | 主要职责 |
| --- | --- | --- |
| 前端 | Vue 3、TypeScript、Vite | 页面、向导、状态和用户交互 |
| UI | lucide-vue-next、项目 CSS | 图标、暗色工作台和模块化页面样式 |
| 后端 | Rust、Axum、Tokio | HTTP API、异步文件、进程、并发和 WebSocket |
| 外部 HTTP | Reqwest + rustls | AI、资源中心、GitHub、核心源和 Java 下载 |
| 本地持久化 | Serde JSON | 服务器、任务、对话、目录和设置状态 |
| 云端持久化 | PostgreSQL + SQLx | 账号、团队、审批、凭据和用量事实 |
| 云端短期状态 | Redis | 会话缓存和 API 分钟级限流 |
| 安全与校验 | Argon2、AES-256-GCM、SHA-256 | 密码哈希、云端凭据加密、文件完整性校验 |

主要实现入口：

- 前端主工作台：frontend/src/App.vue
- 资源中心页面：frontend/src/features/mirror/MirrorCenterView.vue
- 后端路由与本地服务器：backend/src/main.rs
- AI 与 SSE：backend/src/ai.rs
- 对话持久化：backend/src/conversations.rs
- 资源目录：backend/src/catalog.rs
- 核心下载：backend/src/download.rs
- Cloud：backend/src/cloud.rs

---

## 第2章 应用入口与工作台导航

### 2.1 页面入口

前端入口在 frontend/src/main.ts：

- 默认路径加载主工作台 App.vue。
- /resource-admin 加载独立资源源站管理页 ResourceAdminApp.vue。
- 不使用 Vue Router；主页面通过 surface 和 tab 状态切换。
- 不使用 Pinia/Vuex；页面状态主要由组件 ref/computed 管理，共享设置通过 features/settings/store.ts 管理。

### 2.2 主导航

左侧栏提供以下工作区：

| 工作区 | 功能 |
| --- | --- |
| 控制中心 | 服务器列表、对话树、状态、文件和终端 |
| 镜像仓库 | 七类资源浏览、版本管理和 API 文档 |
| AI 自动化 | 任务、风险、审批、进度和取消 |
| 玩家社区 | 玩家、反馈、投票和经济指标 |
| Skills & MCP | 外部连接、能力包和资源同步 |
| 设置 | AI、Agent、外观、个性化、Cloud、Git 和远程连接 |

控制中心内部包含三个标签：

- **总览**：服务器基本信息、性能卡片、任务和操作入口。
- **文件**：工作区目录浏览、文本编辑和新建目录。
- **终端**：命令输入、历史日志和 WebSocket 实时输出。

### 2.3 页面数据加载

主工作台启动后会读取：

- GET /api/dashboard：服务器和任务列表。
- GET /api/system：Java、系统架构、磁盘和内存。
- GET /api/catalog/cores：创建向导所需的核心与版本。
- GET /api/ai/settings：模型、Agent 和审核模式。
- GET /api/ui/settings：外观、语言、个性化和连接配置。

控制中心默认约每 2 秒刷新一次 dashboard；文件页和终端页根据当前标签切换读取策略，终端页建立 WebSocket 后以实时推送为主，连接断开时回退到轮询。

---

## 第3章 服务器项目与控制中心

### 3.1 服务器项目数据

每个服务器项目至少包含：

- 项目 ID、名称和位置。
- 服务端核心和 Minecraft 版本。
- 状态、玩家数、CPU、内存、端口和当前任务。
- memory_gb 最大内存配置。
- Java 进程 PID、运行实例标识和启动时间。
- operation_state、core_ready 和 last_error：分别表示当前互斥操作、核心文件事实与最近失败原因。
- 独立对话、任务、配置和日志索引。

服务器本地工作区位于 `SCULK_DATA_DIR/servers/{server_id}`；未设置变量时数据根目录默认为后端工作目录下的 `data`。新建后默认包含：

~~~text
data/servers/{server_id}/
├─ plugins/
├─ logs/
├─ server.properties
├─ eula.txt
├─ start.ps1
├─ start.sh
└─ server.jar              # 核心下载完成后出现
~~~

### 3.2 服务器生命周期

#### 普通创建

POST /api/servers 负责创建项目和工作区，并返回随后立即执行的持久化 `provision_task`。创建过程会：

1. 检查名称非空、位置合法、端口未占用。
2. 检查 EULA 已确认。
3. 校验内存范围。
4. 建立 plugins/ 和 logs/。
5. 生成 server.properties、eula.txt、start.ps1 和 start.sh；Unix 上的 start.sh 权限为 0755。
6. 写入并派发 `server_provision` 初始化任务。
7. 由任务检查工作区和 EULA，下载、校验并原子安装 server.jar，再检查 Java 兼容性。
8. 将进度、事件、错误、项目、配置和日志持久化；失败或取消后可通过 POST /api/servers/{id}/provision 重试。

初始化中后端退出时，同一任务会在下次启动时回到队列。已经原子安装完成的 server.jar 会直接复用；未完成的 `.part` 不作为可信核心，传输会从头开始。

#### 打开已有目录

控制中心的“打开已有目录”可直接接管本机已有的服务器或通用项目目录。请求 `POST /api/servers/import`（兼容别名 `POST /api/workspaces/open`）时传入本机绝对路径和工作区类型。服务器目录接入时只读扫描根目录的 `server.properties`、`eula.txt`、核心 JAR 和启动脚本，并自动读取端口、最大玩家数、核心/版本提示以及脚本中的一致 `-Xmx` 内存值；不会执行脚本或写入 `sculk.yml`。启动前会重新读取配置并要求 `eula=true`，外部目录只允许从列表移除，不会被工作台删除磁盘文件。

#### 智能规划创建

POST /api/servers/plan 创建一个 planning 状态项目和“开服规划”对话，但不创建服务器目录和文件。规划对话会询问目标玩法、玩家数量和版本偏好，再由后续方案决定核心和部署参数。

#### 启动

启动前必须同时满足：

- server.jar 存在。
- 最近一次首次初始化任务已经完成，且 operation_state 为 idle、core_ready 为 true。
- Java 已安装且主版本兼容，当前推荐 Java 21。
- 同一服务器没有正在执行的核心下载。
- 同一服务器没有已经运行的 Java 进程。
- 内存配置在 2–64 GB 范围内。

后端以工作区为当前目录启动 Java，传入：

~~~text
-Xms{min(memory_gb, 2)}G -Xmx{memory_gb}G -jar server.jar nogui
~~~

启动后会接管 Java 的 stdin/stdout/stderr，更新服务器状态、PID、运行实例标识和日志。

#### 停止与删除

- POST /api/servers/{id}/action，action=start|stop|restart|force_stop。
- 停止优先向 Java stdin 写入 stop，等待进程正常退出，并在必要时使用强制终止回退。
- DELETE /api/servers/{id} 默认只删除项目和状态记录。
- 需要删除磁盘工作区时，必须设置 delete_files=true，并提交精确确认文本 delete all。
- 删除前会检查目标路径仍位于 data/servers 内，避免误删其他目录。

### 3.3 总览信息

总览展示服务器名称、核心、版本、端口、玩家数、CPU、内存、TPS、状态和当前任务。服务器项目支持多项目并列管理，每个项目拥有独立工作区、进程、下载状态、日志广播通道和对话树。

当前 CPU、TPS、玩家和经济数据主要是本地状态或演示数据，未接入 RCON、Query、管理插件或代理网络的实时数据源。

### 3.4 对话树

每台服务器支持多个持久化对话任务：

- 新建、自动命名和重命名。
- 分组、固定、归档。
- 标记已读/未读。
- 分叉对话并复制历史。
- 删除对话。
- 按服务器记忆最近选择的对话。
- 对话级保存模型绑定和 Agent 覆盖。

每个对话最多保留 500 条消息；对话分叉会复制历史，但重置新对话的置顶、未读和归档状态。

---

## 第4章 首次开服向导与 Java 运行时

### 4.1 四步创建向导

| 步骤 | 内容 | 主要校验 |
| --- | --- | --- |
| 1. 名称与位置 | 项目名称、本地或远程位置 | 名称非空；远程位置当前禁用 |
| 2. 服务器参数 | 核心、Minecraft 版本、内存、端口 | 核心和版本存在；端口 1024–65535；内存 2–64 GB |
| 3. 环境检查 | Java、架构、可写性、磁盘、系统内存 | Java 兼容；目录可写；至少约 2 GB 可用空间 |
| 4. 确认创建 | EULA、参数复核和创建 | 必须明确接受 Minecraft EULA |

核心和版本优先来自资源目录；目录不可用时前端使用内置 Paper、Purpur、Fabric、Velocity 选项作为回退。所选核心的 Minecraft 兼容版本会动态刷新版本下拉框。

### 4.2 Java 检测

GET /api/system 返回：

- Java 是否安装。
- Java 版本和主版本。
- Java 可执行文件路径和 JAVA_HOME。
- 操作系统和系统架构。
- 数据目录可写性。
- 可用磁盘空间和总内存。
- 推荐 Java 主版本和可用核心列表。

Java 查找顺序为 SCULK_JAVA_BIN、项目托管运行时、JAVA_HOME 和系统 PATH。

### 4.3 托管 Java 安装

POST /api/runtime/java/install 当前支持安装推荐 Java 21，覆盖 Windows x64、Linux x64 和 Linux ARM64，并通过 Eclipse Adoptium JRE 资源下载、校验和解压。

安装过程包含：

- 同一时间只允许一个 Java 安装任务。
- 下载包的大小和 SHA-256 校验。
- ZIP/tar.gz 条目数量、解压大小、绝对路径、`..`、符号链接、硬链接与特殊文件检查。
- Linux tar.gz 解压后保留 `bin/java` 的可执行权限。
- 使用临时目录和原子替换切换托管运行时。
- 失败时清理临时文件，不覆盖现有有效运行时。

其他平台会返回能力提示，并允许通过系统包管理器、SCULK_JAVA_BIN、JAVA_HOME 或 PATH 使用已有 Java。Java 安装属于配置依赖能力，必须允许访问 Adoptium 上游。

### 4.4 内存和端口规则

- 单服内存最小 2 GB，最大 64 GB。
- 前端会根据系统总内存保留约 20% 的安全余量。
- 后端启动参数和生成的 start.ps1、start.sh 使用相同的 -Xms/-Xmx 规则。
- 创建时检查已登记服务器的端口冲突；启动时还会执行端口可用性检查。

---

## 第5章 核心下载、镜像与安装校验

### 5.1 下载入口

核心下载专用于把所选核心安装为某个服务器工作区中的 server.jar：

- POST /api/servers/{id}/provision：创建、重试或幂等读取首次初始化任务。
- POST /api/servers/{id}/download/core：创建后台下载任务。
- GET /api/servers/{id}/download/status：读取阶段、来源、字节数和百分比。
- POST /api/servers/{id}/download/cancel：请求取消。
- GET /api/download/mirrors：读取镜像配置。
- POST /api/download/preview：预览镜像候选地址和优先级。

### 5.2 来源选择顺序

下载器按照以下策略构造来源：

1. 资源目录中与核心、Minecraft 版本和 stable 渠道匹配的已发布版本。
2. 已启用、支持该核心且 URL 不是占位地址的镜像，按 priority 升序尝试。
3. Paper 或 Velocity 的 PaperMC API。
4. Purpur 的 PurpurMC API。

资源目录优先是为了尽可能使用有可信文件大小和 SHA-256 的制品。预留的 example.com 镜像不会进入真实执行队列。

### 5.3 下载执行流程

~~~mermaid
flowchart TD
    A[解析核心与版本] --> B[生成来源队列]
    B --> C[解析当前来源]
    C --> D[写入 server.jar.part]
    D --> E{传输成功?}
    E -- 否且可重试 --> D
    E -- 否且不可用 --> F[清理临时文件并切换来源]
    F --> C
    E -- 是 --> G[校验大小与 SHA-256]
    G -- 失败 --> F
    G -- 通过 --> H[安全替换 server.jar]
    H --> I[更新任务、日志和目录下载计数]
~~~

实现特性：

- 后台异步执行，持续更新任务进度和服务器任务文字。
- 传输失败、超时、服务端错误最多进行 3 次重试。
- 使用 server.jar.part，完成后再安装为 server.jar。
- 支持 HTTP Range 和 Content-Range 解析，处理传输中断时的同一任务内续传。
- 计算 SHA-256；资源目录制品同时强制校验可信大小和摘要。
- 通过后以可回滚方式替换旧核心，安装失败尝试恢复旧文件。
- 下载和启动对同一服务器互斥，不能重复启动下载或在下载中启动 Java。
- 取消、失败和来源切换会清理临时文件。

### 5.4 下载边界

- 后端重启后不会恢复内存中的下载状态和取消标记。
- 新任务开始前会清除旧 .part，因此不承诺跨重启续传。
- 自定义镜像和部分官方源可能没有可信预期摘要，系统仍会计算实际摘要，但不能进行预期值比对。
- 资源中心的稳定下载接口是 HTTP 307 重定向，核心下载器和资源中心下载不是同一条执行链。

---

## 第6章 对话任务与 AI 交互

### 6.1 两个对话入口的区别

| 接口 | 作用 | 当前性质 |
| --- | --- | --- |
| POST /api/chat | 根据关键词识别意图，创建任务并返回规则回复 | 本地规则流程 |
| POST /api/chat/stream | 连接配置的 OpenAI 格式模型或 ACP Agent，返回 SSE 流 | 实际 AI 对话入口 |

文档、客户端和故障判断应使用 /api/chat/stream 作为真正的 AI 对话接口；/api/chat 保留为简单的兼容/规则入口。

### 6.2 SSE 事件

POST /api/chat/stream 请求可包含：

~~~json
{
  "server_id": "server-1234abcd",
  "conversation_id": "conv-1234abcd",
  "message": "分析最近的启动日志",
  "history": [],
  "model_override": null,
  "agent_override": null
}
~~~

前端处理以下事件：

- meta：实际提供商、模型和是否使用 fallback。
- delta：文本增量，形成打字机输出。
- error：上游响应中断或 Agent 错误。
- done：完成时间、建议动作、关联任务和对话 ID。

对话结束后，用户消息、助手回复和建议动作会追加到持久化对话；上下文只向上游保留最近的合法 user/assistant 内容，总字符约不超过 16,000。

### 6.3 AI 提供商

设置页支持多个 OpenAI 格式提供商：

- 提供商名称、HTTP(S) base_url、API Key 和启用状态。
- base_url 末尾可带或不带 /v1，后端会统一拼接。
- 从上游 /v1/models 同步模型。
- 新同步模型默认关闭，既有模型保留启用状态。
- 支持手动添加、删除和启停模型。
- 对单个 provider + model 发送 hi 连通性测试，返回延迟和回复片段。
- API 返回只显示掩码密钥；本地 state.json 仍保存原始 API Key。

模型生效顺序：

1. 当前请求的模型覆盖。
2. 当前 AI 场景绑定。
3. 全局默认模型。
4. 无可用模型时回退本地规则回复。

后端支持的场景键包括 chat、automation、setup、config、community、speech 和 repair；界面主要围绕对话、管理/自动化、开服向导、配置编写和社区分析展示。

### 6.4 Fallback 行为

以下情况会进入本地规则回复：

- 没有启用的 provider/model。
- 上游连接失败或返回非成功状态。
- 外部 ACP Agent 启动或握手失败。

本地回复仍以 SSE 分片发送，并在 meta 事件中标记 fallback=true。规则意图包括故障修复、投票、宣传和插件交付等。

---

## 第7章 AI 自动化与审批模式

### 7.1 任务属性

自动化任务包含：

- 任务 ID、服务器 ID、标题和类型。
- low、medium、high 风险等级。
- queued、running、completed、failed、cancelled 等状态。
- 进度百分比、创建时间和批准来源。

可通过 GET /api/automation 查看任务、待批准数量和运行数量；通过 POST /api/automation/tasks 创建任务。

当前界面内置入口包括智能诊断、创建备份、经济调控和部署类任务。这个本地 JSON 自动化模块仍不是通用工具执行器；Cloud Agent 的远程任务是独立链路，拥有自己的 PostgreSQL 任务、租约、事件和审批模型。

### 7.2 三档审核模式

| 模式 | 低风险 | 中风险 | 高风险 | 记录 |
| --- | --- | --- | --- | --- |
| 请求批准 approval | 自动运行 | 等待批准 | 等待批准 | 用户批准记为 user |
| 替我审核 auto | 自动运行 | AI 自动批准 | 等待批准 | 中风险记为 ai |
| 完全访问 full | 自动运行 | 自动运行 | 自动运行 | 自动执行记为 auto |

审核模式全局持久化，只影响本项目受管自动化任务的审批流。外部 ACP Agent 的权限请求始终拒绝，不能由审核模式隐式授予能力。full 模式会在 UI 中显示高风险提示。

原生 Codex CLI 复用当前对话的 Agent 选择与 SSE 流程，但权限单独受控：`approval` 与 `auto` 固定使用只读沙盒；`full` 只有在后端仅监听回环地址、启动环境设置 `SCULK_ALLOW_CODEX_FULL=true`，并通过 `SCULK_CODEX_TRUSTED_COMMAND` 指定与当前 Agent 命令一致的绝对 Codex 可执行文件时才会启用。完整权限使用 `--ask-for-approval never` 与 `--sandbox danger-full-access`，不会使用全局 bypass 标志；它以运行后端的本机账户访问宿主机，当前项目或服务器目录只作为初始工作目录。完全访问模式下仅允许受信任的原生 Codex CLI，其他外部 Agent 会被拒绝执行。

### 7.3 当前执行边界

本地审批状态本身已经落地，但模型回复不会自动变成本地文件、终端、停服或经济操作。Cloud Agent 远程操作则必须经过服务端任务权限和租约校验；下一阶段仍需要为本地自动化接入带权限声明、审计、幂等、回滚和人工确认门的通用工具执行器。

### 7.4 Cloud Agent 任务审批

Cloud Agent 使用独立的风险与权限模型：

- `read + low` 的只读任务可以直接进入队列。
- `write + high` 和 `full + critical` 任务必须指定所属团队，并在创建任务时生成唯一的关联审批记录。
- 请求人不能处理自己的审批；只有该团队的 `owner`、`admin` 或 `approver` 可以通过或拒绝。
- 通用审批决定会在同一事务中更新审批和任务：通过后任务进入 `queued`，拒绝后任务进入 `cancelled`；租约查询还会再次核对审批 ID、团队、请求人、决定人和决定人的当前团队角色。
- 重试、从检查点恢复和回滚都会创建新的任务与新的审批，旧任务的批准不会沿用。
- 持久终端创建后先等待审批；只有审批通过后，Agent 才能领取 `start` 命令。

---

## 第8章 文件管理、终端与实时日志

### 8.1 文件管理

文件工作区提供：

- 目录浏览和返回上级。
- UTF-8 文本读取。
- 文本编辑和保存。
- 新建目录。
- server.properties 快捷打开。
- 文件大小、修改时间、文件夹/文件类型和只读标识。
- 独立上传与下载：上传字段支持相对路径和文件内容，单文件上限 256 MiB；上传先写入随机临时文件，再原子移动到目标。
- 上传默认不覆盖既有文件；服务器根目录的 `server.jar`、`server.jar.part` 和 `server.jar.backup` 始终受保护。

API：

- GET /api/servers/{id}/files?path=...
- GET /api/servers/{id}/file?path=...
- PUT /api/servers/{id}/file
- POST /api/servers/{id}/file/upload（multipart 上传）
- GET /api/servers/{id}/file/download?path=...
- POST /api/servers/{id}/directory

编辑器默认信任工作区内的安全路径，支持无扩展名文件、LICENSE、EULA 和项目自定义扩展名，不再用扩展名白名单阻止创建。单文件读写上限约 2 MB；无法按 UTF-8 解码的二进制文件不会进入文本编辑器，但仍可通过下载/上传功能管理。

### 8.2 路径安全

后端通过相对路径解析和 canonical path 检查保护服务器工作区：

- 拒绝绝对路径和路径前缀。
- 拒绝 .. 穿越。
- 拒绝符号链接。
- 解析后的目标必须位于 data/servers/{id} 内。
- 拒绝覆盖、删除或改名 `server.jar`、`server.jar.part` 和 `server.jar.backup` 等根目录核心保护文件；普通文本编辑仍有约 2 MB 上限，传输接口单独受 256 MiB 上限控制。
- 新建目录和写文件都会重新检查父目录路径。

### 8.3 终端命令

POST /api/servers/{id}/command 接收命令文本：

- 服务器运行时，真实写入 Java 标准输入。
- 服务器未运行时，明确拒绝命令并返回冲突错误，避免把未执行的操作误报为成功。
- stdout、stderr、启停事件和命令输出进入同一日志广播通道。

命令接口具备较高控制权限；当前本地 API 没有额外身份认证或角色授权，必须只绑定本机可信地址。

### 8.4 日志与 WebSocket

- GET /api/servers/{id}/logs 获取历史日志。
- GET /api/servers/{id}/ws/logs 建立 WebSocket。
- WebSocket 建立后先发送最近约 200 行历史，再推送新日志。
- 前端终端页显示实时连接状态，断开时回退轮询。
- 日志会写入持久化状态，频繁输出可能造成 JSON 全量写盘压力。

---

## 第9章 资源中心与版本管理

### 9.1 资源类型

资源中心统一管理七类资源：

| 资源类型 | API kind | 典型用途 |
| --- | --- | --- |
| 核心 | core | Paper、Purpur、Fabric、Velocity 等服务端核心 |
| 插件 | plugin | 插件包、插件元数据和兼容信息 |
| 皮肤 | skin | 玩家或资源包相关素材 |
| BBModel | bbmodel | 模型资源 |
| UI 贴图 | ui_texture | UI 图片和界面素材 |
| Skill | skill | AI 能力包 |
| 插件配置 | plugin_config | 插件配置模板或生成配置 |

每类资源包含项目和版本两层：

- 项目：slug、名称、作者、摘要、描述、主页、仓库、预览图、许可证、标签、颜色和特色标记。
- 版本：版本号、渠道、Minecraft 版本、Loader、格式、Java 版本、文件名、大小、SHA-256、下载 URL/内联内容、发行说明、发布时间、状态和下载数。

### 9.2 资源库页面

镜像仓库工作区包括：

- 资源库：面向浏览和筛选。
- 版本管理：面向项目、版本、发布状态和文件维护。
- API 文档：快速开始、目录查询、版本解析、文件下载和错误处理。

页面提供核心库、插件库、皮肤库、BBModel、UI 贴图、Skill 库和插件配置切换，并支持名称/作者/摘要/标签搜索、Minecraft 版本、插件分类和发行渠道筛选。

### 9.3 项目和版本 CRUD

管理页支持：

- 新建、编辑、删除项目。
- 新建、编辑、删除版本。
- 拖拽上传资源文件。
- Skill 和插件配置的内联内容编辑。
- 发布状态 draft、published、yanked。
- 维护 Minecraft 版本、Loader、格式、Java 版本、发行说明和 SHA-256。
- 删除项目时级联删除所属版本。
- 项目 slug 变更时同步迁移版本归属。

关键校验包括：

- slug 为小写 kebab-case，最长 64 字符。
- 版本标识只允许字母、数字、点、下划线、加号和连字符。
- 下载 URL 必须为 HTTP(S)。
- published 版本要求有效发布时间、非零文件大小和 SHA-256；内联内容还要校验内容长度和摘要一致。
- 项目名称/slug 和同一项目下的版本号不能重复。
- Skill、插件配置必须指定 target_plugin。

### 9.4 版本解析和稳定下载

#### 版本解析

GET /api/v1/resolve 根据资源类型、项目 slug、Minecraft 版本和渠道选择最新兼容的已发布版本。核心和插件必须提供 Minecraft 版本；素材类资源以格式和项目版本为主。

返回项目详情、版本元数据和稳定的 download_path。

#### 稳定下载

GET /api/v1/download/{kind}/{project}/{version}：

1. 检查项目和版本是否存在且为 published。
2. 记录下载计数。
3. 内联内容直接返回。
4. 外部资源通过 HTTP 307 重定向到资源 URL 或对象文件。

该接口记录的是稳定入口被请求的次数，不等同于上游文件一定传输成功。对象文件适合由 Caddy 等静态服务提供 Range、ETag 和长期缓存。

### 9.5 插件检索优先级

GET /api/v1/plugins/search 为 AI/自动化检索接口，固定按以下顺序返回：

1. 主流插件库 mainstream。
2. 开源插件库 open_source。
3. 普通插件库 standard。
4. 付费插件库 paid。

支持关键词、Minecraft 版本、Loader 和结果数量。当前已实现目录检索和稳定下载，但尚未自动安装到服务器 plugins/，也未实现依赖、冲突、权限和完整兼容性分析。

### 9.6 独立源站管理

/resource-admin 是独立资源管理入口：

- 读取目录摘要。
- 输入并验证 Bearer 管理令牌。
- 展示上传大小限制。
- 管理项目和版本。
- 浏览器直传对象文件。
- 链接 OpenAPI JSON。

Rust 后端支持两种目录写接口凭证：浏览器管理页使用 `SCULK_CATALOG_ADMIN_USERNAME` 与 `SCULK_CATALOG_ADMIN_PASSWORD`，开服器总站自动同步继续使用 `SCULK_CATALOG_ADMIN_TOKEN`。反向代理分别以完整 Basic 凭证和 `SCULK_RESOURCE_API_TOKEN` 做第一层校验。生产环境至少必须配置其中一套 Rust 凭证。

---

## 第10章 资源同步与 MSL 核心同步

### 10.1 主站 Skill 自动同步

当总站配置独立资源中心后，资源同步 worker 可以：

1. 扫描远程主流插件目录。
2. 查找缺少专属 Skill 或插件配置的项目。
3. 从 GitHub 加载仓库元数据、目录和源文件上下文。
4. 使用配置的 AI 生成 Skill bundle 和插件配置模板。
5. 校验生成内容的结构、slug 和目标插件。
6. 通过资源中心写接口上传内联资源。
7. 保存每个任务的阶段、状态、详情、时间和错误。

管理入口：

- GET /api/resource-sync/status
- PUT /api/resource-sync/settings
- POST /api/resource-sync/scan
- POST /api/resource-sync/run-next

需要配置：

- SCULK_RESOURCE_API_BASE
- SCULK_RESOURCE_API_TOKEN
- SCULK_RESOURCE_SYNC_INTERVAL_SECONDS
- 可选 GITHUB_TOKEN

生成结果依赖外部 AI，虽然有结构校验，但仍应视为不可信输入，发布前需要人工审阅。

### 10.2 MSL 核心同步

资源中心可从 MSL V4 API 同步核心项目、构建和制品信息：

- GET /api/catalog/admin/msl-core-status
- POST /api/catalog/admin/sync-msl-cores

同步器支持目标 Minecraft 版本、周期、并发、请求间隔、429/临时错误重试和下载元数据探测。主要变量为 SCULK_MSL_CORE_SYNC_ENABLED、SCULK_MSL_API_BASE、SCULK_MSL_TARGET_VERSIONS、SCULK_MSL_SYNC_INTERVAL_SECONDS、SCULK_MSL_SYNC_CONCURRENCY、SCULK_MSL_REQUEST_INTERVAL_MS 和 SCULK_MSL_INSPECT_DOWNLOAD_METADATA。

---

## 第11章 玩家社区与运营功能

### 11.1 玩家管理

社区页面按服务器显示真实玩家快照，支持按游戏名、UUID、显示名、身份、标签或备注搜索，并可按玩家、在线状态、等级和更新时间排序。

- 玩家快照从 `world/playerdata/*.dat` 的压缩 NBT 中读取，展示等级、维度、坐标、背包、装备栏、副手和末影箱。
- 潜影盒和收纳袋可展开查看已读取的内部物品；物品名称、数量和 Lore 可在格位中查看。
- 可维护展示名称、身份、标签和管理备注，这些资料持久化在 Sculk 的管理状态中，不会直接改写在线玩家的游戏 NBT。
- 可配置最多 10 个 PlaceholderAPI 变量字段；服务器运行且检测到 PlaceholderAPI 后，可按玩家即时解析并显示变量值。

接口：

- `GET /api/servers/{server_id}/players?query=&sort=&order=`
- `GET`、`PUT /api/servers/{server_id}/players/{player_key}`
- `GET`、`PUT /api/servers/{server_id}/papi/fields`
- `GET /api/servers/{server_id}/players/{player_key}/papi`

世界玩家数据是保存快照，在线状态和背包可能存在保存延迟。若玩家仅由受管控制台识别、尚未写入世界数据，界面会明确标记快照不可用。PlaceholderAPI 的 JAR 检测不等同于插件成功启用，查询失败会返回可见状态而不会伪造变量值。

### 11.2 经济运营

页面展示：

- 玩家总资产。
- 在线玩家数量。
- 通胀指标。
- 经济调控任务入口。

当前总资产可由本地玩家状态汇总，通胀指标为原型数据，尚未接入真实交易流水、物价指数、贫富差距计算或沙盒回滚。

### 11.3 反馈与 AI 聚类

- 反馈列表包含玩家、内容、分类、情绪、状态和时间。
- POST /api/feedback/cluster 根据本地反馈计算分类和正/中/负情绪数量。
- 当前摘要为规则/原型响应，不是完整的外部 AI 聚类流水线。

### 11.4 玩法投票

- 创建投票至少需要标题和两个选项。
- 投票默认有效期约 3 天。
- POST /api/polls/{id}/vote 给指定选项增加票数。
- 投票和选项持久化到 JSON 状态。

当前没有玩家身份校验、防重复投票、投票关闭管理或 Discord/游戏内渠道同步。

---

## 第12章 Skills、MCP 与 ACP Agent

### 12.1 Skills

Skills 页面展示：

- 名称、描述、来源和版本。
- 启用/禁用状态。
- 内置能力和工作区能力。
- 主流插件专属 Skill 自动构建状态。

Skill 的启停通过 POST /api/skills/{id}/toggle 持久化。内置的 `develop-minecraft-server-plugin` Skill 位于 `backend/resources/skills/develop-minecraft-server-plugin/`，启动时会迁移到本地状态；插件开发请求命中且 Skill 启用时，后端会把 `SKILL.md` 和按需参考文档注入直连模型或 ACP Agent 的提示词。当前仍没有通用 Skill 的权限沙箱、签名校验、依赖解析和自动升级机制。

### 12.2 MCP/外部连接

集成页面展示每个连接的名称、类型、状态、端点、能力和延迟，可执行：

- 启用/禁用连接。
- 测试连接。
- 查看能力标签。
- 查看连接状态和最近延迟。

当前 Codex、Discord 和 Metrics 连接多为演示端点；test 接口主要更新本地连接状态和模拟延迟，不代表真实 MCP 握手、工具发现或远端调用已经完成。

### 12.3 ACP Agent

AI 设置可通过 ACP（Agent Client Protocol）接入外部命令：

- Codex CLI。
- Claude Code CLI。
- OpenClaw。
- Hermes。
- 自定义命令。

协议为 stdio 上的逐行 JSON-RPC 2.0，核心流程为：

1. 启动外部命令。
2. initialize 握手。
3. session/new 创建会话。
4. session/prompt 发送用户请求。
5. 读取 session/update 的 agent_message_chunk。
6. 转换为主工作台 SSE delta。
7. 结束后清理 Agent 进程。

ACP 权限行为：

- 所有审核模式都拒绝 ACP Agent 请求的权限选项。
- 文件读写等未支持的反向请求一律拒绝。
- Agent 启动或握手失败时回退到内置模型，再回退到本地规则。

配置的 Agent 命令属于本机外部进程执行边界，必须保证命令来源可信。

---

## 第13章 设置中心

设置页按以下模块组织：

### 13.1 常规与审核

- UI 语言。
- AI 默认回复语言。
- 审核模式 approval、auto、full。
- 全局安全提示。

### 13.2 外观

- 预设风格和强调色。
- 单色、渐变或图片背景。
- 2–5 个渐变颜色、渐变方案和图片遮罩透明度。
- 字体、字号和字体颜色。
- 恢复默认外观。

外观通过 applyAppearance() 即时应用，并保存到 UI 设置。

### 13.3 模型提供商

提供商 CRUD、启停、模型同步、手动添加模型、模型启停、模型删除、hi 测试和场景绑定均已提供。

### 13.4 Agent 管理

支持 Agent CRUD、类型、命令、参数、启停、握手测试和设置默认 Agent。

### 13.5 个性化

- 对话语言风格。
- 自定义风格描述。
- 额外上下文，例如服务器版本、插件和禁止重启时间。

这些设置会生成 AI 系统提示词的一部分。

### 13.6 Git

支持保存 Git 用户名、邮箱、默认分支、远程仓库地址和配置变更自动提交开关。当前没有完整的 Git 工作流执行器、提交历史面板或冲突解决流程。

### 13.7 远程连接

支持配置 SSH/SFTP 连接的主机、端口、用户名、项目根目录和启用状态，并可执行 TCP 端口连通性测试。测试不会执行 SSH/SFTP 握手，也不会真正创建或管理远程服务器；创建向导的远程位置当前禁用。

### 13.8 插件与集成

设置页还提供与 Skills & MCP 入口相近的连接和能力包管理，包括连接启停、测试、Skill 启停以及云端 Skill 链接展示。

---

## 第14章 Sculk Cloud 云服务

### 14.1 服务定位

Sculk Cloud 是可选的云端能力层，与本地服务器工作台的 JSON 状态分离：

- PostgreSQL 保存账号、团队、设备、审批、令牌、凭据和用量事实。
- Redis 保存短期会话缓存和分钟级 API 限流计数。
- 本地服务器、对话、目录等运行状态仍可以保存于 state-cloud.json。

Cloud 只有在 DATABASE_URL、REDIS_URL 和长度至少 24 个字符的 SCULK_MASTER_KEY 均可用时启用；不可用时本地 API 仍可以单独运行。

### 14.2 账号与设备

支持：

- 注册、登录、退出。
- 当前用户资料。
- 设备列表和撤销设备。
- 会话有效期配置，默认 30 天，可调 1–365 天。
- 密码使用 Argon2 哈希。
- Bearer 会话令牌使用 scs_ 前缀，数据库只保存哈希。

首次注册账号按当前实现自动成为管理员。

### 14.3 工作区同步与凭据

Cloud 工作区支持同步 UI 设置、快捷提示词和 Skill 链接，并拒绝通用同步载荷中的嵌套密钥字段。用户级上游凭据使用由 SCULK_MASTER_KEY 派生的 AES-256-GCM 密钥加密保存，响应只返回掩码和摘要边缘信息。

同步使用版本号解决并发更新，版本冲突返回 HTTP 409。前端会在 localStorage 保存 Cloud 会话和部分工作区缓存。

### 14.4 团队与审批

支持团队创建、成员查看、邀请和接受邀请。Cloud Agent 的 high/critical 任务与持久终端会在同一事务中创建唯一团队审批，并通过 `agent_task_id` 或 `terminal_session_id` 建立外键关联；审批通过后才进入可租约状态，拒绝或取消会同步关闭资源。请求人不能自批，只有团队 `owner`、`admin` 或 `approver` 可以决定。重试、恢复和回滚均创建新的资源与审批，不继承旧决定；这套机制不等同于本地 JSON `automation` 任务的通用执行器。

#### 14.4.1 主机 Agent 与远程终端

Agent 通过出站 HTTPS 心跳领取任务和终端命令：

- 任务租约带有短期 token 和过期时间；有有效审批的任务或低风险任务过期后可以回到队列，旧版本缺少可验证审批的高风险租约过期后会进入明确失败状态，不会形成不可执行的排队任务。
- `log.tail` 仅允许日志目录和崩溃报告目录，拒绝环境变量、数据库和凭据类文件名，并对常见密钥与 Bearer Token 脱敏。
- Shell 在 Windows 使用 Job Object，在 Unix 使用独立进程组；取消会等待 Agent 回报进程树终止，不能把请求发送成功误报为进程已经停止。
- 终端的 `start` 命令只有在关联审批通过、决定人仍是团队审批角色且决定人不是请求人时才可租约。
- 云部署接口仍是预留能力，不会因为 Agent 在线而自动创建或调度云资源。

### 14.5 API Token 与 OpenAI 兼容中转

支持：

- 创建、列出和撤销个人 API Token。
- 创建时仅展示一次原始 Token，服务端保存哈希。
- 按 Token 统计请求次数、延迟和 Token 用量。
- 管理员配置一个 OpenAI 兼容上游，API Key 在 PostgreSQL 中加密保存。

/api/cloud/v1/chat/completions 当前支持非流式请求，默认每个 Token 每分钟 60 次，可通过 SCULK_CLOUD_RATE_LIMIT 调整。stream=true 会返回不支持错误。

### 14.6 云部署边界

部署接口已经预留，但当前未真正创建、调度或计费云资源：

- GET /api/cloud/deployments/capability 返回 planned。
- GET /api/cloud/deployments 返回空集合。
- POST /api/cloud/deployments 返回 HTTP 501 和 deployment_planned。

---

## 第15章 数据模型、通信方式与持久化

### 15.1 本地状态模型

PersistedState 包含：

- servers：服务器项目。
- tasks：自动化、初始化和下载任务。
- configs：服务器配置快照。
- logs：服务器日志。
- mirrors：镜像配置。
- players、feedback、polls：社区数据。
- integrations、skills：集成和能力包。
- catalog：七类资源目录。
- ai：提供商、模型、情景、审核模式和 Agent。
- ui：界面、个性化、Git 和连接设置。
- resource_sync：资源同步任务。
- conversations：对话和消息。

### 15.2 状态文件

- 默认文件：backend/data/state.json。
- Cloud 并行实例可使用 SCULK_STATE_FILE=data/state-cloud.json。
- 资源中心可使用独立的 data/resource-center.json。
- 同一状态文件启动时创建 .lock 并独占锁定。
- 写入先生成临时文件并 sync_all，再替换正式文件。
- 上一版本保存为 .bak，主文件损坏时尝试恢复。
- 读取旧结构时执行字段默认值和目录迁移。

这套实现适合单机 MVP，不适合高并发写入、横向扩容、多节点共享或需要事务查询的生产场景。高频日志可能触发大量全量 JSON 写盘。

### 15.3 前端缓存

主要 localStorage 键：

| 键 | 用途 |
| --- | --- |
| sculk-cloud-session | Cloud 会话令牌 |
| sculk.resource-admin-token | 资源管理页令牌 |
| sculk-cloud-workspace-v2 | Cloud 工作区缓存 |
| sculk-cloud-ui-v2 | UI 设置缓存 |

### 15.4 通信方式

| 方式 | 用途 |
| --- | --- |
| REST/JSON | 页面数据、CRUD、任务和设置 |
| SSE | AI 流式文本、meta、错误和完成事件 |
| WebSocket | Minecraft 服务器日志历史 + 实时广播 |
| 轮询 | dashboard、下载进度和 WebSocket 断线回退 |
| stdio JSON-RPC | ACP 外部 Agent |

---

## 第16章 API 功能清单

以下为当前后端主要路由分组。具体字段约束以源码中的 Serde 结构和 GET /api/openapi.json 为准。

### 16.1 基础与服务器

~~~text
GET    /api/health
GET    /api/dashboard
GET    /api/system
POST   /api/runtime/java/install
POST   /api/chat
POST   /api/servers
POST   /api/servers/plan
DELETE /api/servers/{id}
POST   /api/servers/{id}/action
POST   /api/servers/{id}/command
GET    /api/servers/{id}/config
PUT    /api/servers/{id}/config
GET    /api/servers/{id}/logs
GET    /api/servers/{id}/ws/logs
~~~

### 16.2 对话

~~~text
GET    /api/servers/{id}/conversations
POST   /api/servers/{id}/conversations
GET    /api/conversations/{id}
PUT    /api/conversations/{id}
DELETE /api/conversations/{id}
PUT    /api/conversations/{id}/execution
POST   /api/conversations/{id}/fork
POST   /api/chat/stream
~~~

### 16.3 文件与下载

~~~text
GET    /api/servers/{id}/files
GET    /api/servers/{id}/file
PUT    /api/servers/{id}/file
POST   /api/servers/{id}/file/upload
GET    /api/servers/{id}/file/download
POST   /api/servers/{id}/directory
GET    /api/download/mirrors
POST   /api/download/preview
POST   /api/servers/{id}/download/core
GET    /api/servers/{id}/download/status
POST   /api/servers/{id}/download/cancel
~~~

### 16.4 自动化与社区

~~~text
GET    /api/automation
POST   /api/automation/tasks
POST   /api/tasks/{id}/approve
POST   /api/tasks/{id}/cancel
GET    /api/community
POST   /api/polls
POST   /api/polls/{id}/vote
POST   /api/feedback/cluster
POST   /api/players/{id}/action
~~~

### 16.5 集成与同步

~~~text
GET    /api/integrations
POST   /api/integrations/{id}/toggle
POST   /api/integrations/{id}/test
POST   /api/skills/{id}/toggle
GET    /api/resource-sync/status
PUT    /api/resource-sync/settings
POST   /api/resource-sync/scan
POST   /api/resource-sync/run-next
GET    /api/catalog/admin/msl-core-status
POST   /api/catalog/admin/sync-msl-cores
~~~

### 16.6 AI 与 Agent

~~~text
GET    /api/ai/settings
POST   /api/ai/providers
PUT    /api/ai/providers/{id}
DELETE /api/ai/providers/{id}
POST   /api/ai/providers/{id}/models/sync
POST   /api/ai/providers/{id}/models/toggle
POST   /api/ai/providers/{id}/models/add
POST   /api/ai/providers/{id}/models/remove
POST   /api/ai/test
PUT    /api/ai/scenarios
PUT    /api/ai/review-mode
POST   /api/ai/agents
PUT    /api/ai/agents/{id}
DELETE /api/ai/agents/{id}
POST   /api/ai/agents/{id}/test
PUT    /api/ai/agents/active
~~~

### 16.7 UI 与连接

~~~text
GET    /api/ui/settings
PUT    /api/ui/settings
POST   /api/ui/connections
PUT    /api/ui/connections/{id}
DELETE /api/ui/connections/{id}
POST   /api/ui/connections/{id}/test
~~~

### 16.8 资源目录

~~~text
GET|POST       /api/catalog/{resource}
GET|PUT|DELETE /api/catalog/{resource}/{slug}
GET|POST       /api/catalog/{resource}/{slug}/versions
GET|PUT|DELETE /api/catalog/{resource}/{slug}/versions/{version}
GET            /api/catalog/summary
POST           /api/catalog/admin/verify
POST           /api/catalog/admin/upload
GET            /api/v1/resolve
GET            /api/v1/plugins/search
GET            /api/v1/download/{kind}/{project}/{version}
GET            /api/openapi.json
~~~

{resource} 可取 cores、plugins、skins、bbmodels、ui-textures、skills 和 plugin-configs。列表常用查询参数为 search、minecraft、channel、plugin_category 和 target_plugin。

### 16.9 Cloud

~~~text
GET    /api/cloud/status
POST   /api/cloud/auth/register
POST   /api/cloud/auth/login
POST   /api/cloud/auth/logout
GET    /api/cloud/me
PATCH  /api/cloud/me
GET    /api/cloud/devices
DELETE /api/cloud/devices/{id}
GET|PUT /api/cloud/sync/settings
GET|POST /api/cloud/credentials
DELETE /api/cloud/credentials/{id}
GET|POST /api/cloud/teams
GET    /api/cloud/teams/{id}/members
POST   /api/cloud/teams/{id}/invitations
POST   /api/cloud/invitations/accept
GET|POST /api/cloud/approvals
POST   /api/cloud/approvals/{id}/decision
GET|POST /api/cloud/agent-tasks
GET    /api/cloud/agent-tasks/{id}
POST   /api/cloud/agent-tasks/{id}/cancel|retry|rollback
GET|POST /api/cloud/terminal-sessions
POST   /api/cloud/terminal-sessions/{id}/input|resize|terminate
GET    /api/cloud/terminal-sessions/{id}/events
GET|POST /api/cloud/conversations
GET    /api/cloud/conversations/{id}
POST   /api/cloud/conversations/{id}/plans
GET|POST /api/cloud/tokens
DELETE /api/cloud/tokens/{id}
GET    /api/cloud/usage
GET|PUT /api/cloud/admin/relay-provider
POST   /api/cloud/v1/chat/completions
GET    /api/cloud/deployments/capability
GET|POST /api/cloud/deployments
~~~

---

## 第17章 运行、部署与环境配置

### 17.1 本地开发

后端默认监听 127.0.0.1:8787，前端 Vite 默认访问 http://127.0.0.1:5173：

~~~powershell
cd backend
cargo run

cd ..\frontend
npm run dev
~~~

前端生产构建：

~~~powershell
cd frontend
npm run build
~~~

### 17.2 本地生产构建

scripts/start-local.ps1 使用 backend/target-local/release/backend.exe 和 frontend/dist，默认端口 8787，并设置：

~~~text
SCULK_BIND_ADDRESS=127.0.0.1:8787
SCULK_STATIC_DIR=frontend/dist
SCULK_DATA_DIR=backend/data
SCULK_STATE_FILE=backend/data/state.json
~~~

脚本会等待 /api/dashboard 就绪，并以隐藏窗口运行后端。

默认启动会清除子进程继承的 `SCULK_ALLOW_CODEX_FULL` 和 `SCULK_CODEX_TRUSTED_COMMAND`，保持 Codex 只读沙盒。需要将本地回环服务作为 Codex WebUI 并明确授予完整权限时，先停止正在运行的后端，然后指定与原生 Codex CLI Agent 命令相同的绝对路径：

~~~powershell
.\scripts\stop-local.ps1
$codexCommand = (Get-Command codex.cmd -CommandType Application).Path
.\scripts\start-local.ps1 -EnableCodexFullAccess -CodexCommand $codexCommand
~~~

`-CodexCommand` 必须是存在的 `.exe`、`.cmd` 或 `.bat` 文件；npm 安装的 Windows Codex 应使用 `codex.cmd` 而非 PowerShell shim `codex.ps1`。该参数只配置新启动的后端，不会改变已经运行的进程。即使脚本已启用，后端仍要求回环监听、当前 Agent 命令与该路径 canonical 后完全一致，并且对话选择 `full` 审核模式；完整权限以运行后端的宿主机账户执行，项目/服务器目录仅作为初始工作目录。

Linux 使用 `scripts/start-local.sh` 和 `scripts/stop-local.sh`。停止脚本向后端发送 SIGTERM，后端停止接受新启动任务并并行安全停服；超时才强制退出。生产部署模板 `deploy/sculk-catalyst.service` 使用 `KillMode=mixed`，先让主进程处理安全停服，再由 systemd/cgroup 清理超时后的剩余进程。

### 17.3 Cloud 本地模式

Docker Compose 模式启动 PostgreSQL 17 和 Redis 7：

~~~powershell
docker compose -f docker-compose.cloud.yml up -d
Copy-Item .env.cloud.example .env
cd backend
cargo run
~~~

关键配置：

~~~text
DATABASE_URL=postgres://...
REDIS_URL=redis://...
SCULK_MASTER_KEY=至少 24 个字符的随机密钥
SCULK_CLOUD_SESSION_DAYS=30
SCULK_CLOUD_RATE_LIMIT=60
SCULK_STATE_FILE=data/state-cloud.json
~~~

Windows 原生 scripts/start-cloud.ps1 是另一条运行链路：PostgreSQL 使用 127.0.0.1:55432，Redis 使用 127.0.0.1:56379，后端使用 127.0.0.1:8788 和 target-cloud/debug/backend.exe。不要混用 Docker 配置中的端口与原生脚本配置。它也支持 `-EnableCodexFullAccess -CodexCommand $codexCommand`，具有与本地生产脚本相同的显式授权、路径校验和重启要求。

### 17.4 独立资源中心

资源中心可以单独部署 Rust API、静态对象和 Caddy：

- Rust API 默认只监听 127.0.0.1:8789。
- 目录元数据使用独立状态文件。
- 对象文件使用 SCULK_RESOURCE_OBJECT_DIR。
- Caddy 负责 HTTPS、对象静态分发、缓存和写操作第一层令牌校验。
- Linux systemd 和 Docker host-network 两种部署模板均已提供。
- 资源中心备份脚本按日打包元数据和对象，默认保留 14 天。

主要变量：

~~~text
SCULK_BIND_ADDRESS=127.0.0.1:8789
SCULK_STATE_FILE=data/resource-center.json
SCULK_RESOURCE_OBJECT_DIR=/opt/sculk-resource/objects
SCULK_RESOURCE_PUBLIC_BASE=https://resources.example.com
SCULK_RESOURCE_UPLOAD_MAX_BYTES=268435456
SCULK_CATALOG_ADMIN_TOKEN=高熵随机令牌
SCULK_RESOURCE_API_TOKEN=与反向代理一致的高熵令牌
SCULK_CATALOG_ADMIN_USERNAME=浏览器管理账号
SCULK_CATALOG_ADMIN_PASSWORD=浏览器管理密码
SCULK_CATALOG_ADMIN_BASIC_AUTH=账号:密码的 Base64 编码
~~~

前端构建时可用 VITE_RESOURCE_API_BASE 指向独立资源域名；主站后端同步使用 SCULK_RESOURCE_API_BASE 和 SCULK_RESOURCE_API_TOKEN。

### 17.5 重要运行变量

| 变量 | 作用 |
| --- | --- |
| SCULK_BIND_ADDRESS | 后端监听地址和端口 |
| SCULK_STATIC_DIR | 前端静态文件目录 |
| SCULK_DATA_DIR | 状态、服务器工作区和托管 Java 的统一数据根目录 |
| SCULK_STATE_FILE | 本地状态文件路径 |
| SCULK_ALLOWED_ORIGINS | 允许的额外 CORS 来源 |
| SCULK_ALLOW_CODEX_FULL | 设为 true 时允许回环监听的后端启用 Codex 完整权限 |
| SCULK_CODEX_TRUSTED_COMMAND | 启用 Codex 完整权限时必须指定的绝对 Codex 可执行文件路径 |
| SCULK_JAVA_BIN | 指定 Java 可执行文件 |
| SCULK_CATALOG_ADMIN_TOKEN | 资源目录写操作令牌 |
| SCULK_CATALOG_ADMIN_USERNAME | 浏览器管理账号 |
| SCULK_CATALOG_ADMIN_PASSWORD | 浏览器管理密码 |
| SCULK_CATALOG_ADMIN_BASIC_AUTH | Caddy 精确匹配管理 Basic 请求头使用的 Base64 凭证 |
| SCULK_RESOURCE_OBJECT_DIR | 资源对象文件目录 |
| SCULK_RESOURCE_PUBLIC_BASE | 上传后生成的公开对象基地址 |
| SCULK_RESOURCE_API_BASE | 主站连接独立资源中心的地址 |
| SCULK_RESOURCE_API_TOKEN | 主站同步资源中心的 Bearer 令牌 |
| DATABASE_URL | Cloud PostgreSQL 地址 |
| REDIS_URL | Cloud Redis 地址 |
| SCULK_MASTER_KEY | Cloud 凭据加密主密钥 |
| SCULK_CLOUD_SESSION_DAYS | Cloud 会话有效期 |
| SCULK_CLOUD_RATE_LIMIT | Cloud Token 每分钟请求上限 |
| GITHUB_TOKEN | 可选的 GitHub API 访问令牌 |

---

## 第18章 安全边界、限制与当前缺口

### 18.1 必须明确的安全边界

1. 本地多数 API 没有登录、RBAC 或细粒度授权；只要能访问监听地址，就可能调用文件写入、命令发送和服务器控制接口。
2. 默认绑定 127.0.0.1 是重要的安全假设，公开部署必须使用精确 CORS、HTTPS 反向代理、登录和网络访问控制。
3. 本地 AI provider 的 API Key 在 state.json 和 .bak 中明文保存，接口响应虽然脱敏，但不等于磁盘加密。
4. AI provider 的 URL 目前只校验 HTTP(S)，未完成 SSRF、回环地址和私网地址限制。
5. ACP Agent 的 command 是任意本机命令配置，必须限制配置来源和文件权限。
6. 同时未配置管理账号密码和 `SCULK_CATALOG_ADMIN_TOKEN` 时，目录写操作会放行；资源站生产部署必须至少启用一种服务端凭证。
7. 本地 JSON 自动化仍不是通用工具执行器；Cloud Agent 的 high/critical 任务和持久终端已经有服务端强制审批门，校验团队、审批关联、独立决定人和当前角色。
8. 审计日志当前可写入本地 JSON，尚不是不可篡改审计。

### 18.2 当前功能缺口

#### 服务器与 Minecraft 集成

- 后端重启后无法重新关联已运行的 Java 进程。
- 尚无完整崩溃识别、自动重启、退避和进程恢复。
- 未接入 RCON、Query、管理插件或代理 API。
- CPU、TPS、玩家和部分经济数据不是真实线上采集。
- 未实现世界备份、恢复、克隆和镜像服差异同步。

#### 插件与资源

- 插件只支持目录检索、版本筛选和稳定下载，未自动安装到 plugins/。
- 尚无插件依赖、冲突、权限风险和兼容矩阵验证。
- 资源目录仍使用单一 JSON，没有分页、事务、历史版本审计和对象回收；浏览器对象上传与静态 Range/ETag 已提供，但 Rust 主 API 不代理大文件。
- 稳定下载通过 307 跳转，不由 Rust API 代理大文件。

#### AI 与自动化

- 本地 AI 对话尚未直接调用文件、终端、服务器启停或插件安装工具；Cloud Agent 的远程任务和持久终端属于独立已实现链路。
- 意图分类和任务创建仍采用关键词规则，模型回复不改变任务判定。
- 尚无跨对话知识库、摘要记忆、Token 统计、计费和本地限流。
- 自动修复、配置生成、经济修改和正式部署尚无完整回滚链。

#### Cloud 与外部系统

- 云部署接口尚未创建实际资源。
- 外部 MCP、Discord 和监控连接主要是演示端点。
- Cloud 远程审批已经自动绑定 Cloud Agent 任务和持久终端；它仍不会自动接管本地 JSON `automation` 任务。
- 多用户隔离、完整 RBAC、多租户和密钥轮换尚未完成。

### 18.3 许可提醒

项目主体和 Cloud 部分使用分区许可：

- 未特别声明的主体文件适用 Apache License 2.0。
- Sculk Cloud 相关文件适用 PolyForm Noncommercial License 1.0.0。
- 完整边界和署名要求以 NOTICE、LICENSE 和 LICENSES/PolyForm-Noncommercial-1.0.0.md 为准。

---

## 第19章 典型用户流程

### 19.1 首次创建并运行本地服务器

1. 启动 Rust 后端和 Vue 前端。
2. 在左侧点击“创建服务器”。
3. 填写名称并选择本地位置。
4. 选择核心和兼容 Minecraft 版本。
5. 设置端口和最大内存。
6. 通过环境检查，必要时安装 Java 21。
7. 阅读并接受 EULA。
8. 创建项目；总览自动显示首次初始化任务。
9. 等待核心下载、大小与 SHA-256 校验以及 Java 检查完成；失败时在原卡片重试。
10. 初始化完成后在总览点击启动。
11. 在终端查看真实日志并发送命令。

### 19.2 智能规划开服

1. 在创建向导中选择智能规划。
2. 输入名称后创建规划项目。
3. 系统只建立 planning 项目和“开服规划”对话，不创建文件。
4. 在对话中说明玩法、人数、版本和插件偏好。
5. 由 AI 或本地规则生成核心和部署建议。
6. 方案确认后再进入实际项目创建和核心下载流程。

### 19.3 修改配置并验证

1. 选择服务器并进入“文件”。
2. 打开 server.properties 或 YAML/JSON 配置。
3. 修改并保存，后端同步保存配置快照。
4. 进入“终端”查看日志。
5. 需要执行命令时，服务器运行中写入真实 Java stdin；未运行时会明确提示先启动服务器。
6. 涉及停服、覆盖或高风险变更时，通过自动化任务和审批模式控制。

### 19.4 配置 AI 对话

1. 进入设置 → 模型。
2. 新增 OpenAI 格式 provider，填写地址和 API Key。
3. 同步 /v1/models 并启用需要的模型。
4. 发送 hi 测试连通性。
5. 为对话、自动化、开服向导、配置和社区场景绑定模型。
6. 在对话输入区按需覆盖模型或 Agent。
7. 选择审核模式，确认高风险操作策略。
8. 在工作区中通过 SSE 查看流式回复和关联任务。

### 19.5 发布资源并供核心下载

1. 进入镜像仓库或 /resource-admin。
2. 创建核心项目和版本。
3. 填写 Minecraft 版本、Loader、文件名、大小、SHA-256、下载 URL 和发行说明。
4. 将版本发布为 published。
5. 使用 /api/v1/resolve 验证兼容版本能被解析。
6. 使用稳定下载 URL 验证 307 重定向或内联内容。
7. 开服器下载时会优先消费资源目录中的可信核心制品。

### 19.6 配置 Cloud 与团队审批

1. 启动 PostgreSQL 和 Redis。
2. 设置 DATABASE_URL、REDIS_URL 和 SCULK_MASTER_KEY。
3. 启动后端并等待 /api/cloud/status 显示可用。
4. 注册首个管理员账号。
5. 创建团队并邀请成员。
6. 创建 API Token 或配置上游中转。
7. 通过 Cloud 审批接口处理远程批准/拒绝。
8. 通过用量接口查看请求和 Token 统计。

---

## 第20章 验证基线与后续路线

### 20.1 当前验证基线

在当前工作区执行的基础验证结果：

- 后端：`cargo fmt -- --check`、`cargo test --all-targets --locked`、`cargo clippy --all-targets --locked -- -D warnings -A clippy::too_many_arguments` 通过；Windows MSVC target 的 `cargo check --all-targets --locked` 通过。
- Agent：`cargo fmt -- --check`、`cargo test --all-targets --locked` 通过。
- 前端：工作区 Node.js 执行 `vue-tsc -b` 和 Vite production build 通过，并生成主工作台与资源管理入口。
- 脚本与迁移：PowerShell E2E 脚本通过语法解析；Cloud 迁移为顺序执行的 PostgreSQL migration，并对新审批约束执行 `VALIDATE CONSTRAINT`。

### 20.2 建议优先级

#### P0：真实单机开服闭环

1. SQLite、事务、迁移、备份和恢复。
2. Java 版本绑定、可靠安装和运行时切换。
3. 进程恢复、崩溃识别、自动重启和强制终止策略。
4. 下载任务持久化、暂停/恢复、跨重启接续、限速和镜像健康检查。
5. 本地管理员登录、密钥加密、RBAC 和不可绕过的审批门。

#### P1：插件与测试服闭环

1. 插件自动安装到服务器工作区。
2. 依赖、冲突、权限和 Minecraft 兼容检查。
3. 配置 Schema、Diff、备份和回滚。
4. 镜像服自动启动、测试脚本和结果报告。
5. Codex/MCP 真实构建、测试、产物回传和灰度部署。

#### P2：社区运营与产品化

1. 接入 Discord、QQ、网页和游戏内反馈渠道。
2. 投票身份校验、防重复投票和多渠道同步。
3. 真实经济流水、物价指数和调控沙盒。
4. 多用户、RBAC、多租户和远程管理。
5. Tauri 桌面端、自动升级、系统托盘和可观测性。

### 20.3 公开测试验收条件

进入公开测试前，至少应满足：

- 全新 Windows x64、Linux x64 或 Linux ARM64 环境能够准备兼容 Java。
- Paper/Purpur 核心可通过配置镜像下载并完成可信校验。
- 服务器可创建、启动、停止、重启并恢复状态。
- 终端命令和日志均为真实数据。
- 配置变更具备 Diff、审批、备份和回滚。
- AI 无法绕过高风险审批。
- 密钥不会明文出现在日志、状态文件和前端响应中。
- 后端异常退出后不会遗留失控 Java 进程。
- 核心链路具备自动化测试和可复现错误报告。

---

## 相关文档

- [项目功能现状与开发路线](PROJECT_STATUS.md)
- [Sculk Cloud 说明](SCULK_CLOUD.md)
- [资源中心部署说明](RESOURCE_CENTER.md)
- [项目 README](../README.md)
- [Apache License 2.0](../LICENSE)
- [NOTICE](../NOTICE)
