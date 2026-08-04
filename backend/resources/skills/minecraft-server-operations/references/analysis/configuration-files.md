# 配置语义分析

本 Skill 只识别配置格式、字段语义、影响范围和应用条件，不编辑或应用配置。

## 格式

- Java Properties：关注转义、续行、重复键、编码和 `server-ip/server-port/online-mode`。
- YAML：关注缩进、注释、锚点、重复键及 Bukkit/Paper/Purpur/Bungee 版本差异。
- TOML：关注 Velocity、Forge/NeoForge 配置的表、数组表和字符串语义。
- JSON：要求严格解析；不要把 JSON5/HOCON/自定义 conf 当标准 JSON。
- 启动定义：分别分析 BAT/CMD、PowerShell、POSIX shell、systemd 和 Compose 的引用及变量规则。
- NBT、MCA、数据库、JAR：只识别和评估，不能建议文本修改。

## 决策输出

对每项建议列出所属组件、原值证据、建议值、配置版本、预期影响、是否要求完整重启、原文件 SHA-256、关联文件、风险、回滚要求和验证步骤。未知 schema 保留未知字段，不把猜测写成确定补丁。

网络、认证、世界生成、Java/JVM、加载器、插件/模组列表通常要求外部执行角色在停机窗口处理。全局 `/reload` 和热插拔不是默认建议。
