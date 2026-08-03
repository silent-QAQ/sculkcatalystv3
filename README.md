# Sculk Catalyst V3

[![CI](https://github.com/silent-QAQ/sculkcatalystv3/actions/workflows/ci.yml/badge.svg)](https://github.com/silent-QAQ/sculkcatalystv3/actions/workflows/ci.yml)
广告：此参考代码版本会落后最新版本3个版本，添加作者qq群点击链接加入群聊【我的世界younimc综合网站】：https://qm.qq.com/q/isbVXXHnkQ 获取最新构建

广告2：赔钱云 全网最低价！（本项目使用赔钱云机器开发，120元8h40g/年 云服务器，正好安装codex使用）
https://www.peiqianyun.com/aff/TJOEKQKN

广告3：广告招租


无需任何基础，java不会装都能帮您全自动开服加管理的新时代aimc开服器。

从服务器创建，java安装，到插件选择和配置编写，小白也有开服梦。

从玩家意向收集到编写多个计划邀请用户投票， 创意不应该只来源于服主。

从最受欢迎玩法到落地为完善规范化插件，创意不应该受限于编程能力。

从24小时机器测试到创建镜像邀请玩家实测，插件应该经过严谨的测试。

从服务器经济调控到玩家画像与管理，辅助你完成服务器的管理。

从视频宣传文案编写到视频评论自动评论，bili机器人助力萌新服主抓住更多可能流失的流量。

从主动与用户私聊到协助游戏安装设置与游戏玩法学习，qq机器人手把手教会新人玩家感受mc魅力。

从入驻游戏生物ai对话到亲密度养成到引擎动作执行，mc也是旮旯game。

是的，你没看错，这就是sculkV3，服主和玩家的mcai助手，更多功能还在开发中，star me，求求了，qaq claude与codex的战略合作伙伴，中转站的亲爹，token的终结者来了！！！！

Sculk Catalyst V3 是一个 AI 驱动的 Minecraft 服务器工作台。它把服务器创建、Java 环境、核心下载、进程控制、日志与终端、文件管理、资源目录、自动化任务和可选的 Sculk Cloud 主机代理整合到一个 Web 界面中。

项目当前处于“可交互 MVP / 技术验证原型”阶段：本地服务器管理主链路已经可以运行，部分 Cloud、社区运营、MCP 和 AI 自动化能力仍属于局部实现或演示能力。不要把当前版本当作已经完成鉴权、隔离和生产运维加固的公共开服平台。

## 目录

- [项目状态](#项目状态)
- [功能概览](#功能概览)
- [运行架构](#运行架构)
- [运行模式](#运行模式)
- [API 快速参考](#api-快速参考)
- [环境要求](#环境要求)
- [本地开发](#本地开发)
- [本地生产构建](#本地生产构建)
- [Sculk Cloud 与主机 Agent](#sculk-cloud-与主机-agent)
- [资源中心](#资源中心)
- [配置项](#配置项)
- [测试与 CI](#测试与-ci)
- [常见问题与排错](#常见问题与排错)
- [目录结构](#目录结构)
- [已知限制与安全边界](#已知限制与安全边界)
- [文档索引](#文档索引)
- [许可证](#许可证)

## 项目状态

最后审计：2026-08-01。

本轮项目审计已关闭以下问题：资源搜索输入可能带入本地路径、Token 或密码等敏感词；Rust 1.88 下后端 Clippy 与跨平台 CI 不一致；Cloud、Agent、资源中心的启动入口和配置说明不完整；服务器文件传输、Windows 进程树清理、真实 CPU/RSS 指标、Agent 日志脱敏以及高风险任务/终端审批链路缺少闭环。对应代码和文档已在本地完成验证，本次推送后的 GitHub Actions run `30685344322` 也已通过。这不代表项目已经具备公网生产环境所需的登录、RBAC、沙箱和高可用能力。

| 审计项 | 处理结果 | 验证依据 |
| --- | --- | --- |
| 资源搜索敏感输入 | 已修复：过滤 Windows/Unix 路径、`token`、`password` 等词后再进入搜索上下文 | [`backend/src/server_intelligence.rs`](backend/src/server_intelligence.rs)，提交 `8db1fa3` |
| Rust 1.88 Clippy | 已修复：后端 Clippy 规则与 CI 命令已对齐 | [`.github/workflows/ci.yml`](.github/workflows/ci.yml)，提交 `1da86f2` |
| 跨平台 CI | 已验证：Ubuntu 后端、Windows 后端、前端构建均通过 | [Actions run 30685344322](https://github.com/silent-QAQ/sculkcatalystv3/actions/runs/30685344322) |
| README 操作闭环 | 已补齐：运行模式、Cloud/Agent、资源中心、配置、API、排错和备份说明 | [README 当前版本](https://github.com/silent-QAQ/sculkcatalystv3/blob/main/README.md) |
| 服务器文件传输 | 已修复：上传/下载限制在安全工作区，单文件上限 256 MiB，禁止覆盖既有文件和 `server.jar*` 保护文件 | [`backend/src/main.rs`](backend/src/main.rs)，后端文件传输测试 |
| 进程树与运行指标 | 已修复：Windows Job Object、Unix 进程组、真实 CPU 与 RSS MiB 采样，并按进程代际写回 | [`backend/src/process_platform.rs`](backend/src/process_platform.rs)、[`backend/src/runtime.rs`](backend/src/runtime.rs) |
| Cloud/Agent 审批闭环 | 已修复：任务/终端与团队审批建立外键关联，禁止请求人自批，租约发放前复核当前审批人角色，旧无关联高风险任务和终端启动 fail-closed | [`backend/src/cloud.rs`](backend/src/cloud.rs)、`backend/migrations/20260801000*.sql` |

仍属于产品功能缺口的内容，以本节下方“主要缺口”和“已知限制与安全边界”为准，不会因为审计项关闭而被标记为已实现。

| 模块 | 当前状态 | 已具备能力 | 主要缺口 |
| --- | --- | --- | --- |
| 本地工作台 | 已实现 MVP | 多服务器导航、项目模式、对话树、可调主栏分隔、服务器控制、任务和设置中心 | 以 Web 工作台交付；桌面应用打包、系统托盘和自动更新不在产品范围内 |
| 开服向导 | 已实现 MVP | 普通创建、智能规划、Java/端口/磁盘检查、核心选择和工作区生成 | 远程路径创建、完整核心兼容矩阵 |
| 首次初始化 | 已实现 MVP | 持久化 `server_provision` 任务、核心下载与校验、Java 检查、取消、重试和后端重启后重新入队 | 字节级跨重启续传、限速和模板版本管理 |
| 服务器进程 | 部分实现 | 真实 Java 子进程、就绪检测、优雅停止、超时强杀、原子重启、Windows Job Object、Unix 进程组、真实 CPU/RSS 指标 | 后端重启后不会重新接管已有 Java；崩溃自动恢复和历史进程重连 |
| 实时终端 | 已实现 | WebSocket 日志、运行中的 stdin 转发、未运行时明确拒绝、断线回退轮询 | 命令历史、补全、多会话终端 |
| AI 对话 | 部分实现 | OpenAI 格式提供商、模型同步、SSE 流式回复、情景模型、ACP Agent、审核模式 | 模型尚未直接调用文件/终端/服务器工具，暂无完整上下文压缩和用量计费 |
| 文件管理 | 已实现 MVP | 安全目录浏览、文本编辑、重命名、上传/下载、删除、跨目录移动和单文件复制；256 MiB 单文件上限、原子写入和受保护核心文件 | 差异对比、目录复制和批量传输 |
| 自动化任务 | 部分实现 | 风险等级、团队审批、取消、进度、审计状态、检查点、重试和结构化回滚 | 依赖图、更多工具权限和跨节点恢复 |
| 资源中心 | 已实现，可独立部署 | 七类资源目录、版本管理、上传、大小与 SHA-256、稳定下载、Range/ETag 静态对象、OpenAPI | 多管理员 RBAC、分页、对象回收和更完整的限流审计 |
| Sculk Cloud | 部分实现 | 账号、团队、设备、设置同步、审批、Token、用量、数据库迁移 | 云资源创建/调度接口仍返回 `501 deployment_planned` |
| 主机 Agent | 部分实现 | 出站配对、指纹确认、心跳、任务租约、团队审批、Shell、持久终端、checkpoint、取消、重试、回滚、日志路径限制与脱敏 | 更细粒度文件/日志/进程权限、租约恢复和断线审计 |
| 社区与运营 | 部分实现 | 玩家/反馈/投票/经济模块入口和本地持久化 | RCON/Query/管理插件接入、真实玩家和 TPS 数据 |
| Skills / MCP / 机器人 | 部分实现 | Minecraft 插件 Skill 编译期加载、参考注入、迁移；QQ/NapCat 与评论 webhook 适配器 | 通用 Skill 沙箱、签名、依赖升级、真实 MCP 客户端和更多平台适配 |

能直接验证当前实现细节和路线的状态文档见 [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md)。

## 功能概览

### 本地服务器工作台

- 多服务器、项目和对话树导航，支持新建、重命名、归档、分叉、删除、搜索和未读状态。
- 桌面宽度下可拖动对话区与工作区之间的分隔线调整比例，双击可恢复默认；窗口、侧栏与界面缩放变化时会保持可用边界。
- 四步开服向导：名称与位置、服务器参数、环境检查、确认创建。
- 普通创建会生成独立服务器目录、`server.properties`、`eula.txt`、启动脚本、`plugins` 和 `logs`。
- 智能创建只建立规划项目与“开服规划”对话，不会假装已经下载核心或生成服务器文件；在规划对话中回复“继续”“开始创建”等确认语，或点击“按当前方案创建”，会创建受审计的 `server_bootstrap` 任务。
- 在“请求批准”或“替我审核”模式下，`server_bootstrap` 任务需要先在任务执行器中批准；批准前不会下载核心、写入服务器文件或启动 Java 进程。“完全访问权限”会让任务直接入队执行，只应在可信的本机环境中使用。任务依次解析资源、校验核心、检查 Java、准备工作区与 EULA，并在启动后等待真实就绪标记。
- 支持启动、停止、重启、状态查询、实时日志、终端命令和文本配置编辑。
- 文件管理器限制在服务器工作区内，拒绝绝对路径、路径穿越和符号链接；文本编辑上限约 2 MB。
- 文件和目录可在工作区内重命名；可直接变更 `.md`、`.txt`、`.yml`/`.yaml` 等后缀。扩展名不决定可编辑性，读取仍要求内容是 UTF-8 文本且不超过约 2 MB。
- 文件上传/下载单文件上限 256 MiB，上传采用临时文件 + 原子重命名，默认不覆盖既有文件，并保护根目录 `server.jar`、`server.jar.part` 和 `server.jar.backup`。

### 初始化、下载和 Java

- `server_provision` 是持久化任务：后端重启后会把未完成的初始化任务重新放回队列。
- 核心下载支持资源目录优先、多源回退、流式写入 `.part`、文件大小与 SHA-256 校验、取消、失败清理和原子替换。
- 同一服务器的下载与启动互斥，重复初始化请求可以复用已有任务或已安装的非空 `server.jar`。
- 托管 Java 安装使用 Eclipse Adoptium，当前托管安装目标为 Windows x64、Linux x64 和 Linux ARM64，默认版本为 Java 21。
- 已有 Java 的检测顺序为 `SCULK_JAVA_BIN`、项目托管 Java、`JAVA_HOME`、系统 PATH；外部 Java 仍需通过版本和兼容性检查。
- `SCULK_DATA_DIR` 统一控制状态文件、服务器工作区和托管 Java 目录。

初始化任务可以跨后端重启恢复，但这不等于 Minecraft Java 进程恢复：后端重启后不会自动重新接管一个已经在运行的旧 Java 进程。核心下载也不提供跨重启的字节级续传。

### AI、ACP 和自动化

- 支持多个 OpenAI 兼容提供商、模型列表同步、连通性测试和五类情景模型绑定。
- `POST /api/chat/stream` 提供 SSE 流式对话；对话、模型、Agent 和审核模式会持久化。
- 支持内置模型 Agent 与 ACP stdio JSON-RPC Agent；ACP 连接失败时回退到内置模型，再回退到本地规则回复。
- 审核模式包括请求批准、替我审核和完全访问权限；自动化任务提供风险等级、审批、取消、进度和审计状态。
- 内置模型与 ACP Agent 不具备任意文件、终端、停服或经济操作权限，不能把模型回复当成已经完成的服务器变更。
- 仅服务器规划确认会被本地规则识别并派发到受审计任务执行器；它不会根据模糊的“RPG”需求猜测或自动安装插件，插件清单需要明确确认后再处理。

### 资源中心

资源中心统一管理以下资源类型：

- 服务端核心 `core`
- 插件 `plugin`
- 玩家皮肤 `skin`
- Blockbench 模型 `bbmodel`
- UI 贴图 `ui_texture`
- Agent Skill `skill`
- 插件配置 `plugin_config`

它提供项目与版本 CRUD、兼容版本筛选、四级插件优先检索、稳定解析与下载接口、浏览器文件上传、自动生成文件大小和 SHA-256、OpenAPI 3.1 描述，以及可选的独立高带宽对象服务器。插件资源可以检索、管理和下载，但尚未自动安装到服务器 `plugins/`，也没有依赖和冲突解析。

### Sculk Cloud 与主机 Agent

- Cloud 支持账号、团队、邀请、审批、个人 Token、API 凭据、用量、设备和设置同步。
- 主机 Agent 采用出站连接，不要求在开发机或 Minecraft 主机开放入站端口。
- Agent 支持短码配对、指纹确认、心跳、任务租约、结构化工作区操作、Shell、持久终端、检查点、取消、停止、恢复、重试和回滚任务。
- Windows Shell 使用 Job Object，Unix Shell 使用独立进程组清理进程树。
- Cloud Agent 的 high/critical 任务和持久终端必须绑定团队审批；请求人不能自批，重试/回滚会生成新的审批；`log.tail` 只读日志目录并对常见密钥和 Bearer Token 脱敏。
- Agent 的 `full` Shell 权限不是沙箱；命令最终受运行 Agent 的操作系统账号权限约束，Cloud 审批也不等价于文件系统隔离。

### 机器人与外部集成

- QQ/NapCat、Bilibili、抖音和通用 webhook 适配器的配置集中在 `.env.bot.example` 与 [`docs/BOT_INTEGRATIONS.md`](docs/BOT_INTEGRATIONS.md)。
- QQ 群消息需要显式 @ 机器人后才触发回复；私聊按适配器规则处理。
- 入站 webhook 使用 `SCULK_BOT_WEBHOOK_TOKEN` 校验，出站回复交给 NapCat 或平台桥接服务；这些令牌不应写进前端或提交到仓库。

## 运行架构

```mermaid
flowchart LR
    UI[Vue 3 工作台] --> API[Rust Axum API]
    API --> STATE[state.json 与备份]
    API --> WORKSPACE[服务器工作区]
    API --> JAVA[Java 子进程]
    API --> CATALOG[资源目录与对象存储]
    API --> AI[OpenAI 兼容 API 或 ACP Agent]
    API --> CLOUD[(PostgreSQL / Redis，可选)]
    CLOUD --> AGENT[主机 Agent，可选]
    AGENT --> HOST[远程工作区、Shell 与终端]
```

本地模式可以只运行 Rust 后端和前端开发服务器；Cloud 模式额外需要 PostgreSQL 与 Redis；独立资源中心可以把目录 API、Caddy 静态对象和主站前端分开部署。

## 运行模式

| 模式 | 后端/入口 | 依赖 | 适用场景 |
| --- | --- | --- | --- |
| 本地工作台 | Rust API `127.0.0.1:8787`，前端 `127.0.0.1:5173` | JSON 状态文件；运行 Minecraft 时需要 Java | 单机管理服务器、开发和功能验证 |
| Cloud Web | Cloud API：Docker 本地通常为 `127.0.0.1:8787`，Windows 原生脚本为 `127.0.0.1:8788` | PostgreSQL、Redis、`SCULK_MASTER_KEY` | 账号、团队、设备和 Agent 协作 |
| 独立资源管理 | 前端 `/resource-admin`，资源 API 可为官方源站或自部署域名 | 资源中心 Rust API；可选 Caddy 和对象目录 | 管理核心、插件、皮肤、Skill 等资源 |
| 主机 Agent | 主机侧 `sculk-agent` 主动连接 Cloud | Rust 编译产物、可访问 Cloud 的 HTTPS 地址 | 管理与 Cloud 不在同一台机器的服务器 |

本地工作台和 Cloud 可以并行运行，但必须使用不同的监听端口和状态文件；推荐 Cloud 使用 `SCULK_STATE_FILE=data/state-cloud.json`，避免两个后端同时写入同一个 `state.json`。

## API 快速参考

以下是最常用的入口，完整路由和请求结构以代码及专项文档为准：

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| `GET` | `/api/health` | 后端健康检查，返回 `ok` |
| `GET` | `/api/dashboard` | 读取工作台总览、服务器和任务状态 |
| `POST` | `/api/servers` | 创建服务器并生成首次初始化任务 |
| `POST` | `/api/servers/{id}/provision` | 启动或重试核心初始化 |
| `POST` | `/api/servers/{id}/action` | 启动、停止或重启服务器进程 |
| `POST` | `/api/servers/{id}/command` | 向运行中的服务器 stdin 发送命令 |
| `GET` | `/api/servers/{id}/ws/logs` | 订阅实时日志 WebSocket |
| `POST` | `/api/chat/stream` | 获取 SSE 流式 AI 对话回复 |
| `GET` | `/api/resource-catalog/...` | 主控制台读取远程资源目录的同源只读代理 |
| `GET` | `/api/openapi.json` | 独立资源中心的 OpenAPI 描述 |

Cloud 路由统一位于 `/api/cloud/...`，包括账号、团队、Agent、任务、终端、Token 和用量；资源中心自身的目录接口位于 `/api/catalog/...`，不要将这两套路由与主控制台的 `/api/resource-catalog/...` 代理混用。

## 环境要求

基础开发环境：

- Rust 1.88 或兼容的稳定版工具链。
- Node.js 与 npm；CI 使用 Node 24。
- Java 不是启动后端的必需依赖，但创建和运行 Minecraft 服务器时需要 Java 21，或让项目自动准备托管 Java。
- Git。

按场景增加：

- Sculk Cloud：Docker、PostgreSQL、Redis。
- Linux 发布：bash、curl；CI 还会运行 ShellCheck。
- Agent：对应平台的 Rust 编译环境。
- 独立资源中心：Linux 服务环境、HTTPS 反向代理和用于存放对象的磁盘目录。

## 本地开发

### Windows PowerShell

在第一个终端启动本地后端：

```powershell
Set-Location backend
cargo run
```

在第二个终端启动前端：

```powershell
Set-Location frontend
npm ci
npm run dev
```

打开 <http://127.0.0.1:5173>。Vite 开发服务器默认把 `/api` 请求代理到 `http://127.0.0.1:8787`；如果后端端口不同，在启动前端的终端设置 `VITE_API_PROXY`。

### Linux / macOS shell

终端 1：

```bash
cd backend
cargo run
```

终端 2：

```bash
cd frontend
npm ci
npm run dev
```

前端入口由路径和构建模式决定：默认路径加载本地工作台，`/resource-admin` 加载资源管理页，`VITE_APP_MODE=cloud` 加载 Cloud 入口。Cloud 前端也可以使用：

```bash
cd frontend
VITE_API_PROXY=http://127.0.0.1:8788 \
npm run dev:cloud
```

Windows PowerShell 对应写法：

```powershell
Set-Location frontend
$env:VITE_API_PROXY = 'http://127.0.0.1:8788'
npm run dev:cloud
```

Cloud 前端仍然由 Vite 提供，默认访问 <http://127.0.0.1:5173>；它只是把 API 代理到 Cloud 后端，不会替代 PostgreSQL、Redis 或 Rust API。

## 本地生产构建

### Linux

```bash
cd backend
CARGO_TARGET_DIR=target-local cargo build --release --locked

cd ../frontend
npm ci
npm run build

cd ..
chmod +x scripts/start-local.sh scripts/stop-local.sh
./scripts/start-local.sh
```

默认访问 <http://127.0.0.1:8787>。脚本使用 `backend/target-local/release/backend`、`frontend/dist` 和 `backend/data`，并在 `.runtime` 写入 PID 与日志。停止服务：

```bash
./scripts/stop-local.sh
```

可以通过第一个参数或 `SCULK_PORT` 修改端口，也可以使用 `SCULK_BACKEND_BIN`、`SCULK_STATIC_DIR` 和 `SCULK_DATA_DIR` 覆盖路径。systemd 模板位于 [`deploy/sculk-catalyst.service`](deploy/sculk-catalyst.service)。

### Windows

```powershell
Set-Location backend
$env:CARGO_TARGET_DIR = 'target-local'
cargo build --release --locked

Set-Location ..\frontend
npm ci
npm run build

Set-Location ..
.\scripts\start-local.ps1
```

Windows 启动脚本默认使用 `backend\target-local\release\backend.exe` 和 `frontend\dist`，启动后访问 <http://127.0.0.1:8787>。停止服务：

```powershell
.\scripts\stop-local.ps1
```

如需保持后端前台等待，可使用 `-KeepAlive`；NapCat 连接可通过 `-NapCatApiUrl` 和 `-NapCatConfigPath` 传入。启动脚本会检查 PID 文件对应的确实是当前后端可执行文件，避免误杀其他进程。

### 健康检查

```text
GET http://127.0.0.1:8787/api/health
```

Linux 启动脚本使用 `/api/health`，Windows 启动脚本使用 `/api/dashboard` 等待服务就绪。API 服务未就绪时脚本会失败并保留错误日志供排查。

## Sculk Cloud 与主机 Agent

### Cloud 本地依赖

Cloud 本地开发推荐使用 Docker Compose 这一套端口：PostgreSQL 为 `127.0.0.1:5432`，Redis 为 `127.0.0.1:6379`，Rust 后端默认监听 `127.0.0.1:8787`，前端默认监听 `127.0.0.1:5173`。复制示例配置前，如果根目录已经存在 `.env`，请手动合并变量，不要覆盖已有密钥：

```powershell
docker compose -f docker-compose.cloud.yml up -d
Copy-Item .env.cloud.example .env
```

本地配置至少应确认以下值：

```text
DATABASE_URL=postgres://sculk:sculk_dev_password@127.0.0.1:5432/sculk_cloud
REDIS_URL=redis://127.0.0.1:6379/
SCULK_MASTER_KEY=请替换为至少 24 个字符的随机值
SCULK_CLOUD_PUBLIC_URL=http://127.0.0.1:8787
SCULK_BIND_ADDRESS=127.0.0.1:8787
SCULK_STATE_FILE=data/state-cloud.json
SCULK_ALLOWED_ORIGINS=http://127.0.0.1:5173
```

启动完整 Cloud 开发链路：

```powershell
# 终端 1：从项目根目录启动 Rust API；启动时会自动执行 backend/migrations
Set-Location backend
cargo run

# 终端 2：启动 Cloud 前端
Set-Location ..\frontend
npm ci
$env:VITE_API_PROXY = 'http://127.0.0.1:8787'
npm run dev:cloud
```

打开 <http://127.0.0.1:5173>，进入“设置 > Sculk Cloud”注册首个账号；数据库中的第一个账号会自动成为 Cloud 管理员。`SCULK_MASTER_KEY` 用于 Cloud 上游凭据加密，生产环境必须替换为独立高熵值。

仓库还提供 `scripts/start-cloud.ps1`，但它是另一套 Windows 原生运行链路，不要与 Docker Compose 的端口混用：它要求本机 PostgreSQL 18、`.runtime\redis` 中的 Redis、`backend\target-cloud\debug\backend.exe`，并使用 PostgreSQL `127.0.0.1:55432`、Redis `127.0.0.1:56379`、Cloud 后端 `127.0.0.1:8788`。需要使用该脚本时，应先按脚本要求准备运行时、设置对应 `DATABASE_URL`/`REDIS_URL`，再执行：

```powershell
Set-Location backend
$env:CARGO_TARGET_DIR = 'target-cloud'
cargo build
Set-Location ..
.\scripts\start-cloud.ps1
```

生产环境对外部署必须使用 HTTPS 反向代理，并限制 PostgreSQL、Redis 只允许内网访问。`SCULK_CLOUD_PUBLIC_URL` 必须是 Agent 可访问的 HTTPS 根地址，不能包含账号密码、查询参数或片段；只有 `localhost` 或回环地址的本地开发环境允许使用 HTTP。详细的数据库迁移、会话、Token 和部署边界见 [`docs/SCULK_CLOUD.md`](docs/SCULK_CLOUD.md)。

### 构建和运行 Agent

```bash
cd agent
cargo build --release --locked

agent_bin='./target/release/sculk-agent'
"$agent_bin" pair \
  --cloud "$CLOUD_URL" \
  --code "$PAIRING_CODE" \
  --name 'mc-host' \
  --workspace 'minecraft' \
  --workspace-root '/srv/minecraft' \
  --permissions full \
  --capabilities heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1
"$agent_bin" run
```

Windows PowerShell 对应命令：

```powershell
Set-Location agent
cargo build --release --locked

$agent = '.\target\release\sculk-agent.exe'
& $agent pair `
  --cloud 'https://your-cloud.example.com' `
  --code 'scp_replace_with_pairing_code' `
  --name 'mc-host' `
  --workspace 'minecraft' `
  --workspace-root 'D:\minecraft' `
  --permissions full `
  --capabilities 'heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1'
& $agent run
```

构建产物不会自动安装到 PATH；Windows 路径是 `agent\target\release\sculk-agent.exe`，Linux/macOS 路径是 `agent/target/release/sculk-agent`。Agent 默认只向 Cloud 发起 HTTPS 请求，Windows 默认配置保存在 `%APPDATA%\SculkCatalyst\agent.json`。配对码是一次性短时凭据，完成配对后还需要在 Cloud 控制台确认指纹。下载版命令、配置文件、权限模型和 Shell 风险见 [`docs/SCULK_AGENT.md`](docs/SCULK_AGENT.md)。

## 资源中心

资源 API 有两个前端入口：主控制台默认使用同源只读代理 `/api/resource-catalog`；独立 `/resource-admin` 页面默认直连官方源站 `https://res.mcmy.love`。构建时设置 `VITE_RESOURCE_API_BASE` 可以让独立管理页指向自部署资源域名，例如：

```powershell
$env:VITE_RESOURCE_API_BASE = 'https://resources.example.com'
Set-Location frontend
npm run build
```

主控制台的 Rust 代理由 `SCULK_RESOURCE_API_BASE` 指定上游，未配置时也回退到 `https://res.mcmy.love`。该代理只允许公开资源的 `GET`/`HEAD` 读取，不转发管理写入、Authorization 或 Cookie；创建、修改、删除和上传资源时，应让 `/resource-admin` 直连资源服务，并在 HTTPS 反向代理层配置认证。

资源中心的核心 API 包括：

- `GET /api/catalog/{resource}`：查询资源项目；
- `GET /api/catalog/{resource}/{slug}/versions`：查询项目版本；
- `GET /api/catalog/summary`：读取目录摘要；
- `GET /api/v1/resolve`：按资源、Minecraft 版本和渠道解析兼容版本；
- `GET /api/v1/plugins/search`：搜索插件；
- `GET /api/v1/download/{kind}/{project}/{version}`：下载或重定向到资源文件；
- `GET /api/openapi.json`：查看 OpenAPI 3.1 描述。

本地或独立资源中心使用 `.env.resource-center.example` 配置。常见部署文件包括：

- `deploy/Dockerfile.resource-center`
- `deploy/docker-compose.resources.yml`
- `deploy/sculk-resource.service`
- `deploy/sculk-resource-backup.service`
- `deploy/sculk-resource-backup.timer`
- `deploy/Caddyfile.resources`
- `deploy/Caddyfile.resources.example`
- `scripts/deploy-resource-center.ps1`

资源管理页路径为 `/resource-admin`。浏览器管理页使用 `SCULK_CATALOG_ADMIN_USERNAME`/`SCULK_CATALOG_ADMIN_PASSWORD` 的 Basic Auth，自动化客户端使用 `SCULK_CATALOG_ADMIN_TOKEN` 的 Bearer Token；Caddy 的 `SCULK_RESOURCE_API_TOKEN` 应与后者保持一致，`SCULK_CATALOG_ADMIN_BASIC_AUTH` 仅供 Caddy 匹配浏览器凭证。不要把令牌或密码写入 `VITE_*` 前端构建变量。

对象存储常用配置为：`SCULK_RESOURCE_OBJECT_DIR`（Rust 写入目录，默认 `data/objects`）、`SCULK_RESOURCE_OBJECT_ROOT`（Caddy 静态服务目录，应与前者一致）、`SCULK_RESOURCE_PUBLIC_BASE`（生成公开下载 URL 的基地址）和 `SCULK_RESOURCE_UPLOAD_MAX_BYTES`（默认 256 MiB，实际限制范围 1 MiB–2 GiB）。主控制台代理响应上限由 `SCULK_RESOURCE_PROXY_MAX_BYTES` 控制，默认 16 MiB，实际范围 64 KiB–64 MiB。

如果直接暴露资源中心 Rust 后端，必须至少配置一套服务端认证；不要依赖 Caddy 单层保护，也不要在未配置认证变量时将写接口暴露到公网。完整的对象上传、镜像同步、备份和 Caddy 配置见 [`docs/RESOURCE_CENTER.md`](docs/RESOURCE_CENTER.md)。

## 配置项

本地后端最常用的配置如下：

| 变量 | 作用 | 默认或说明 |
| --- | --- | --- |
| `SCULK_BIND_ADDRESS` | 监听地址 | 本地脚本默认 `127.0.0.1:8787` |
| `SCULK_STATIC_DIR` | 前端静态文件目录 | 发布脚本默认 `frontend/dist` |
| `SCULK_DATA_DIR` | 状态、服务器工作区、托管 Java 的统一根目录 | 默认是后端运行目录下的 `data` |
| `SCULK_STATE_FILE` | JSON 状态文件路径 | 默认是 `SCULK_DATA_DIR/state.json` |
| `SCULK_JAVA_BIN` | 指定 Java 可执行文件 | 优先级高于托管 Java、`JAVA_HOME` 和 PATH |
| `SCULK_ALLOWED_ORIGINS` | CORS 允许的前端来源 | 生产环境填写精确来源，不要使用宽泛通配 |
| `SCULK_RESOURCE_API_BASE` | 主控制台只读代理连接的资源 API 上游 | 未配置时回退到 `https://res.mcmy.love` |
| `SCULK_RESOURCE_PROXY_MAX_BYTES` | 主控制台资源代理的单响应上限 | 默认 16 MiB，实际范围 64 KiB–64 MiB |
| `SCULK_RESOURCE_API_TOKEN` | 反向代理层校验资源同步写请求的令牌 | 由 Caddy 使用，应与 `SCULK_CATALOG_ADMIN_TOKEN` 保持一致 |
| `SCULK_CATALOG_ADMIN_TOKEN` | 资源目录 Rust 写接口 Bearer Token | 至少 16 字符；生产环境必须配置 |
| `SCULK_CATALOG_ADMIN_USERNAME` / `SCULK_CATALOG_ADMIN_PASSWORD` | 资源管理页 Basic Auth | 密码 8–256 字符；只配置在服务端 |
| `SCULK_CATALOG_ADMIN_BASIC_AUTH` | Caddy 匹配完整 Basic 凭证的 Base64 值 | 仅供反向代理使用，不写入前端 |
| `SCULK_RESOURCE_OBJECT_DIR` / `SCULK_RESOURCE_OBJECT_ROOT` | Rust 对象写入目录 / Caddy 静态对象目录 | 两者应指向同一目录 |
| `SCULK_RESOURCE_PUBLIC_BASE` | 资源上传响应中生成公开下载 URL 的基地址 | 生产环境填写 HTTPS 资源域名 |
| `SCULK_RESOURCE_UPLOAD_MAX_BYTES` | 资源上传大小上限 | 默认 256 MiB，实际范围 1 MiB–2 GiB |
| `DATABASE_URL` / `REDIS_URL` | 启用 Cloud 数据层 | 未同时配置时本地模式仍可运行 |
| `SCULK_MASTER_KEY` | Cloud 上游凭据加密密钥 | 至少 24 字符，生产环境使用独立随机值 |
| `SCULK_CLOUD_PUBLIC_URL` | Agent bootstrap 使用的 Cloud 根地址 | 生产必须是 Agent 可访问的 HTTPS 根地址 |
| `SCULK_CLOUD_SESSION_DAYS` / `SCULK_CLOUD_RATE_LIMIT` | Cloud 会话期限 / Token 每分钟请求上限 | 示例值分别为 30 / 60 |
| `SCULK_NAPCAT_API_URL` / `SCULK_NAPCAT_ACCESS_TOKEN` | NapCat OneBot HTTP API 地址 / Bearer Token | 见 `.env.bot.example` |
| `SCULK_BOT_WEBHOOK_TOKEN` | 入站机器人 webhook 共享令牌 | 建议配置高熵随机值 |
| `VITE_API_PROXY` | Vite 开发服务器的 `/api` 代理目标 | 默认 `http://127.0.0.1:8787` |
| `VITE_RESOURCE_API_BASE` | 前端资源 API 地址构建覆盖项 | 不配置时主控制台代理，`/resource-admin` 默认官方源站 |

完整变量名按场景拆分在 `.env.*.example` 中，包括机器人、Cloud、主站资源同步和独立资源中心。`VITE_*` 会进入前端构建产物，只能放公开地址，不能放密码、API Key 或令牌。真实 `.env`、数据库状态、令牌和运行时密钥不会随项目发布。

## 测试与 CI

本地常用检查：

```bash
cd backend
cargo fmt --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings -A clippy::too_many_arguments
cargo test --all-targets --locked

cd ../agent
cargo check --all-targets --locked

cd ../frontend
npm ci
npm run build
```

仓库内还提供：

- `scripts/test-local-server-provision.ps1`：隔离验证核心下载、初始化、取消、重试和后端重启恢复。
- `scripts/test-cloud-agent-tasks.ps1`：验证 Agent 任务、独立团队审批、Shell、checkpoint、恢复、重试和回滚；高风险场景需要传入 `-ApprovalTeamId` 与独立的 `-ApproverToken`。
- `scripts/test-cloud-terminal-conversations.ps1`：验证 Cloud 终端、对话、独立团队审批和输入幂等；高风险场景需要传入 `-ApprovalTeamId` 与独立的 `-ApproverToken`。
- `scripts/cloud-smoke-test.ps1`：Cloud API 冒烟测试。
- `bash -n scripts/*.sh` 与 ShellCheck：Linux 启停脚本检查。

GitHub Actions 工作流见 [`.github/workflows/ci.yml`](.github/workflows/ci.yml)，当前覆盖 Ubuntu/Windows 的后端与主机 Agent 矩阵、Rust 格式化、check、Clippy、测试、Linux shell 检查和 Node 24 前端构建。隔离的 `cloud-smoke` job 会验证数据库迁移、账号与独立审批、加密中转和 Docker Compose 配置；主机 Agent 任务与持久终端的完整 PowerShell E2E 脚本仍需按场景单独运行。

## 常见问题与排错

### 后端或前端端口被占用

- 本地后端通过 `SCULK_BIND_ADDRESS` 修改监听地址，例如 `127.0.0.1:8790`；生产脚本也支持 `SCULK_PORT`。
- 前端通过 `PORT` 修改 Vite 端口，同时把 `VITE_API_PROXY` 指向实际后端地址。
- Cloud Docker 链路的 PostgreSQL、Redis 端口分别是 `5432`、`6379`；不要直接套用 `scripts/start-cloud.ps1` 的 `55432`、`56379`。

### Cloud 不可用

先确认容器和日志：

```powershell
docker compose -f docker-compose.cloud.yml ps
docker compose -f docker-compose.cloud.yml logs postgres redis
Invoke-RestMethod http://127.0.0.1:8787/api/cloud/status
```

再检查 `.env` 中的 `DATABASE_URL`、`REDIS_URL`、`SCULK_MASTER_KEY`、`SCULK_BIND_ADDRESS` 和 `SCULK_ALLOWED_ORIGINS`。后端启动时会执行迁移；如果迁移失败，应先查看启动终端或 `.runtime/backend.err.log`，不要反复删除数据库卷。

### 前端显示 API 请求失败

确认前端启动前设置的 `VITE_API_PROXY` 与后端监听地址一致；修改 Vite 环境变量后需要重启 `npm run dev`。资源管理页若报 CORS 或 401，分别检查 `VITE_RESOURCE_API_BASE`、`SCULK_ALLOWED_ORIGINS` 和资源中心的服务端认证变量。

### Java 或服务器初始化失败

运行 `java -version` 验证系统 Java；如果使用指定路径，检查 `SCULK_JAVA_BIN`，否则依次检查托管 Java、`JAVA_HOME` 和 PATH。初始化任务失败后可以通过工作台重试；后端重启能恢复任务队列，但不会重新接管已经在运行的旧 Java 进程。

### 状态文件备份

本地状态通常位于 `data/state.json`（若从 `backend` 目录启动，则为 `backend/data/state.json`），并可能有 `.bak` 和 `.lock` 文件。备份前先停止对应后端，确保同一状态文件只有一个进程写入：

```powershell
Copy-Item backend\data\state.json backend\data\state.json.manual-backup
Copy-Item backend\data\state.json.bak backend\data\state.json.bak.manual-backup -ErrorAction SilentlyContinue
```

Cloud 的 `data/state-cloud.json` 与本地状态分开；资源中心对象目录和目录 JSON 的备份策略见 [`docs/RESOURCE_CENTER.md`](docs/RESOURCE_CENTER.md)。

## 目录结构

```text
.
├── backend/       Rust + Axum API、任务执行器、服务器进程、资源目录与 Cloud
├── agent/         独立 Rust 主机 Agent
├── frontend/      Vue 3 + TypeScript + Vite 工作台、Cloud 和资源管理页
├── deploy/        systemd、Docker、Caddy 和资源中心部署模板
├── scripts/       本地启动/停止、Cloud、资源中心和 E2E 脚本
├── docs/          功能状态、架构、Cloud、Agent、资源中心和模板文档
├── artifacts/     可公开的界面截图与测试辅助文件
├── LICENSE        项目主体 Apache License 2.0
├── LICENSES/      受限模块的附加许可证
└── NOTICE         文件范围、署名和第三方声明
```

## 已知限制与安全边界

- 本项目默认面向本机或受信任的内网管理环境；后端默认绑定回环地址，不代表已经具备公网安全模型。
- 本地 JSON 状态会保存服务器、对话、任务和资源目录；本地 AI API Key 在当前版本以明文保存在 `state.json` 及其备份中，应严格限制文件权限。
- Cloud 的上游凭据使用 `SCULK_MASTER_KEY` 加密，但生产环境仍需保护数据库、Redis、会话密钥和反向代理。
- Cloud Agent 审批是服务端任务执行门：高风险任务和终端启动必须有同团队、非请求人决定的有效审批；旧无关联高风险排队项在迁移时取消。审批不替代操作系统账户、容器或文件系统沙箱。
- 资源中心有浏览器管理认证和自动化 Bearer Token，但当前没有完整的多管理员 RBAC、限流、对象回收和不可篡改审计；不要直接暴露未加固的写接口。
- Agent 的 `full` Shell 不提供工作区沙箱。审批能限制任务流程，不能替代操作系统权限、容器隔离或专用低权限账号。
- 玩家在线状态、CPU、内存、TPS、经济和反馈部分仍使用本地状态或演示数据；当前没有 RCON、Query 或管理插件的真实同步链路。
- 插件目录可以搜索和下载，但没有自动安装、依赖解析、冲突检查或权限分析。
- Cloud 云资源部署接口当前只提供能力预览和预留路由，创建请求会返回 `501 deployment_planned`，不会创建、计费或调度云资源。
- 外部 MCP、Discord 和监控连接中的部分测试地址仍是演示端点，不表示已经接入真实协议或凭据。

如果发现安全问题，请不要把令牌、数据库导出或真实状态文件提交到 Issue；先移除敏感信息，并通过仓库维护者提供的安全渠道联系项目负责人。

## 文档索引

- [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md)：按模块记录当前实现、缺口和路线。
- [`docs/FUNCTIONAL_DOCUMENTATION.md`](docs/FUNCTIONAL_DOCUMENTATION.md)：面向功能和用户流程的详细说明。
- [`docs/SCULK_CLOUD.md`](docs/SCULK_CLOUD.md)：Cloud 数据库、会话、Token、加密和部署边界。
- [`docs/SCULK_AGENT.md`](docs/SCULK_AGENT.md)：主机 Agent 的下载、配对、权限、任务和终端。
- [`docs/RESOURCE_CENTER.md`](docs/RESOURCE_CENTER.md)：独立资源中心、对象上传、镜像同步和备份。
- [`docs/BOT_INTEGRATIONS.md`](docs/BOT_INTEGRATIONS.md)：QQ/NapCat、Bilibili、抖音及通用 webhook 适配器。
- [`docs/SERVER_TEMPLATE_MANIFEST.md`](docs/SERVER_TEMPLATE_MANIFEST.md)：可导入的便携开服模板格式。

## 许可证

项目采用分区许可：

- 除特别声明外，项目主体使用 [Apache License 2.0](LICENSE)。
- Sculk Cloud 云账号系统相关文件使用 [`PolyForm Noncommercial License 1.0.0`](LICENSES/PolyForm-Noncommercial-1.0.0.md)，允许范围和限制以该许可证为准。
- 文件范围、署名要求和第三方声明见 [`NOTICE`](NOTICE)。未在 NOTICE 中另行列出的文件适用 Apache License 2.0。

PolyForm Noncommercial 部分属于 source-available，不属于 OSI 定义的开源软件。商业使用 Sculk Cloud 相关受限文件前，请先取得原作者授权。
