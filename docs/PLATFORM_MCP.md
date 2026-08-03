# B站与抖音独立 MCP 适配器

平台程序与主后端、Sculk Agent 分离运行：

~~~text
B站/抖音独立程序
  ├─ 平台授权、评论读取、平台回复
  └─ MCP stdio
        ↓
Sculk Agent
  ├─ 读取任务并启动已配置的 MCP 子进程
  └─ 只允许受限的 platform.mcp.* 操作
        ↓
Sculk Cloud
  ├─ 只下发 server/tool/arguments
  └─ 回复评论任务必须人工审批
~~~

## 构建

~~~powershell
cargo build --release --manifest-path platform-bots/Cargo.toml --bins
~~~

产物：

- platform-bots/target/release/sculk-douyin.exe
- platform-bots/target/release/sculk-bilibili.exe

两个程序都使用 MCP stdio JSON-RPC，不监听公网端口。标准输出只写 MCP 响应，诊断信息写标准错误。

## MCP 工具

两个程序提供相同工具：

- platform_status：查看连接器是否配置，不输出令牌；
- list_comments：读取指定视频评论；
- reply_comment：回复评论，实际发送前必须经过 Sculk Cloud 的 platform.mcp.reply 审批任务。

抖音默认使用评论相关 Open API 路径；仍然需要在抖音开放平台申请权限并配置 OAuth access-token。B站不预设未知网页接口，必须配置官方或经过授权的合规连接器。

## Agent 配置

在 paired agent.json 中加入 mcp-v1 能力和 MCP 程序配置。令牌不要写进这个文件：

~~~json
{
  "capabilities": [
    "heartbeat",
    "tasks-v1",
    "task-checkpoints-v1",
    "mcp-v1"
  ],
  "mcp_servers": [
    {
      "id": "douyin",
      "command": "D:\\sculk\\sculk-douyin.exe",
      "args": [],
      "enabled": true
    },
    {
      "id": "bilibili",
      "command": "D:\\sculk\\sculk-bilibili.exe",
      "args": [],
      "enabled": true
    }
  ]
}
~~~

Agent 使用无 Shell 的子进程方式启动 command，因此不会把命令拼接后交给 PowerShell。平台环境变量应配置在 Agent 启动环境中。

## 本地扫码登录

平台程序有两种启动方式：

- 直接双击，或在终端执行 `sculk-douyin.exe --login` / `sculk-bilibili.exe --login`：启动本地登录页，并自动打开浏览器。
- 由 SculkAgent 通过标准输入启动：保持 MCP stdio 模式，不打开登录页。

抖音示例（PowerShell）：

```powershell
$env:SCULK_DOUYIN_CLIENT_KEY="你的抖音开放平台 Client Key"
$env:SCULK_DOUYIN_CLIENT_SECRET="你的抖音开放平台 Client Secret"
$env:SCULK_DOUYIN_REDIRECT_URI="http://127.0.0.1:18432/oauth/callback"
& .\sculk-douyin.exe --login
```

然后在 `http://127.0.0.1:18432/` 点击授权。抖音官方授权页支持扫码/授权，授权完成后会回调本机页面；回调地址必须与开放平台配置完全一致。B 站使用同样流程，但必须填写 B 站官方开放平台审核通过的 `SCULK_BILIBILI_AUTH_URL`、`SCULK_BILIBILI_TOKEN_URL`、Client ID 和 Secret，程序不会使用网页 Cookie、逆向接口或私有接口。

授权账号只保存在本机平台数据目录，访问令牌使用本机生成的 AES-256-GCM 密钥加密，API 页面和日志不会展示令牌。默认目录为 `%APPDATA%\SculkCatalyst\platform-bots\<platform>`，可用 `SCULK_PLATFORM_DATA_DIR` 指定到 D 盘目录。

若仅需验证程序是否启动，可访问 `/api/status`；返回 `ready: false` 代表 OAuth 环境变量未配置，不代表程序崩溃。

## Cloud 任务

读取评论：

~~~json
{
  "operation": "platform.mcp.read",
  "input": {
    "server": "douyin",
    "tool": "list_comments",
    "arguments": {
      "video_id": "item-id",
      "limit": 20
    }
  }
}
~~~

发送回复：

~~~json
{
  "operation": "platform.mcp.reply",
  "input": {
    "server": "douyin",
    "tool": "reply_comment",
    "arguments": {
      "video_id": "item-id",
      "comment_id": "comment-id",
      "content": "服务器地址是 mc.mc520.love，版本为 Leaves 26.2。",
      "dry_run": false
    }
  }
}
~~~

platform.mcp.read 只允许 platform_status 和 list_comments；platform.mcp.reply 只允许 reply_comment，并被标记为高风险写操作。
