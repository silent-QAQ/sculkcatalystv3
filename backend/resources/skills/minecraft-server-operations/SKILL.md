---
name: minecraft-server-operations
description: 面向 Minecraft Java 服务器的选型、开服、插件与模组配置、性能诊断、安全加固、世界管理、跨版本迁移、备份恢复和社区玩法规划。Use when the user asks to build, configure, operate, troubleshoot, secure, migrate, optimize, or recover a Minecraft server, including Paper, Purpur, Leaves, Folia, Fabric, Forge, NeoForge, Velocity, Geyser, plugins, mods, JVM, worlds, or server modes.
---

# Minecraft 服务器运维

把服务器问题拆成“核心/加载器、Java 与资源、内容组件、世界与网络、运行数据、验证与回滚”六层，再给出最小可执行方案。先由工作台读取当前服务器和部署机器事实；不要要求用户代替工作台汇报操作系统、CPU、内存、磁盘、Java、目录或端口。缺少用户偏好时采用可回滚的保守默认值并明确说明，不要把经验值当成兼容性结论。

## 使用规则

- 在规划模式或当前对话已经明确是 Minecraft 服务器时自动启用本 Skill，不要求用户输入 Skill 名称；沿用前几轮用户已经给出的核心、玩法、人数和版本约束。
- 规划时先从用户话语中提取已知条件，直接给出暂定架构和理由，只追问会改变核心选择或存在安全风险的少数问题；不要把所有可选项一次性变成四项以上的问卷。
- 面向不了解 Java、核心、插件和模组概念的服主：接受“和朋友玩生存”“想要更多装备”等自然语言。先解释最终体验，再在需要时补充技术名词，不要求用户先学习术语。
- 操作系统、架构、CPU、内存、磁盘、Java、安装路径、启动脚本和端口必须由工作台自动检测或分配，禁止向用户提问。Java 缺失时交给托管安装器；内存按机器容量和玩法给出保守初值；端口自动避让。
- 用户没有指定 Minecraft 版本时，从本地知识库和资源中心选择可验证的最新稳定版；资源中心没有答案才联网核实。玩家人数未给出时按约 12 人的小型服起步，之后依据运行指标调整，不为此阻塞创建。
- “确定、确认、就这样、开始吧、创建吧、按这个方案”等短确认表示提交最近的明确方案，不得继续追问部署系统、Java 或内存。
- “创建服务器”“继续帮我创建服务器”“使用 lsfk 创建”等明确执行表达视为开服指令：直接绑定最近一次已确认方案并进入真实任务执行器，不得再次转交模型索要文件执行器或让用户手工准备 Java/核心。`LSQFK`（常见简写 `lsfk`）必须作为独立核心名称处理；资源标识（如 `leavesslientqaqfork`）只作为下载/目录解析键，不能把它误判成 Leaves。
- “生电、技术生存、红石、更新抑制、刷沙、TNT 复制、原版机制”是高优先级信号。插件生电默认推荐 Leaves：它保留 Bukkit/Paper 插件生态，同时比 Paper 更重视原版机制还原；只有用户接受模组生态、追求纯机制还原时，才把 Fabric + Carpet 作为主方案。必须使用普通 Paper 插件服时，要明确说明机制损失风险。
- 先判断目标是插件服、模组服、代理群组、技术生存、RPG、小游戏、空岛、创造、PvP 还是 Java+基岩互通，再选核心和内容栈。
- 普通插件服以 Paper 作为默认起点；Purpur 用于玩法定制；插件生电优先 Leaves；Pufferfish/Leaf 只在性能优先且接受兼容风险时考虑；Folia 及其分支必须单独验证插件适配和玩家分布。
- Fabric、Forge、NeoForge 与 Bukkit/Paper 是不同生态。不要把插件、模组、客户端模组和数据包混为一谈，也不要承诺“混合端”无兼容风险。
- 现代版本优先检查 Java 21 与核心要求，老版本按实际核心和插件构建链选择 Java；内存按玩家、世界、插件/模组和负载测量，不用“分得越多越好”的规则。
- 修改 `server.properties`、Paper/Spigot 配置、插件配置或世界数据前先备份；高风险操作先停服或执行 `save-all`，保留可回滚副本。
- 下载插件、核心和模组时优先官方发布页、Modrinth、Hangar、GitHub Release 或官方 CI；确认文件是实际 JAR、版本和加载器匹配，并在安装后查看启动日志。
- 排错要先收集证据，再做一次变量修改并复测；优先使用 spark/Timings、日志、线程/内存和区块/实体数据，不靠猜测批量改配置。
- 任何“最新版本、当前兼容、具体下载地址、外部服务规则”的结论都要以当前资源库、项目发布页或用户提供的版本为准；本 Skill 中的经验只作为决策起点。
- Sculk Agent 的 ACP/CLI 会话可以直接在当前工作区读写文本文件；普通 IDE 编辑不需要创建任务执行器任务。只有安装核心、Java、插件、启动/停服、端口和世界等服务器级变更才进入任务执行器，以保留审批、日志、产物和回滚闭环。
- 不执行未经确认的删除、覆盖、回滚、开放公网端口、降级安全设置或全量世界迁移。涉及玩家数据时说明影响、备份点和恢复路径。

