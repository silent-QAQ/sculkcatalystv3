# 插件生态分析

主 Skill 只识别已安装插件、配置、日志、数据能力和跨插件引用，输出设计与风险。不得加载插件、调用 Bukkit API、执行命令或写配置。

## 证据模型

每个插件生成 `PluginEvidence`：

- `identity`：descriptor 中的名称、版本、main、依赖和 JAR SHA-256。
- `runtime`：当前启动周期的 enable、disable、异常和依赖日志。
- `configuration`：配置目录、格式、schema 线索和引用，不展开秘密。
- `data_capabilities`：余额快照、交易流水、任务进度、权限图等可用性。
- `client_requirements`：客户端模组、资源包、协议和降级行为。
- `documentation`：来源、页面、读取日期、适用版本和置信度。

插件存在不等于能力可用。能力状态只能是 `supported`、`unsupported`、`unknown`、`version-mismatch` 或 `client-missing`。

## 能力注册表

使用能力而不是插件名驱动设计：

- `economy.balance`、`economy.ledger`、`market.orders`、`market.trades`
- `permission.resolve`、`permission.context`、`permission.audit`
- `quest.definition`、`quest.progress`、`conversation.graph`
- `mob.definition`、`skill.timeline`、`drop.expectation`
- `model.render`、`animation.play`、`ui.open`、`input.receive`
- `npc.identity`、`npc.dialogue`、`affinity.state`

同一能力有多个 Provider 时，必须识别实际注册者、作用域和数据权威来源，不得自动合并。

## 按需文档

1. 先从 JAR 与配置确认插件及准确版本。
2. 仅在当前请求需要该能力时读取对应文档。
3. 优先本地版本文档和 API 依赖，其次是匹配版本的官方文档。
4. 每个 API 结论记录来源与版本；不匹配时输出阻塞性未知项。
5. 不把第三方方法签名、完整配置键或固定兼容表长期硬编码在主 Skill。

## 跨插件规则

- Vault 是服务抽象，不是交易账本。
- PlaceholderAPI 是展示/读取桥梁，不是业务数据权威源。
- 客户端模型、动画和 UI 回调不能决定伤害、经济、掉落或权限。
- 任务奖励必须进入经济与物品供给评估。
- 权限建议必须包含 LuckPerms 之外的 OP、代理和插件私有白名单旁路。
- MythicMobs、任务、NPC、模型引擎之间使用稳定的插件中立 IR 连接。
