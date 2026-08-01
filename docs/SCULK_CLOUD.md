# Sculk Cloud 账号体系

Sculk Cloud 使用 PostgreSQL 保存账号、团队、审批和用量事实，使用 Redis 保存短期会话缓存与 API 分钟级限流计数。现有本地服务器工作台仍使用 JSON 状态文件，两套数据职责互不覆盖；`scripts/start-cloud.ps1` 默认设置 `SCULK_STATE_FILE=data/state-cloud.json`，避免与 8787 本地后端共写 `state.json`。

## 本地启动

```powershell
# 项目根目录
docker compose -f docker-compose.cloud.yml up -d
Copy-Item .env.cloud.example .env
# 编辑 .env：为 SCULK_POSTGRES_PASSWORD、SCULK_REDIS_PASSWORD 和
# SCULK_MASTER_KEY 写入不同的高熵随机值，并把前两个值同步到
# DATABASE_URL 与 REDIS_URL。密码建议使用 URL 安全字符；否则需 URL 编码。

# 终端 1
Set-Location backend
cargo run

# 终端 2
Set-Location frontend
npm run dev
```

后端会读取当前目录或项目根目录的 `.env`，启动时自动执行 `backend/migrations`。打开 `http://127.0.0.1:5173`，在“设置 > Sculk Cloud”注册首个账号。数据库中的首个账号自动成为云管理员，可以配置 API 中转上游。

`docker-compose.cloud.yml` 仅把 PostgreSQL 和 Redis 发布到 `127.0.0.1`，并要求显式设置 `SCULK_POSTGRES_PASSWORD`、`SCULK_REDIS_PASSWORD`。`SCULK_MASTER_KEY` 同样是必填项：后端会拒绝空值以及 `replace-with-*`、`change-me`、`example*` 等公开占位值。生产环境应使用与开发环境不同的高熵值，并限制 API 仅经 HTTPS 反向代理对外访问。对外服务时设置 `SCULK_BIND_ADDRESS=0.0.0.0:8787` 与精确的 `SCULK_ALLOWED_ORIGINS`。若启用 Agent 一键启动包，必须将 `SCULK_CLOUD_PUBLIC_URL` 设置为 Agent 可访问的 HTTPS 公网根地址；只有 `localhost` 或回环地址的开发环境允许使用 HTTP。

## 账号与同步

- 密码使用 Argon2id 哈希保存。
- 登录凭证为高熵不透明 Token；PostgreSQL 只保存 SHA-256 摘要，Redis 缓存 15 分钟会话视图。
- 每次登录创建独立设备和会话，用户可以远程撤销其他设备。
- 设置同步使用单调递增版本号。`PUT /api/cloud/sync/settings` 必须提交 `base_version`，版本落后时返回 HTTP 409，不会静默覆盖云端。
- 登录成功后，前端会自动拉取并应用云端工作区；工作区包含界面设置、快捷提示词、Skill 链接、开服参数模板和不含密钥的模型元数据。旧版 `{ ui }` 快照会自动迁移，当前工作区结构为 `schema_version: 3`。
- 通用同步负载最大为 1 MiB，并递归拒绝密码、API Key、访问令牌和私钥等敏感字段。公开站点使用浏览器本地缓存承接每个账号的界面配置，不写入服务端共享的本地工作台状态。

## 个人 API 凭据

用户上传的 API Key 不进入通用设置快照，而是按用户隔离写入 `cloud_user_api_credentials`：

- 原值使用由 `SCULK_MASTER_KEY` 派生的 AES-256-GCM 密钥加密，随机 nonce 与密文分列保存。
- SHA-256 指纹用于同一用户内去重和识别；这部分是不可逆摘要，不用于恢复 API Key。
- 列表接口只返回首尾掩码和 12 位指纹，不提供明文下载接口。
- API Key 若只保存不可逆哈希，后续无法代用户调用上游，所以采用“可恢复加密密文 + 不可逆哈希指纹”的组合。

## 团队与审批

团队角色包括 `owner`、`admin`、`approver` 和 `member`。邀请绑定邮箱并在 7 天后过期。团队成员都可以发起审批，只有所有者、管理员和审批人可以通过或拒绝。

审批记录保存请求人、风险等级、业务负载、决定人、意见与时间。Cloud Agent 的 high/critical 任务与持久终端会在同一事务中创建唯一的团队审批记录，并通过 `agent_task_id` 或 `terminal_session_id` 建立关联；审批通过后才会进入可租约状态，拒绝/取消会同步关闭资源。请求人不能处理自己的审批，只有团队 `owner`、`admin` 或 `approver` 可以决定。重试和回滚会创建新的资源与新的审批，不继承旧决定；这套机制不等同于本地 JSON `automation` 任务的通用执行器。

## 主机代理

Sculk Agent 由 Minecraft 主机主动连接 Cloud，不要求主机拥有可入站访问的公网 IP，也不开放本地控制台端口。配对流程分为账号生成一次性配对码、Agent 匿名领取、账号核对指纹并确认三步：

