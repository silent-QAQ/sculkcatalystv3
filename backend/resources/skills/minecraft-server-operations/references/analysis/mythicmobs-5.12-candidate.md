# MythicMobs 5.12.0 候选知识索引

本页索引用户提供的第三方总结 `assets/source-material/newmythicmobs-5.12.0.lisk`。原始材料 SHA-256 为 `947006b6d38b100bca9eca83915fd0e049323fa91d57058684b1c1060fadc932`。

该材料自称来自 MythicMobs 5.12.0 反编译信息和官方示例，但没有附带可逐项核验的源码版本、构建号、官方页面或测试结果。因此它是 `candidate`，不是权威 API 文档。不得仅凭此材料宣称参数可部署。

## 使用条件

1. 从实际 JAR descriptor、启动日志或只读版本证据证明确切版本为 MythicMobs 5.12.0。
2. 记录 MythicMobs edition、Minecraft/Paper 版本，以及 MythicCrucible、ModelEngine 等扩展是否存在。
3. 只检索当前设计需要的章节，不把整份材料加载为全局规则。
4. 用实际 JAR、本地示例、匹配版本官方文档或测试服至少一种证据复核具体名称、别名、参数类型、默认值和 edition 限制。
5. 若证据冲突，以实际安装版本的运行证据为准，并把候选条目标记为 `rejected` 或 `version-mismatch`。

## 内容地图

- `SECTION 0`：YAML、ID、参数分隔等全局规则。
- `SECTION 1`：Items 模板、属性、Hide、Options 和 MythicCrucible 声明。
- `SECTION 2`：Mobs、Options、AI Goal/Target Selector、Equipment、Drops、Damage/Level Modifier。
- `SECTION 3`：Skills、Triggers、Targeters 和 Meta Targeters。
- `SECTION 4`：Mechanics 参数与示例。
- `SECTION 5`：位置、实体、物品、世界、天气、伤害、变量和视线 Conditions。
- `SECTION 6`：Boss、召唤、范围伤害和阶段切换示例。
- `SECTION 7`：材料名、格式、版本和 MythicCrucible 常见错误。

使用 `rg -n "SECTION|mechanic-name|targeter-name|condition-name" assets/source-material/newmythicmobs-5.12.0.lisk` 定位所需条目。

## 可吸收的设计知识

- 将 Items、Mobs、Skills 和 DropTables 建立独立命名空间与引用图。
- 对 Mob Skills 区分触发器、概率、条件、目标选择器和 Mechanic。
- 对 Projectile、Missile、Orbital、Totem、Beam、Chain 等持续或多跳能力计算每次施放的触发上界。
- 区分视觉 Mechanics 与造成伤害、破坏方块、传送、召唤、发物品或执行命令的权威 Mechanics。
- 将 AI Goal Selector 与 Target Selector 分开分析，检查清空默认 AI、优先级、攻击对象和寻路预算。
- 将 MythicCrucible、LibsDisguises、WorldGuard、经济与权限等能力作为显式依赖，不因语法出现就假设可用。

## 风险分类

- `L1/L2`：纯粒子、音效、标题、无玩法效果的动画表现草案。
- `L3`：伤害、治疗、控制、属性、普通召唤、普通掉落和 Boss 时间线。
- `L4`：`command`、OP/玩家身份命令、发放高价值物品、真实爆炸/方块修改、跨世界传送、无限召唤、全服目标、经济/权限联动和 Boss AI 接管。

即使材料给出完整示例，也只能生成中立 IR 和候选配置设计。不得执行 MythicMobs 命令、reload、生成实体或写入配置。

## 必须复核的高风险断言

- “物品 Skills 全部需要 MythicCrucible”及各物品 Trigger 的 edition/版本边界。
- `~onTimer`、`~onSignal` 和其他 Trigger 的确切语法及参数语义。
- AIGoalSelectors、AITargetSelectors、Targeters 和 Mechanics 的大小写、别名与支持范围。
- `damage`、`projectile`、`explosion`、`command`、`blockmask`、`teleport` 等默认值与安全效果。
- 1.20.5+ 输入 Trigger、Paper 特性和玩家类型 MythicMob 的要求。
- Materials、Attributes、Potion Effects 和 Bukkit 枚举在目标 Minecraft 版本中的名称。

验证不完整时输出 `blocking_unknown`，不要通过近似别名或历史版本经验补齐。
