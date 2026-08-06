# Sculk Catalyst Paper Bridge

面向 Sculk Catalyst 玩家管理的只读 Paper/Folia 桥接插件。插件主动连接后端 WebSocket，提供在线状态、实时等级和坐标、背包/装备/末影箱快照、有限的潜影盒与收纳袋预览，以及受白名单限制的 PlaceholderAPI 字段。

## 运行范围

| 项目 | 支持范围 |
| --- | --- |
| Java | 21 |
| Paper API 编译基线 | 1.21.6-R0.1-SNAPSHOT |
| Paper 运行目标 | Paper 1.21.6+ |
| Folia | 使用全局区域与实体调度器的兼容实现 |
| PlaceholderAPI | 可选软依赖 |

`plugin.yml` 和 `paper-plugin.yml` 都声明了 `folia-supported: true`。插件不注册命令、不授予权限、不执行控制台命令，也不会修改等级、位置、物品、末影箱或任何玩家数据。

## 安装与配置

1. 使用 Java 21 构建插件，或取得对应的桥接 JAR。
2. 将 `build/libs/sculk-catalyst-paper-bridge-0.1.0-SNAPSHOT.jar` 放入目标 Paper/Folia 服务器的 `plugins/` 目录。
3. 首次启动后编辑 `plugins/SculkCatalystPaperBridge/config.yml`，设置 `enabled`、稳定的 `server-id`、`backend-ws-url` 和每服独立的高熵 `token`。
4. 在后端使用相同密钥配置 `SCULK_BRIDGE_TOKENS=server-id=token`；仅本机开发可使用 `SCULK_BRIDGE_TOKEN` 作为默认密钥。
5. 重启服务器。配置不会通过命令热重载；可通过 `GET /api/servers/{server_id}/bridge/status` 确认连接和能力。

生产环境必须使用 `wss://`，并让后端仅接受受控的桥接端点。默认的 `ws://127.0.0.1:8787/api/bridge/v1/ws` 只适合本机开发。

```yaml
enabled: true
server-id: "survival-01"
backend-ws-url: "wss://panel.example.com/api/bridge/v1/ws"
token: "replace-with-a-unique-high-entropy-secret"

papi:
  enabled: true
  fields:
    balance: "%vault_eco_balance%"
    primary_group: "%luckperms_primary_group_name%"
```

PlaceholderAPI 为可选软依赖。`papi.fields` 的键仅用于配置可读性，值才是实际变量白名单；即使前端指定了显示字段，插件也只会解析与此映射任一值精确相同的表达式，响应仍按后端字段 ID 返回。未安装 PlaceholderAPI、禁用 PAPI、字段未白名单或扩展解析失败时会返回明确状态，绝不通过控制台执行 `papi parse`。

## 协议

桥接使用严格的 v2 JSON 信封。业务 payload 不是顶层 `payload` 字段：`payload_json` 是原始 UTF-8 JSON 的无填充 Base64URL 编码，签名覆盖信封元数据和该原始 payload 的 SHA-256 摘要。

```json
{
  "protocol_version": 2,
  "type": "snapshot_request",
  "request_id": "req-42",
  "server_id": "survival-01",
  "instance_id": "ef3e34c6-c123-4f91-9d83-d3c021d8ee1c",
  "session_id": "9ab8b6c8-c586-4a19-a4a8-0df05a9dc915",
  "seq": 18,
  "sent_at": 1725000000000,
  "payload_json": "eyJwbGF5ZXJfdWlkIjoiYjU0Y2YzZjgtOWY1MS00YzRjLWEzZDEtOTU4NGZjZTI4MjJjIn0",
  "signature": "base64url-hmac-sha256"
}
```

`request_id` 用于关联请求和响应，非请求消息为 JSON `null`。`seq` 在单个发送方向内单调递增，`sent_at` 是 Unix 毫秒。已建立会话的帧必须携带 `session_id` 与 `signature`；`hello_init` 和 `challenge` 的这两个字段为 `null`，签名 `hello` 的 `session_id` 仍为 `null`。收到不匹配的协议版本、服务器/实例标识、会话、序号、时间戳或签名时，连接会被拒绝。

### v2 握手与会话

1. 插件发送不带签名的 `hello_init`，其中包含随机 `client_nonce`。
2. 后端返回仅对当前连接有效、带过期时间的 `challenge`，其中包含回显的 `client_nonce` 和新的 `server_nonce`。
3. 插件在挑战有效期内发送签名的 `hello`，同时带回两个 nonce、能力列表和运行代次。该签名使用插件配置的 `token` 验证。
4. 后端校验后创建会话，派生客户端到服务端与服务端到客户端两个方向的会话密钥，并返回签名的 `hello_ack` 与 `session_id`。
5. 后续所有业务帧都使用对应方向的会话密钥进行 HMAC-SHA-256 签名，并验证 `session_id`、`seq` 和 `sent_at`。

