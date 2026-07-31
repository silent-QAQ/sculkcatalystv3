# Sculk Catalyst 开服参数模板

`sculk-catalyst/server-template` 是与 Cloud 服务解耦的 Apache-2.0 便携配置格式。它可以由本机控制台、Sculk Cloud 或第三方工具生成和导入。

当前版本只描述创建服务器所需的可移植参数：

- 模板名称和说明
- 服务器名称
- 服务端核心
- Minecraft 版本
- 最大内存
- 首选端口

模板不包含 EULA 接受状态、本机或远程路径、服务器 ID、运行状态、日志、命令、脚本、环境变量、下载地址、API Key 或 Token。导入模板只会填充本机创建向导；用户仍需完成核心与版本兼容性、端口、Java、内存、磁盘检查，并再次确认 Minecraft EULA。

## 示例

```json
{
  "format": "sculk-catalyst/server-template",
  "manifest_version": 1,
  "template": {
    "title": "Paper 生存服",
    "description": "适合中小型纯生存服务器",
    "server": {
      "name": "深暗生存服",
      "core": "Paper",
      "minecraft_version": "1.21.4",
      "memory_gb": 8,
      "port": 25565
    }
  }
}
```

机器可读 Schema 位于 `/schemas/sculk-server-template-v1.schema.json`。导入器拒绝未知字段和超过 64 KiB 的文件；格式版本不兼容时不会尝试自动应用。
