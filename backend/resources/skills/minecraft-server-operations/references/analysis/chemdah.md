# Chemdah 任务与对话分析

本 Skill 只读分析 Chemdah 内容，不加载插件类、不执行 Kether、不调用命令，也不修改任务或玩家档案。使用 `scripts/analyze-chemdah.py <serverRoot> --pretty` 扫描证据。

## 证据优先级

1. 实际 Chemdah JAR、插件启动日志和服务器内注册结果。
2. 与安装版本匹配的本地配置、示例和 API 元数据。
3. [Chemdah 官方文档](https://plugins.ptms.ink/plugin/chemdah) 中与版本相符的页面。
4. 无法核对版本的网页示例仅作为设计线索。

部分开发文档可能由 AI 辅助生成。没有版本与运行证据时，不据此断言方法签名、配置键或线程语义。

## 分析范围

- 扫描 `plugins/Chemdah/core/quest` 与 `core/conversation` 下的 YAML、JSON、TOML。
- 建立任务、对话、NPC、等级线、跳转及前置条件的引用图。
- 检查重复内容 ID、重复条目、无法解析的配置、未解析引用和证据截断。
- 标出 Objective 类型并要求与服务器实际注册表核验；自定义 Objective 不是当然错误。
- 标出 `then`、`then-async`、Kether、命令和奖励表面，只审查文本，不求值。
- 结合日志判断档案加载、数据库、跨服同步、退出保存和异步时序问题。

## 高风险判定

以下内容至少需要人工复核：控制台命令、权限变更、货币或点券变更、文件/数据库动作、玩家输入或占位符拼接。任务奖励还需检查取消重接、队伍共享、断线、跨服重放、每日重置和幂等键。

配置中出现动态文本不等于已确认注入漏洞。报告必须同时给出输入来源、转义或白名单证据、执行身份和可到达路径；证据不足时标记 `inconclusive`。

## 设计输出

任务建议使用插件中立 IR，包含受众、目标、阶段、条件、失败路径、奖励预算、反滥用约束和验证指标。只有 Chemdah 专项开发 Skill 在确认版本后才能把 IR 编译为候选配置；候选仍须人工或授权策略审核，并交给外部执行角色部署。

所有结论附文件路径、SHA-256、引用路径和置信度。最终报告声明 `analysis_only=true`、`writes_performed=false`、`kether_executed=false`。