`token` 仅保存在插件配置和后端受控环境变量中，从不写入 WebSocket 帧。断线重连会丢弃旧会话及旧出站帧，新的连接必须重新完成上述握手。

支持的消息类型：

| 方向 | 类型 | 作用 |
| --- | --- | --- |
| 插件 -> 后端 | `hello_init` | 发起挑战握手，携带客户端 nonce |
| 后端 -> 插件 | `challenge` | 返回一次性服务端 nonce 与过期时间 |
| 插件 -> 后端 | `hello` | 使用 token 签名的鉴权与能力声明 |
| 后端 -> 插件 | `hello_ack` | 使用会话密钥签名，建立 `session_id` 并触发 `presence_sync` |
| 插件 -> 后端 | `presence_sync` | 完整在线名单的实时基础字段 |
| 插件 -> 后端 | `player_delta` | 加入/退出增量 |
| 后端 -> 插件 | `snapshot_request` | 请求一个在线玩家的快照 |
| 插件 -> 后端 | `snapshot_response` | 返回快照或 `player_unavailable` |
| 后端 -> 插件 | `papi_request` | 请求白名单 PAPI 字段 |
| 插件 -> 后端 | `papi_response` | 返回字段结果、拒绝或不可用状态 |
| 插件 -> 后端 | `heartbeat` | 连接存活和在线人数 |
| 双向 | `error` | 协议或请求错误 |
| 插件 -> 后端 | `bye` | 插件停用通知 |

### 快照与物品数据

`snapshot_request` 的 `payload_json` 解码后为：

```json
{
  "player_uuid": "b54cf3f8-9f51-4c4c-a3d1-9584fce2822c",
 "sections": ["basic", "inventory", "ender_chest"]
}
```

`sections` 必须为非空数组，最多包含 3 项，允许的值为 `basic`、`inventory` 和 `ender_chest`。请求只适用于在线玩家；离线玩家应由后端的 `playerdata` Provider 兜底。

`snapshot_response` 的 `payload_json` 解码对象中的 `snapshot` 是玩家快照，包含 `uuid`、`name`、`online`、`observed_at`、等级、维度和 `position`。背包使用 `inventory.slots`，固定包含槽位 `0..35`、装备槽位 `100..103` 与副手 `-106`；末影箱使用 `ender_chest.slots` 的 `0..26`。

每个物品只包含 `id`（如 `minecraft:diamond_sword`）、`count`、可选纯文本 `name`、`lore` 和 `container`。潜影盒和收纳袋的 `container` 使用 `{kind, size, slots}` 递归返回有限槽位预览，受到配置中的文本长度、Lore 行数、递归深度和预览数量限制。不会传输原始 NBT、`ItemStack#serialize()` 输出或插件私有数据。

编码后的完整快照若超过桥接帧上限，插件会返回 `snapshot_too_large`，不会让该请求断开整个桥接连接。

## Folia 调度约束

- `Bukkit.getGlobalRegionScheduler()`：握手、心跳、在线名单协调和全局生命周期任务；`Bukkit.getOnlinePlayers()` 与 `Bukkit.getPlayer(UUID)` 仅在此上下文调用。
- `Player#getScheduler()`：等级、坐标、背包、末影箱、PAPI 与所有玩家对象读取。
- 玩家在任务执行前退休时，快照/PAPI 请求返回 `unavailable` 和 `player_unavailable`；不读取已退休实体。
- WebSocket、JSON、重连和有界队列只在独立 I/O 线程运行，网络阻塞不会占用区域线程。

PlaceholderAPI 扩展本身也必须兼容 Folia。桥接插件只能保证在玩家实体调度器上调用 `PlaceholderAPI.setPlaceholders`，不能修复第三方扩展中的跨线程访问。

只有 PlaceholderAPI 在启动时实际可用，`hello` 才会声明 `papi_read` 能力。断线重连会清空旧出站帧，并在新连接的 `hello_ack` 前拒绝所有普通业务帧。

## 构建与验证

Windows：

```powershell
.\gradlew.bat build
```

其他系统：

```bash
./gradlew build
```

已包含的纯 Java 协议测试覆盖严格 v2 信封、挑战握手、会话 HMAC、`payload_json` 编解码、PAPI 字段对象、`request_id: null` 和 UTF-8 安全截断。

当前尚未在真实 Paper/Folia 服务端完成联调。构建和协议测试不构成真实服务器兼容性结论；上线前仍需分别在目标 Paper 与 Folia 服务器中验证插件启停、断线重连、玩家跨区移动、退出退休回调、潜影盒/收纳袋预览和所有启用的 PAPI 扩展。
