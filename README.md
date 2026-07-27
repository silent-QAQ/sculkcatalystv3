# Sculk Catalyst V3

AI 驱动的 Minecraft 开服管理工具原型。前端使用 Vue 3 + TypeScript，后端使用 Rust + Axum。

完整功能现状、已知限制和开发路线见 [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md)。Sculk Cloud 的数据库部署、安全约束与接口说明见 [`docs/SCULK_CLOUD.md`](docs/SCULK_CLOUD.md)。

## 本地运行

~~~powershell
# 终端 1：Rust API
cd backend
cargo run

# 终端 2：Vue 前端
cd frontend
npm run dev
~~~

打开 http://127.0.0.1:5173。

## Sculk Cloud

云账号、设置同步、团队协作、远程审批和 OpenAI 兼容 API 中转使用 PostgreSQL 与 Redis：

~~~powershell
docker compose -f docker-compose.cloud.yml up -d
Copy-Item .env.cloud.example .env
cd backend
cargo run
~~~

首次注册的云账号自动成为管理员。前端仍通过 `npm run build` 输出可独立部署的静态 HTML、CSS 与 JavaScript。

## 当前原型

- 三栏式多服务器工作台，每台服务器支持多个持久化对话任务
- AI 对话与开服任务流（SSE 流式输出，可接入 OpenAI 格式模型；对话可分组、固定、归档、分叉和标记未读）
- 模型提供商管理：多提供商、模型同步、hi 测试、情景模型绑定、三档审核模式
- 服务器状态、性能与启停交互
- 文件管理与配置预览
- 服务器终端命令交互
- 核心库、插件库与版本管理资源中心
- 多源核心下载、进度、取消与 SHA-256 校验
- Rust API 提供服务器数据和 AI 意图响应
- Skills、MCP、Codex 镜像服协作入口

## 后续架构建议

- 使用 Tauri 2 打包桌面端，并通过 sidecar 管理 Java 与服务端进程
- 引入 SQLite 保存服务器、任务、审计记录与玩家反馈
- 将 AI 行为拆分为可授权工具，并为停服、数据覆盖和经济调整设置审批门
- 通过 MCP 客户端管理 Codex、插件仓库、Discord 和监控系统连接

## MVP 持久化能力

- Rust 后端将服务器状态、对话、AI 配置、任务和日志保存到 `backend/data/state.json`；写盘使用同目录临时文件、原子提交和上一版本 `state.json.bak`，损坏时自动恢复
- 同一状态文件只允许一个后端进程持有；并行实例通过 `SCULK_STATE_FILE` 指定独立文件，Cloud 启动脚本默认使用 `data/state-cloud.json`
- 每台服务器独立保存最大内存；直接启动 Java 与生成的 `start.ps1` 共用同一组 `-Xms/-Xmx` 参数
- 启动、停止和重启状态通过后端 API 管理
- server.properties 支持读取、编辑与保存
- 终端命令通过 Rust API 执行并记录审计日志

## 实时终端与日志

- `GET /api/servers/{id}/ws/logs` WebSocket 端点：连接后同步最近 200 行历史日志，随后实时推送新日志
- Java 进程 stdout/stderr、启停事件与命令输出统一经广播通道推送
- 服务器进程运行时，终端命令真实写入 Java 标准输入；未运行时返回带 `SIM` 标记的模拟输出
- 前端终端标签页自动建立 WS 连接（显示“实时连接”），断线自动回退轮询模式

## 首次开服向导

- 四步流程：名称与位置、服务器参数、环境检查、确认创建
- 普通创建按参数生成工作区；智能创建只建立规划项目与「开服规划」对话，不预设核心、不创建文件
- 本机数据目录可直接选择；远程服务器位置已预留连接接口，当前暂不执行远程创建
- 自动检测本机 Java 版本、系统架构与工作区可写性
- 从镜像资源目录动态读取核心与兼容 Minecraft 版本；目录不可用时回退到内置 Paper、Purpur、Fabric 与 Velocity 选项
- 自动生成独立目录、server.properties、eula.txt、start.ps1、plugins 和 logs
- 创建后自动加入核心下载与首次测试启动任务

## 镜像资源中心

- 内置核心库与插件库，项目资料、版本记录和下载计数持久化到 `backend/data/state.json`
- 核心/插件项目使用 `GET|POST /api/catalog/{cores|plugins}` 与 `GET|PUT|DELETE /api/catalog/{cores|plugins}/{slug}`；版本使用对应的 `/versions` 与 `/versions/{version}` 路径完成查询、新建、编辑和删除
- 项目支持关键词、Minecraft 版本和发行渠道筛选；版本支持关键词、Minecraft 版本和渠道筛选
- `GET /api/v1/resolve` 按资源类型、项目、Minecraft 版本与渠道解析最新的兼容已发布版本
- `GET /api/v1/download/{kind}/{project}/{version}` 提供稳定下载路径，记录下载次数后以 HTTP 307 跳转到版本配置的上游文件
- 资源中心内置快速开始、目录查询、版本解析、文件下载和错误处理文档，并通过 `GET /api/openapi.json` 提供 OpenAPI 3.1 描述
- 原有 `GET /api/download/mirrors` 与 `POST /api/download/preview` 继续用于开服器选择镜像源和预览候选地址

## 实际核心下载