## 参考资料路由

只按任务需要读取参考资料，避免把整套知识库注入每次对话：

- 基础选型、JVM、Linux、代理、安全：读取 [references/foundations.md](references/foundations.md)。
- 插件、模组、配置、自动下载、插件方案：读取 [references/plugins-and-content.md](references/plugins-and-content.md)。
- 世界、数据包、资源包、Geyser、技术生存和玩法模式：读取 [references/worlds-and-modes.md](references/worlds-and-modes.md)。
- TPS、spark、Timings、Paper 配置和容量判断：读取 [references/performance.md](references/performance.md)。
- 数据库、备份、灾难恢复和版本迁移：读取 [references/recovery-and-migration.md](references/recovery-and-migration.md)。
- 需要知道参考资料来自知识库哪些原文件时，读取 [references/index.md](references/index.md)。

## 标准工作流

1. 盘点：读取服务器模板、运行状态、目录结构、日志和配置；记录核心、MC 版本、Java、端口、数据存储、插件/模组和目标人数。
2. 决策：根据玩法和兼容性选择核心、代理、内容组件及资源配置，列出已知风险和不确定项。
3. 变更：先备份，使用最小改动；下载或生成文件时校验来源、格式、版本和摘要，禁止把网页保存成 JAR。
4. 启动：先在测试目录或低风险窗口启动，检查 EULA、端口、Java、依赖、加载日志、世界和玩家连接。
5. 验证：用目标场景复测功能；性能问题用基线和报告验证；跨版本/跨端功能分别验证客户端、代理和后端。
6. 交付：说明改动文件、适用版本、依赖、启动方式、测试结果、备份位置、回滚方式和仍未验证的风险。

## 强制资源解析与下载顺序

资源获取必须执行，不得在尚未尝试回退链路时直接对用户说“不能”“无法”或要求用户手工准备文件。每一次尝试都要写入任务日志，记录来源、版本、URL、SHA-256、失败原因和下一回退层。

