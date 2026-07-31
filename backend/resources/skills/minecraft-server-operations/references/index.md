# 知识库索引

本 Skill 选取 `D:/Knowledge Base/` 中与开服器日常决策直接相关的内容，按主题压缩为五份参考资料。原知识库仍是编辑源；本目录是随开服器编译的稳定快照，修改知识库后需要重新筛选、更新版本号并运行测试。

| 内置参考 | 选取的原文件 | 主要内容 |
|---|---|---|
| `foundations.md` | `00-cores.md`、`02-jvm.md`、`04-security.md`、`06-network-proxy.md`、`15-linux-deploy.md` | 核心/加载器决策、Java/JVM、群组代理、Linux 部署、安全基线 |
| `plugins-and-content.md` | `01-plugins.md`、`07-mod-servers.md`、`08-database-config.md`、`18-plugin-automation.md`、`19-plugin-config-debug.md`、`20-plugin-scenarios.md` | 插件/模组选型、数据库、自动下载审核、配置与场景组合 |
| `worlds-and-modes.md` | `05-world-management.md`、`09-server-modes.md`、`11-geyser-bedrock.md`、`16-magic-world.md`、`17-tech-survival.md` | 世界管理、玩法模式、Java/基岩互通、数据包/资源包、技术生存 |
| `performance.md` | `03-troubleshooting.md`、`10-paper-config.md`、`12-performance-tools.md` | 启动与网络排错、Paper 配置、TPS/实体/区块/插件诊断 |
| `recovery-and-migration.md` | `04-security.md`、`13-backup-strategy.md`、`14-update-migration.md` | 数据保护、3-2-1 备份、恢复、升级与回滚 |

## 使用边界

- 文档中的插件名、核心分支、端口和参数是知识快照，不等于当前发布状态；执行下载或升级前再检查资源库和项目发布信息。
- 文档中的玩家承载量和性能数字只能用作对比样例，不能作为 SLA 或硬件承诺。
- 任何会覆盖世界、数据库、配置或插件 JAR 的操作，都必须先备份并保留回滚点。
