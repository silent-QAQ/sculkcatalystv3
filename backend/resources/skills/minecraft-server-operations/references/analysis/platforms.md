# 平台识别与运行语义

本 Skill 只分析平台、兼容性和运行约束，不运行平台命令、不修改定义、不控制实例。

## 识别证据

- Vanilla：版本日志、server JAR manifest/class 线索。
- Spigot/Paper/Purpur/Folia：Bukkit/Paper 元数据、配置布局和启动日志。
- Fabric：`fabric.mod.json`、Fabric Loader 日志及 Fabric API 依赖。
- Forge/NeoForge：`mods.toml`、ModLauncher 日志、版本与 Java 要求。
- Velocity/BungeeCord：代理配置、插件元数据和监听日志。

目录名或用户描述只能作为线索，结论需要 JAR、日志、配置或进程交叉证明。

## 运行约束

- Bukkit/Paper 主线程、Folia region/entity/global scheduler 和代理事件循环具有不同线程约束。
- Minecraft、加载器和 Java 兼容矩阵随版本变化；使用官方要求、当前成功日志和制品元数据交叉判断。
- systemd、Compose、Windows Service、面板和脚本是不同 ownership 域，不能混合推断。
- 容器持久化取决于 bind mount 或 volume；容器可写层不是可靠持久状态。

handoff 应写明平台置信度、证据、替代解释、线程/生命周期约束和外部执行角色必须重新验证的条件。