- `POST /api/servers/{id}/download/core` 启动后台下载，`GET /api/servers/{id}/download/status` 查询阶段、来源、字节数和百分比，`POST /api/servers/{id}/download/cancel` 请求取消
- 下载器优先解析资源目录中的兼容已发布版本，再按优先级尝试启用镜像以及 Paper/Velocity/Purpur 官方源；预留的 `example.com` 镜像不会进入执行队列
- 文件先流式写入 `server.jar.part`，同步计算 SHA-256；目录制品同时强制校验文件大小与可信摘要，通过后以可回滚方式替换 `server.jar`
- 下载进行中可取消，失败或取消会清理临时文件，并同步任务状态、服务器状态、日志和前端进度
- 下载与启动对同一服务器互斥；并发请求不能重复启动下载或 Java 进程

## 镜像服务限制

- 当前资源中心是 JSON 目录与上游重定向服务，不在本机托管二进制对象；尚无对象存储、上传、CDN、Range、ETag 或 HTTP 断点续传
- 核心下载器会在失败或取消后清理 `.part` 文件，重新下载会从头开始；下载状态保存在内存中，后端重启后不会恢复
- SHA-256 会始终计算，但只有提供可信预期摘要的来源才能执行一致性比对；自定义模板镜像和部分官方源目前可能只记录摘要
- 目录管理 CRUD 尚无登录、API Key 或 RBAC，仅适合本机使用，不应直接暴露到公网
- 插件资源已经可以检索、管理版本和通过稳定接口下载，但尚未自动安装到服务器 `plugins/`，也未实现依赖与冲突解析

## AI 模型接入

- 设置页支持添加多个 OpenAI 格式提供商（官方 API 或中转站，base_url 末尾带不带 `/v1` 均可，API Key 可留空用于本地网关）
- `POST /api/ai/providers/{id}/models/sync` 从上游 `/v1/models` 读取模型列表；每个模型可单独启用并做 "hi" 连通性测试
- 五个情景（对话、管理/自动化、开服向导、配置编写、社区分析）可分别绑定模型，未绑定时回退默认模型；两家中转站可同时提供同名模型
- `POST /api/chat/stream` 提供 SSE 流式对话；每个对话独立保存模型与 Agent 选择，切换对话或重启后恢复，无可用提供商时自动回退本地规则回复
- 审核模式三档：请求批准（中高风险需人工）、替我审核（AI 自动批中风险）、完全访问权限（全部自动执行），全局持久化并作用于任务创建
- API Key 明文存于本地 `state.json` 及上一版本 `.bak`，接口响应仅返回脱敏密钥；编辑时留空表示保持不变

## ACP Agent 接入

- 设置页可通过 ACP 协议（stdio JSON-RPC）接入外部 Agent：Codex CLI、Claude Code CLI、OpenClaw、Hermes 或任意自定义命令
- `POST /api/ai/agents/{id}/test` 做 initialize 握手测试；`PUT /api/ai/agents/active` 设置默认对话 Agent
- 对话栏提供独立的 Agent、模型与审核模式快捷选择器；使用 ACP Agent 时隐藏不生效的直连模型选择，切回内置 Agent 后恢复
- Agent 的流式回复（`session/update` agent_message_chunk）转为 SSE 转发；权限请求按审核模式自动应答，文件读写请求当前一律拒绝
- Agent 失败自动回退内置模型直连，再回退本地规则

## 业务模块

- AI 自动化：创建低、中、高风险任务，支持审批、取消、进度与审计状态
- 玩家管理：玩家状态、身份、资产、游戏时长与处罚操作预览
- 经济运营：总资产、通胀指标和经济调控任务入口
- 意见收集：反馈列表、情绪标记、分类统计与 AI 聚类摘要
- 玩法投票：创建多选项投票、玩家投票与票数持久化
- Skills：能力包列表、来源、版本与启停状态
- MCP：连接启停、能力声明、端点展示、延迟测试和状态持久化

外部 MCP、Discord 与监控地址当前使用演示端点；接入真实凭证和协议客户端后即可替换连接测试实现。

## 安全文件管理

- `GET /api/servers/{id}/files` 浏览服务器工作区目录
- `GET /api/servers/{id}/file` 读取 UTF-8 文本文件，最大 2 MB
- `PUT /api/servers/{id}/file` 保存配置、脚本、日志和 Markdown 等文本文件
- `POST /api/servers/{id}/directory` 在服务器工作区中新建目录
- 拒绝绝对路径、`..` 路径穿越、符号链接和 `server.jar` 等二进制覆盖

## 开源协议

本项目采用分区许可：

- 除特别声明的文件外，项目主体基于 [Apache License 2.0](LICENSE) 开源，允许使用、分发和二次修改；分发修改版本时必须保留原作者、版权与许可声明，并注明修改内容。
- Sculk Cloud 云账号系统相关文件基于 [PolyForm Noncommercial License 1.0.0](LICENSES/PolyForm-Noncommercial-1.0.0.md) 提供，仅允许协议规定的非商业用途，禁止未经授权的商业使用。
- 完整的受限文件范围与署名要求见 [NOTICE](NOTICE)。未明确列入 `NOTICE` 的文件适用 Apache License 2.0。

PolyForm Noncommercial 许可部分属于源码可用（source-available），不属于 OSI 定义的开源软件。如需商业使用 Sculk Cloud，请联系原作者另行取得商业授权。
