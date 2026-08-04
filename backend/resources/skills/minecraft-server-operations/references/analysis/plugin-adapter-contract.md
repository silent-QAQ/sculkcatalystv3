# 插件适配器契约

适配器负责把只读证据转换成标准模型。主 Skill 不调用第三方写 API。

## 输入

- 服务器、Minecraft、Java 和插件准确版本。
- JAR descriptor、配置、当前周期日志和只读数据导出。
- 服主策略、世界/玩家范围、预算和审批状态。
- 当前任务所需的版本化文档证据。

## 输出

每个适配器输出：

- `descriptor`：插件身份、版本、能力和限制。
- `evidence`：来源、SHA-256、采集时间、作用域与新鲜度。
- `assessment`：事实、推论、假设、矛盾和未知项。
- `normalized_ir`：插件中立任务、怪物、权限、经济或表现设计。
- `compatibility`：目标版本与客户端要求。
- `proposal`：建议变更，不包含执行授权。
- `verification`：静态、测试服、灰度和生产观察方案。

## 规范化接口

概念接口包括：

```text
PluginDescriptor probe()
CapabilityReport capabilities()
ReferenceGraph inspectReferences()
ValidationResult validate(IR)
CompatibilityReport compare(IR, descriptor)
ExecutorHandoff buildHandoff(IR)
```

模型/UI 领域另使用 `resolveModel`、`resolveAnimation`、`PresentationPlan`、`UiPlan` 和 `InteractionPlan`。这些是本项目契约，不代表上游插件 API。

## 安全约束

- 解析脚本只解析或标记 Kether、JavaScript、命令和表达式，绝不求值。
- 玩家输入视为不可信，不拼接进命令、权限、SQL、配置或脚本。
- 未知版本只输出中立 IR，不声称可部署。
- 输出固定包含 `may_execute=false`、`handoff_is_authorization=false`、`execution_performed=false`、`writes_performed=false`。
- 外部执行角色必须重新探测版本、证据哈希、审批范围和数据新鲜度。
