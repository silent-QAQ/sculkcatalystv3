# UI、模型与动作引擎适配

本参考用于识别能力、分析绑定和设计插件中立表现方案。不要修改任何服务端/客户端资源，不要调用第三方 API、命令或脚本，不要把文档示例当作已验证的服务器能力。

## 核心边界

模型、动画、UI 和特效是表现层；伤害、治疗、扣款、奖励、任务推进、权限、传送与持久化由服务端业务层权威判定。正确顺序为：

```text
玩家输入 -> 服务端身份/权限/状态/冷却验证 -> 业务结果 -> 表现事件
```

客户端点击、动画事件、UI 回调、资源存在或模型碰撞都不能证明业务动作成功。任何回调都应被当作不可信输入，并在服务端重新关联玩家、会话、实体、动作 nonce、有效期和允许状态。

## 版本取证

只有在服务器目录证明确实安装了某引擎且需求涉及它时，才加载对应文档。依次检查：

1. 实际 JAR 名称、manifest、plugin metadata、依赖声明与 API 包。
2. 本地配置、示例、资源包/客户端模组标识和其他插件的绑定证据。
3. 精确 major/minor 版本匹配的官方文档。
4. 旧 Wiki 或社区资料，仅用于提出待验证假设。

记录插件版本、Minecraft 版本、服务端平台、客户端前置、资源协议、文档 URL 与读取日期。无法匹配版本时输出 `blocking_unknown`，不得推测方法签名、参数顺序、配置键或跨引擎兼容性。

## 独立 Adapter 契约

每个引擎使用独立 adapter；相同概念名不代表格式、生命周期或线程模型兼容：

```text
EngineEvidence inspectEvidence()
CapabilitySet describeCapabilities()
ModelResolution resolveModel(logicalRef)
AnimationResolution resolveAnimation(modelRef, logicalAction)
BindingAssessment assessBinding(entityRef, modelRef, actions)
PresentationPlan designPresentation(businessEvent, fallback)
UiPlan designUi(sessionContract, fallback)
RiskReport assessRisks()
```

这些是本 Skill 的中立分析接口，不是任何第三方 Java API。输出只能包含逻辑引用、能力要求、证据、未知项和编译前置条件。

Adapter 必须分别维护：

| Adapter | 主要角色 | 不可推断事项 |
|---|---|---|
| DragonCore | 1.12.2 常见客户端能力、UI/模型/动作集成 | 不从旧方法集推断新版本参数 |
| GermEngine | 客户端 UI、模型、动画与交互生态 | 不与 DragonCore 格式或事件互换 |
| MEG / ModelEngine | 服务端实体模型与动画表现 | 必须先确认 MEG 的准确产品、artifact 与 major 版本 |
| BetterModel | 独立模型与动画实现 | 不假设 ModelEngine 资产可直接复用 |
| PaiUI | 现代 UI 与交互表现 | 不把 UI 层视为模型、技能或业务判定层 |
| ArcartX | 模型、动作或平台提供的表现能力 | v2 文档不能反推其他 major 版本 |

若同时安装多个引擎，先查清每个实体/UI/资产的唯一所有者、桥接插件和加载顺序。不要设计双重接管同一实体或同一输入事件。

## 中立表现 IR

使用 `minecraft-presentation/v1`：

```json
{
  "schema": "minecraft-presentation/v1",
  "engine": {"adapter": "...", "observed_version": "...", "document_version": "..."},
  "subject": {"logical_id": "...", "type": "entity|npc|player|ui"},
  "business_event": "validated.skill.cast",
  "bindings": [{"model": "...", "animation": "...", "duration_ms": null}],
  "session": {"nonce_required": true, "expires_ms": null},
  "fallback": {"mode": "vanilla|static_text|no_effect"},
  "capability_requirements": [],
  "evidence": [],
  "unknowns": [],
  "approval": {"level": "L2|L3|L4", "plan_hash": "..."}
}
```

动画长度只能帮助表现对齐，不能用作服务端伤害计时的唯一时钟。服务端战斗时间线应独立确定；表现延迟、丢包或客户端缺少资源时，业务结果仍必须一致，并有清晰回退。

## 安全与兼容检查

- 模型、动画、UI、字体、纹理和音频引用是否存在且大小写一致。
- 服务端插件、客户端模组/资源、协议与 Minecraft 版本是否匹配。
- 同一输入是否被多个引擎监听并重复提交。
- UI 会话是否绑定玩家、实例、nonce、有效期与当前服务端状态。
- 动画事件是否错误承担命中、扣款、授权或奖励职责。
- 实体卸载、死亡、切世界、断线、切服和资源加载失败是否有清理/回退。
- 模型骨骼、动画更新、可见玩家扇出、展示实体和数据包是否有预算。
- 玩家可控字符串是否进入文件路径、脚本、命令、资源 ID 或反射/API 方法选择。

## 文档入口索引

文档入口只用于按证据选择资料，不表示所有页面适用于当前版本：

- DragonCore 方法集（自行核对参数与版本）：https://bukkit.wiki/plugins/plugins/dragoncore/Functions.html
- DragonCore 新 Wiki（未完工）：https://arisa.gitbook.io/internal-wiki/
- DragonCore 老 Wiki：https://core.anxidc.com/
- GermEngine 安装/教程入口：http://docs.germmc.com/docs/tutorial/germmod-install
- PaiUI Wiki：http://pai.rulmiao.cn/wiki
- PaiUI 服务端 API 入口：http://pai.rulmiao.cn/wiki#article=server-api-overview
- ArcartX v2：https://wiki.arcartx.com/docs/arcartx_v2

MEG/ModelEngine 与 BetterModel 未在本参考中固定文档 URL。先从实际 artifact 确认产品与 major 版本，再选择官方文档，防止同名、分支或历史版本混淆。

## 审批与 Handoff

纯表现草案通常为 L1/L2；影响交互流程、战斗时间线或大量玩家客户端负载为 L3；可触发资产、权限、强控制、批量迁移或允许 AI 选择业务结果为 L4。

输出 adapter、版本证据、能力矩阵、绑定 IR、服务端业务事件契约、性能预算、回退、未知项、审批范围和 `plan_hash`。专属插件 Skill 负责按已确认版本编译；外部执行角色负责部署和验证。本 Skill 始终只读，handoff 必须保持 `may_execute=false`。
