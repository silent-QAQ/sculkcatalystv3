# MythicMobs 与受限生物 AI 设计

当实际安装版本确认为 MythicMobs 5.12.0 时，可按需读取 `references/mythicmobs-5.12-candidate.md`，并从其索引定位用户提供的原始候选材料。候选材料不能替代实际 JAR、匹配版本官方文档或测试服验证。

本参考只用于分析现有内容、设计插件中立方案和生成供人工审查的 handoff。不要写入 MythicMobs 配置，不要调用技能、刷怪或重载命令，也不要把设计稿视为可部署产物。

## 证据门槛

设计前确认服务器实际安装的插件名称、JAR 版本、Minecraft 版本、平台、扩展插件和本地配置结构。优先级为：实际 JAR/API 元数据与本地示例、匹配版本的官方文档、匹配版本的可信资料、通用概念。无法确认版本或关键扩展时，将具体语法标记为 `blocking_unknown`，只输出中立 IR。

把事实分为：直接证据、由引用图支持的结论、需要运行指标验证的推测。不要仅凭文件名认定机制存在或可用。

## 插件中立 IR

使用 `minecraft-content-mob/v1` 表达意图，不复制任何特定版本的 MythicMobs YAML：

```json
{
  "schema": "minecraft-content-mob/v1",
  "identity": {"id": "namespace:mob", "role": "boss|elite|rare|ambient"},
  "audience": {"party_size": [1, 4], "progression_band": "..."},
  "spawn": {"scope": "...", "concurrency_cap": 1, "cooldown": "...", "despawn_policy": "..."},
  "stats_budget": {"health": null, "armor": null, "movement": null, "target_ttk_seconds": null},
  "phases": [],
  "skills": [],
  "behavior": {"states": [], "transitions": [], "allowed_actions": []},
  "presentation_bindings": [],
  "reward_budget": {"expected_value": null, "daily_supply_cap": null},
  "performance_budget": {},
  "safety": {"escape_state": "...", "hard_time_limit": "...", "forbidden_actions": []},
  "evidence": [],
  "unknowns": [],
  "approval": {"level": "L4", "plan_hash": "..."}
}
```

每个技能项至少包含触发条件、目标选择、冷却、资源成本、预警时间、有效范围、可反制方式、失败/取消路径、每次触发实体与粒子上限。伤害、控制、位移和掉落必须由服务端权威判定；模型、动画、音效和 UI 只能表现结果。

## 引用图与静态检查

建立 Mob、Skill、Item、DropTable、Spawner、Faction、Placeholder、模型与动画的有向引用图，并检查：

- 缺失、大小写漂移、跨包重名和不可达引用。
- 直接递归与间接递归；循环必须有可证明的深度、次数或时间上限。
- 零冷却/极短冷却触发链、伤害触发伤害、死亡触发召唤、召唤物再次召唤。
- 每 Tick 技能、无界范围选择器、全世界扫描和高频路径重算。
- 无上限召唤物、投射物、掉落物、光效、粒子、声音与元数据持有。
- 阶段跳转互相反弹、阶段不可达、无逃生状态和永久无敌。
- 清理路径缺失：主人死亡、区块卸载、玩家离场、超时或服务器异常后仍残留。
- 掉落表循环、期望价值异常、保底叠加和可重复结算。

递归检查不能只找文本自引用；应在解析别名、条件分支、元技能和扩展动作后计算强连通分量。无法证明循环有界时按阻塞风险处理。

## 性能预算

不要把“配置能加载”当成“性能可接受”。按单次触发、单实体、单区块和全服四层估算：

- 同时存活实体、召唤物、投射物与展示实体峰值。
- 每秒触发次数、目标候选数、射线/碰撞/路径计算次数。
- 粒子、声音、数据包和模型骨骼动画的玩家扇出。
- 同步数据库、脚本、Placeholder 与外部插件调用的最坏耗时。
- Boss 同场人数增加时的非线性放大。

输出预算值、推导依据、无法测量项、观测指标与停止条件。缺少 spark/timings、实体计数或真实并发证据时，只能给区间和验证方案，不得承诺 TPS 影响。

## 战斗与经济约束

用目标 TTK、队伍规模、有效 DPS、控制覆盖率和治疗能力校验数值。每阶段必须有可读预警、玩家反制窗口和失败恢复；避免靠不可见秒杀、永久控制或装备硬门槛制造难度。

掉落分析至少计算每次击杀期望值、单位时间供给、每日理论上限、保底成本和多开收益。将经济结论交给经济治理模块；MythicMobs 设计本身不得调整余额、市场或权限。

## 受限生物 AI

语言模型不得逐 Tick 接管实体。采用经审核的有限状态机或行为树：感知输入映射为有限事件，只能选择 `allowed_actions` 中已批准的技能、移动和表现动作。

必须具备：状态/转移白名单、参数范围、调用频率、并发上限、区域边界、硬超时、确定性回退、审计事件和紧急禁用条件。玩家聊天、书本、物品名或 Placeholder 内容不得成为脚本、命令、权限节点或任意技能标识。

稀有怪、Boss、跨区块寻路、资产奖励、强制位移和长时间控制属于 L4。默认逐案人工审核；只有服务器级完全信任与第二次、限定范围的 L4 完全信任同时有效时，外部审批角色才可自动审批。此 Skill 始终 `may_execute=false`。

## Handoff 要求

输出版本证据、引用图摘要、IR、预算、风险、未知项、审批级别、不可变 `plan_hash`、回滚要求和验证窗口。明确专属 MythicMobs Skill 仍需按实际版本编译与验证，外部执行角色必须重新检查证据新鲜度，且建议不构成授权。
