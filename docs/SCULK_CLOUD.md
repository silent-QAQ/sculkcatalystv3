# Sculk Cloud 账号体系

Sculk Cloud 使用 PostgreSQL 保存账号、团队、审批和用量事实，使用 Redis 保存短期会话缓存与 API 分钟级限流计数。现有本地服务器工作台仍使用 JSON 状态文件，两套数据职责互不覆盖；`scripts/start-cloud.ps1` 默认设置 `SCULK_STATE_FILE=data/state-cloud.json`，避免与 8787 本地后端共写 `state.json`。

## 本地启动

```powershell
# 项目根目录
docker compose -f docker-compose.cloud.yml up -d
Copy-Item .env.cloud.example .env

# 终端 1
Set-Location backend
cargo run

# 终端 2
Set-Location frontend
npm run dev
```

后端会读取当前目录或项目根目录的 `.env`，启动时自动执行 `backend/migrations`。打开 `http://127.0.0.1:5173`，在“设置 > Sculk Cloud”注册首个账号。数据库中的首个账号自动成为云管理员，可以配置 API 中转上游。

生产环境必须替换 `SCULK_MASTER_KEY` 和 PostgreSQL 密码，并限制 PostgreSQL、Redis 只允许内网访问。对外服务时设置 `SCULK_BIND_ADDRESS=0.0.0.0:8787` 与精确的 `SCULK_ALLOWED_ORIGINS`，并在 Rust API 前配置 HTTPS 反向代理。

## 账号与同步

- 密码使用 Argon2id 哈希保存。
- 登录凭证为高熵不透明 Token；PostgreSQL 只保存 SHA-256 摘要，Redis 缓存 15 分钟会话视图。
- 每次登录创建独立设备和会话，用户可以远程撤销其他设备。
- 设置同步使用单调递增版本号。`PUT /api/cloud/sync/settings` 必须提交 `base_version`，版本落后时返回 HTTP 409，不会静默覆盖云端。

## 团队与审批

团队角色包括 `owner`、`admin`、`approver` 和 `member`。邀请绑定邮箱并在 7 天后过期。团队成员都可以发起审批，只有所有者、管理员和审批人可以通过或拒绝。

审批记录保存请求人、风险等级、业务负载、决定人、意见与时间，适合后续接入服务器高风险操作。当前远程审批接口已经可用，但尚未自动接管本地 `automation` 任务。

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
| 设置同步 | `GET/PUT /api/cloud/sync/settings` |
| 团队 | `GET/POST /api/cloud/teams`、成员、邀请、接受邀请 |
| 审批 | `GET/POST /api/cloud/approvals`、`POST /api/cloud/approvals/{id}/decision` |
| Token 与用量 | `GET/POST /api/cloud/tokens`、`DELETE /api/cloud/tokens/{id}`、`GET /api/cloud/usage` |
| 中转管理 | `GET/PUT /api/cloud/admin/relay-provider` |
| OpenAI 兼容中转 | `POST /api/cloud/v1/chat/completions` |
| 云部署预留 | `GET /api/cloud/deployments/capability`、`GET/POST /api/cloud/deployments` |

## 云部署预留

`GET /api/cloud/deployments/capability` 返回预览版本和预留端点；`GET /api/cloud/deployments` 返回空集合；`POST /api/cloud/deployments` 返回 HTTP 501 和 `deployment_planned`。当前版本不会创建、计费或调度任何云资源。
