# 生命周期证据与决策

本 Skill 不启动、停止、重启、reload 或终止进程，只判断外部执行角色是否拥有足够证据。

## Supervisor 证据

- 脚本、同名 unit、Compose 文件或服务显示名只证明声明存在。
- systemd ownership 需要 unit、MainPID、cgroup、Java cwd、WorkingDirectory 和唯一实例闭环。
- Compose ownership 需要唯一容器、service/config/working-dir 标签和精确 bind mount。
- Windows Service ownership 需要双 CIM 快照、稳定 PID/CreationDate/ParentPID/ExecutablePath 链和唯一绝对 JAR。
- 多候选、多实例、字段缺失或采样漂移标为冲突或未知。

`execution_handoff` 只供外部角色审查，不表示授权、可安全执行或已经执行。

## 状态语义

- `stopped-or-undetected`：证据不足，不能断言确定停机。
- `starting-or-running`：存在相关进程，readiness 未确认。
- `ready`：当前周期、ownership 与可选协议证据一致。
- `failed`：当前周期存在 fatal 或明确启动失败。
- `unknown-listener`：端口响应但所有权未知。

## 当前周期

调用方可保存 `capture-baseline.py` 的 stdout，再提供给诊断/验证脚本；Skill 自身不落盘。有效 baseline 支持同 inode 追加或一次可证明轮转。截断、旧 inode 缺失、多个候选、链接、不稳定读取或超预算都使证据不完整，不得回退旧 `Done`。

## 外部执行前置建议

- 重新确认 serverRoot、supervisor、进程身份、端口和磁盘。
- 验证 Java/服务端兼容性、故障原因、备份及回滚能力。
- 确认没有并发运维或第二实例。
- 身份或证据漂移时 abort 并重新分析。

## 外部执行后验证建议

- supervisor 仍为同一目标，运行身份属于新实例。
- 当前周期出现 readiness，之后没有 fatal。
- 可选 Minecraft Ping 合法，但不能替代 ownership。
- 任务相关组件或业务功能 smoke test 通过。

报告必须区分建议、外部角色报告和本 Skill 的只读验证；不得声称本 Skill 完成生命周期操作。
