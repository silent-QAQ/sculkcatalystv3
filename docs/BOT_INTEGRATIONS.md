# 机器人适配器

机器人功能是可选扩展，不安装时不会接收或发送平台消息。当前内置四种适配器：

- QQ / NapCat：OneBot 11 HTTP API，支持群聊和私聊回复。
- Bilibili 评论：标准化评论 Webhook；评论回复通过外部桥接地址完成。
- 抖音评论：标准化评论 Webhook；评论回复通过外部桥接地址完成。
- 其他视频平台 Webhook：复用同一套评论筛选、意向识别和知识引导逻辑。

## 配置

复制根目录的 `.env.bot.example` 到部署环境，并按实际服务修改：

- `SCULK_NAPCAT_API_URL`：NapCat OneBot HTTP API 地址。
- `SCULK_NAPCAT_ACCESS_TOKEN`：NapCat HTTP API 的 Bearer Token，可为空。
- `SCULK_BOT_WEBHOOK_TOKEN`：Sculk 接收入站消息时校验的 `X-Sculk-Bot-Token` 或 `Authorization: Bearer ...`。
- `SCULK_BILIBILI_REPLY_URL` / `SCULK_DOUYIN_REPLY_URL`：评论回复桥接服务地址。Sculk 不保存平台 Cookie、Access Token，也不绕过平台风控。

启动后在“集成 / 机器人扩展”中安装并启用适配器，再填写 QQ 群号或加群链接、回复模式、关键词、意向阈值、PCL2/模组包/规则/知识库链接和冷却时间。

## Webhook 地址

```text
POST /api/bots/qq-napcat/webhook
POST /api/bots/bilibili-comments/webhook
POST /api/bots/douyin-comments/webhook
POST /api/bots/video-webhook/webhook
```

QQ 适配器直接接受 OneBot 11 的消息事件。例如群消息：

```json
{
  "post_type": "message",
  "message_type": "group",
  "group_id": 123456789,
  "user_id": 987654321,
  "message_id": 1001,
  "raw_message": "想进服务器一起玩",
  "self_id": 1122334455
}
```

视频平台或桥接服务可以发送以下最小格式；字段名也兼容 `comment`、`uid`、`cid`、`video_id` 等常用变体：

```json
{
  "platform": "bilibili",
  "comment_id": "c-1001",
  "video_id": "av1001",
  "user_id": "u-2001",
  "content": "怎么进服务器？"
}
```

命中策略后，Sculk 返回生成的回复，并向对应桥接地址 POST：

```json
{
  "platform": "bilibili",
  "comment_id": "c-1001",
  "video_id": "av1001",
  "user_id": "u-2001",
  "content": "怎么进服务器？",
  "reply": "..."
}
```

桥接服务应自行使用平台官方授权接口完成评论回复，并处理平台限流、审核和失败重试。没有配置出站桥接地址时，Sculk 仍会返回生成的回复和事件记录，但不会声称已在平台发布。

## 运行规则

- `all` 模式回复所有非空评论；`keywords` 模式只回复包含配置关键词的评论。
- 关键词匹配后会计算 Minecraft 游玩意向分数。超过阈值时才追加 QQ 群引导和服务器规则链接。
- 询问启动器、模组、规则或 Minecraft 版本时，会追加对应资源链接。
- 同一评论只处理一次；没有评论 ID 时按“适配器 + 用户 + 目标视频/会话”应用冷却时间。
- 回复记录只保留最近 100 条，平台 Token 和 Cookie 不写入状态文件。

QQ 适配器的事件字段和 `send_group_msg` / `send_private_msg` 动作遵循 NapCat 的 OneBot 11 接口；部署前请在 NapCat 的 HTTP 服务中把事件推送地址指向上述 Webhook。