- 配对码 10 分钟内有效且只能领取一次；数据库只保存 SHA-256 哈希。生成新的待配对请求会立即使同一账号先前待领取的配对码过期。
- Agent 凭据以 `sca_` 开头，只在领取时返回一次，数据库同样只保存哈希。
- 用户可以在 Cloud 控制台确认或撤销 Agent；撤销后心跳凭据立即失效。
- 已认证的 `POST /api/cloud/agent-bootstrap` 会返回可嵌入下载包的 JSON：一次性配对码、可信 Cloud 地址、主机与工作区元数据，以及当前账号明确批准的完整 Agent 默认能力和权限。响应不包含账号密码、会话或 Agent 凭据。
- 在线状态由 90 秒内的真实心跳推导，不由前端模拟。
- Agent 会通过出站心跳报告真实的 `commands_available` 状态，并轮询任务与终端命令租约。低风险只读任务可以直接执行；写入、Shell 和终端启动必须经过关联团队审批。终端输入会在数据库中加密保存，Agent 确认投递后立即移除可恢复内容；终端输出会在持久化前脱敏，跨分块的疑似凭据会以安全占位内容替代。`log.tail` 限制在日志目录并对常见密钥/Token 脱敏，Windows Shell 使用 Job Object，Unix Shell 使用进程组清理子进程。

迁移会对旧数据采取 fail-closed 策略：没有可验证审批关联的旧高风险任务和未启动终端会话不会继续排队等待；已经运行的旧会话保留用于显式终止和租约回收。旧高风险任务若仍处于租约中，租约过期后会标记为失败而不会重新排队。新建的每次重试、回滚或终端会话都必须生成新的审批记录。

独立程序的下载、命令和配置位置见 [`SCULK_AGENT.md`](SCULK_AGENT.md)。

## API 中转

管理员通过控制台配置 OpenAI 兼容上游。上游 API Key 使用 `SCULK_MASTER_KEY` 派生的 AES-256-GCM 密钥加密后写入 PostgreSQL。

用户创建的个人 Token 只在创建时显示一次。调用示例：

```powershell
$headers = @{ Authorization = "Bearer sk-sc_your_token" }
$body = @{
  model = "gpt-5-mini"
  messages = @(@{ role = "user"; content = "hello" })
} | ConvertTo-Json -Depth 5

Invoke-RestMethod `
  -Uri http://127.0.0.1:8787/api/cloud/v1/chat/completions `
  -Method Post -Headers $headers -ContentType application/json -Body $body
```

当前端点仅支持非流式请求。每个 Token 默认限制为每分钟 60 次，可通过 `SCULK_CLOUD_RATE_LIMIT` 调整。状态码、延迟、模型与上游返回的 Token 用量会写入 `cloud_api_usage`。

## 接口概览

| 范围 | 主要接口 |
| --- | --- |
| 认证 | `POST /api/cloud/auth/register`、`login`、`logout` |
| 资料与设备 | `GET/PATCH /api/cloud/me`、`GET /api/cloud/devices`、`DELETE /api/cloud/devices/{id}` |
| 主机代理 | `POST /api/cloud/agent-pairings`、`POST /api/cloud/agent-bootstrap`、匿名 `POST /api/cloud/agent-pairings/claim`、`GET /api/cloud/agents`、`POST /api/cloud/agents/{id}/confirm`、`DELETE /api/cloud/agents/{id}`、`POST /api/cloud/agent/heartbeat` |
| 设置同步 | `GET/PUT /api/cloud/sync/settings`（设置、提示词、Skill 链接、开服参数模板） |
| 加密凭据 | `GET/POST /api/cloud/credentials`、`DELETE /api/cloud/credentials/{id}` |
| 团队 | `GET/POST /api/cloud/teams`、成员、邀请、接受邀请 |
| 审批 | `GET/POST /api/cloud/approvals`、`POST /api/cloud/approvals/{id}/decision` |
| Agent 任务 | `GET/POST /api/cloud/agent-tasks`、`GET /api/cloud/agent-tasks/{id}`、取消、重试、回滚；Agent 端使用心跳、租约、事件、检查点和完成接口 |
| 持久终端与对话 | `GET/POST /api/cloud/terminal-sessions`、输入、调整大小、终止、事件；`GET/POST /api/cloud/conversations` 及计划任务接口 |
| Token 与用量 | `GET/POST /api/cloud/tokens`、`DELETE /api/cloud/tokens/{id}`、`GET /api/cloud/usage` |
| 中转管理 | `GET/PUT /api/cloud/admin/relay-provider` |
| OpenAI 兼容中转 | `POST /api/cloud/v1/chat/completions` |
| 云部署预留 | `GET /api/cloud/deployments/capability`、`GET/POST /api/cloud/deployments` |

## 云部署预留

`GET /api/cloud/deployments/capability` 返回预览版本和预留端点；`GET /api/cloud/deployments` 返回空集合；`POST /api/cloud/deployments` 返回 HTTP 501 和 `deployment_planned`。当前版本不会创建、计费或调度任何云资源。
