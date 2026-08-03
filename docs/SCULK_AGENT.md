# Sculk Agent

Sculk Agent 是安装在开发机或 Minecraft 服务器主机上的独立轻量程序。它主动连接 Sculk Cloud，因此主机只需要能够访问互联网，不需要公网 IP、端口映射，也不需要把本地控制台、SSH 或 RCON 暴露到公网。

Agent 提供结构化文件/日志操作与完整 Shell。Cloud 控制台可以创建任务、人工批准高风险操作、查看执行事件与产物元数据，并对支持的结构化写入创建回滚任务。Shell 命令内容不过滤，权限与启动 Agent 的操作系统账户完全相同，可调用该账户环境中的 Git、Java、构建工具以及 Codex、Claude 等 CLI。

## 下载

- [Windows x86_64](https://sculk.mcmy.love/downloads/sculk-agent-windows-x86_64.exe?v=20260731-running-cancel-v1)
- [Linux x86_64 静态链接版](https://sculk.mcmy.love/downloads/sculk-agent-linux-x86_64?v=20260731-running-cancel-v1)
- [SHA-256 校验值](https://sculk.mcmy.love/downloads/sculk-agent-SHA256SUMS.txt?v=20260731-running-cancel-v1)

Linux 版使用 musl 静态链接，不依赖目标主机安装特定版本的 glibc。

## 配对

1. 登录 Sculk Cloud，进入“主机代理”，点击“连接新主机”。
2. 在 Minecraft 主机下载对应平台的 Agent。
3. 使用页面生成的短时配对码执行命令：

Windows PowerShell（下载版文件名）：

```powershell
.\sculk-agent-windows-x86_64.exe pair --cloud "https://sculk.mcmy.love" --code "scp_..." --name "mc-host" --workspace "minecraft" --workspace-root "D:\minecraft" --permissions "full" --capabilities "heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1,mcp-v1"
```

Linux：

```bash
chmod +x ./sculk-agent-linux-x86_64
./sculk-agent-linux-x86_64 pair --cloud "https://sculk.mcmy.love" --code "scp_..." --name "mc-host" --workspace "minecraft" --workspace-root "/srv/minecraft" --permissions "full" --capabilities "heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1,mcp-v1"
```

4. 对照 Agent 终端和 Cloud 控制台显示的指纹。
5. 指纹一致后，在 Cloud 控制台确认该主机。

配对码 10 分钟内有效且只能领取一次。Cloud 数据库仅保存配对码与 Agent 凭据的 SHA-256 哈希；`sca_` 凭据只在首次领取时返回给 Agent，不会打印到终端。

使用一键启动包前，Cloud 服务端必须设置 `SCULK_CLOUD_PUBLIC_URL` 为主机可访问的 HTTPS 公网根地址。该地址会写入 bootstrap JSON；生产环境不得填写内网、回环或带有账号密码、查询参数的 URL。`POST /api/cloud/agent-bootstrap` 返回的 JSON 只含短期一次性配对码和经当前账号授权的 Agent 配置，不含账号密码、会话或 Agent 凭据。

## 运行

### 一键启动包

在 Cloud 控制台的“主机代理”中填写主机名、工作区名称和对应平台的工作区根目录，然后点击 Windows 或 Linux 的“生成并下载”。下载内容只包含短期一次性配对码，不包含账号密码、Cloud 会话或长期 Agent 凭据。

- Windows：将 `.exe` 与同名 `.json` 放在同一目录，双击 `.exe` 即可自动领取配对凭据并启动。
- Linux：将 `sculk-agent-linux-x86_64` 与同名 `.json` 放在同一目录，执行页面提供的 `chmod +x` 和 `./sculk-agent-linux-x86_64 run --config ...` 命令。

首次启动后，Agent 会把一次性 JSON 原子替换为长期配置并清除配对码；随后仍需在 Cloud 控制台核对指纹并确认主机。

Windows（下载版）：

```powershell
.\sculk-agent-windows-x86_64.exe run
```

Linux：

```bash
./sculk-agent-linux-x86_64 run
```

Agent 每 30 秒发送一次 HTTPS 心跳，并轮询已批准任务，不监听任何入站端口。网络暂时不可用时会继续重试；凭据被撤销后会停止运行并提示重新配对。

默认配置位置：

- Windows：`%APPDATA%\SculkCatalyst\agent.json`
- Linux：`$XDG_CONFIG_HOME/sculk-catalyst/agent.json`，未设置时使用 `~/.config/sculk-catalyst/agent.json`

Linux 配置文件创建时权限为 `0600`。不要分享或提交 `agent.json`；如怀疑泄露，应立即在 Cloud 控制台撤销该主机并重新配对。

## 完整 Shell 权限

推荐配对参数为 `--permissions full`。它表示：

- Shell 命令不受 `workspace-root` 限制；任务可指定任意可访问的绝对工作目录。
- 命令继承 Agent 进程的 PATH、代理与其他环境，可调用已安装的 CLI。
- Agent 不会提升权限；以管理员或 root 运行时，远程 Shell 同样拥有管理员或 root 权限。
- `workspace-root` 是结构化文件操作和 Shell 默认工作目录，不是完整 Shell 的沙箱。

低风险的只读任务可以直接排队；`shell.exec` 是 critical 操作，必须选择一个 Agent 所属团队，并由该团队的 owner、admin 或 approver 中除请求人之外的成员批准。持久终端启动也遵循同一团队审批规则。审批决定与任务/终端建立数据库关联，重试和回滚会创建新的审批，不能复用旧决定；批准只是防误操作确认，不会限制 Shell 命令内容。若不希望 Cloud 任务拥有整机管理员权限，请用专用非管理员账户运行 Agent；这是操作系统账户边界，不是 Agent 功能裁剪。

## 任务执行

Cloud 控制台的“远程任务”支持：

- 主机信息、工作区列表与日志末尾读取。
- `log.tail` 只允许读取 `logs/` 和 `crash-reports/` 下的日志文件，并拒绝环境变量、数据库和凭据类文件名；返回内容会对常见密钥、Token、Bearer 值做脱敏。
- 创建目录和受控更新 `server.properties`。
- 完整 Shell 命令与最长 30 分钟超时。
- high/critical 任务会在 Cloud 控制台显示关联团队、审批 ID、请求人和决定人；请求人不能给自己发起的任务或终端审批。
- 持久终端创建后先处于等待审批状态；批准后才会向 Agent 投递 start 命令，拒绝或未批准的会话不能启动。
- 运行中的 Shell 停止请求，以及 Windows/Linux 子进程树终止确认。
- 实时 stdout/stderr 事件、最终输出摘要和产物元数据。
- 对支持的结构化写入创建人工批准的回滚任务。

Shell 不提供自动回滚。需要可回滚的部署时，应把备份、校验和恢复步骤写成结构化任务，或在 Shell 命令中显式完成事务与补偿。

### 检查点与重新执行

Agent 在操作完成后、向 Cloud 提交最终状态前保存结果检查点。如果此时 Agent 或网络中断，可以从检查点创建新的执行尝试：新尝试会恢复已经完成的结果，不会再次运行原命令或重复文件写入。

“从头重新执行”与“从检查点恢复”含义不同：

- 从检查点恢复只适用于存在可验证成功结果的任务。
- 从头重新执行会再次运行完整操作，原任务产生的副作用可能重复发生。
- Shell 和写入任务无论采用哪种方式，新的执行尝试都需要重新批准。
- 新尝试会创建新的团队审批记录；旧审批被拒绝、取消或已用于其他尝试时不会使新尝试自动获得执行资格。
- 每次尝试都有独立任务 ID、状态和事件，并保留在同一任务谱系中。

运行中的进程本身不能跨 Agent 进程重启迁移。检查点恢复的是任务已完成但尚未确认的结果；尚未完成的 Shell 命令只能从头重新执行。

### 停止运行中的 Shell

Cloud 控制台可向运行中的 Shell 任务发送停止请求。收到请求后，Agent 会终止该 Shell 的整个进程树，并向 Cloud 确认最终的“已取消”状态：Windows 使用 Job Object，Linux 使用独立进程组。停止不会撤销命令此前已经产生的文件、网络或外部系统副作用。

此功能需要本页当前版本的 Agent。旧版 Agent 不支持取消控制轮询；Cloud 不会把未获 Agent 确认的请求伪装成“已取消”，租约到期后会将任务标记为失败并提示最终结果未知。升级方式是停止旧进程、替换可执行文件，然后继续使用原配置启动。

升级 Cloud 数据库后，旧版本没有可验证团队审批关联的尚未启动高风险任务和终端会话会被取消；已经运行的旧会话保留到显式终止或租约回收，以避免升级过程留下失控进程。旧高风险任务若在升级时仍处于租约中，租约过期后会失败而不会重新排队等待不存在的审批。

## 从源码构建

Agent 是独立 Cargo 包，不会编译 PostgreSQL、Redis 或 Cloud 后端依赖：

```bash
cd agent
cargo build --release --locked
```

源码构建产物不带下载版的平台后缀：

```powershell
# Windows PowerShell
.\target\release\sculk-agent.exe run
```

```bash
# Linux / macOS shell
./target/release/sculk-agent run
```

Linux 静态版：

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl
```

Agent 源码采用 Apache License 2.0；Sculk Cloud 服务端和云端账号 UI 的许可范围见项目 `NOTICE`。