1. 先查本地资源库/资源中心的已发布版本与可信下载对象。核心、Java、插件都先复用已有文件和校验信息；命中后直接下载、校验并安装。远程资源中心核心查询使用 `GET /api/catalog/cores?search={identifier}&minecraft={version}&channel=stable`，再使用返回项目的真实 `slug` 查询 `/api/catalog/cores/{slug}/versions`；资源显示名、资源标识和分支 slug 必须分开保存（例如 LSQFK、`leavesslientqaqfork`、`lsqfk` 不是同一个字段）。资源中心没有精确匹配时才进入下一层，不能因 `/api/v1/resolve` 不可用就跳过资源中心。
2. 核心或 Java 在资源库没有精确匹配时，调用 MSL API V4 国内镜像：
   - 接口契约文档：`https://apidoc-v4.mslmc.cn/api-429587266`（同一文档中的 Java 接口为 `/api-429587275`、`/api-429587276`）；实际请求基址为 `https://api.mslmc.cn/v4`。
   - 核心支持列表：`https://api.mslmc.cn/v4/mirrors?view=list`；版本列表：`/mirrors/{server}` 或 `/mirrors/{server}/{minecraft}`；下载：`/download/server/{server}/{minecraft}?build=latest`。
   - Java 下载：`/download/jdk/{major}?os={windows|linux}&arch={x64|aarch64}`。
   - 只接受 HTTPS、真实文件 URL 和有效 SHA-256；下载后必须验证压缩包/JAR、版本和可执行内容。
3. 插件在资源库没有精确匹配时，先查 Modrinth 官方 API `https://api.modrinth.com/v2/search`，限制 `project_type=plugin`，按目标 Minecraft 版本和 Paper/Spigot/Bukkit 加载器筛选；随后读取版本文件的真实下载 URL。
4. Modrinth 没有匹配时，查 SpigotMC 官方资源搜索 `https://www.spigotmc.org/resources/?search=...`，记录项目页和可用下载入口，并验证下载内容是 JAR。
5. 上述官方源均无结果时，才使用 GitHub Release、项目官方 CI 或配置的互联网搜索 API；不得把搜索结果页面误当成 JAR 下载地址。

核心、Java、插件的执行器必须沿用这条顺序；模型只能说明正在使用哪一层，不能跳过下载、校验、安装、启动日志检查和失败回滚。所有平台都使用同一策略，Windows/Linux 只影响 MSL 的 `os`、`arch` 参数和压缩包解压方式。

## 输出要求

给出可执行步骤和命令时同时标明 Windows/Linux、执行目录和是否需要停服。涉及配置时只给必要字段，避免覆盖用户现有配置；涉及插件时给出依赖顺序和兼容矩阵；涉及事故时先保护数据，再诊断和恢复。

## 只读分析与外部执行交接

当请求包含 `$manage-minecraft-server`、日志/崩溃诊断、服务器审计、插件生态取证或需要评估现有服务器状态时，启用本 Skill 的只读分析模式。该模式融合自 `analysis-SKILL.md`，优先收集证据并输出事实、结论、假设、矛盾、阻塞未知项、风险、前置条件、回滚要求和验证标准。

- 只读分析模式不得修改、移动、删除、覆盖、备份或恢复服务器文件，不得启动、停止、重启、杀死、重载服务器或发送控制台/RCON 命令。
- 不执行 JAR、启动脚本或网络探测；只有用户明确要求实时端点验证时才进行协议探测。
- `scripts/` 下的分析工具只输出证据或交接建议；`execution_performed=false` 与 `writes_performed=false` 必须保持为假值，不得把建议当成授权。
- 需要实际变更时，先给出只读分析和结构化交接单，再由具备审批、日志、产物和回滚闭环的执行路径处理。
## 智能资料获取协议

服务器规划需要外部事实时，必须按以下顺序处理：先检查用户消息中的本机文件路径和 JAR 描述，再查内置资源中心 `https://res.mcmy.love`，资源中心没有结果才查公开互联网来源。每条外部结论都要保留来源和版本，不得因为搜索结果为空而猜测核心、Java、插件兼容性或下载地址。

本机文件存在时直接使用探测到的文件名、大小、SHA-256 摘要和 `plugin.yml` 信息，不要重复追问用户已经明确提供且本机可验证的路径。资源库和互联网都找不到时，只询问会改变方案或安全性的最小缺失信息；不要一次列出无关问卷。用户明确说“不知道”“不清楚”或要求去群里确认时，才调用已经配置并启用的 QQ/NapCat 适配器，向指定群发起协查；没有群号、适配器未启用或发送失败时，要如实告诉用户并回到最小问题。
