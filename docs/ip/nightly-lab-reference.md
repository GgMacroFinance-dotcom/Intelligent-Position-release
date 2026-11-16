# nightly-lab 参考清单（只参考设计，不复制代码）

nightly-labs/openbot 是 PolyForm Noncommercial 许可。Rust 重写只参考它公开的功能与架构设计，所有代码自行用 Rust 编写。
本仓库不包含 nightly-lab 的任何代码。下表只记它公开的 README 里描述的设计，以及 Rust 版对应做到了哪一步。

## 值得在 Rust 版里对应实现的设计点

| 设计点 | nightly-lab 的做法（来自其 README） | Rust 版状态 / 建议 |
| --- | --- | --- |
| 持久化多 agent | 每个 agent 独立工作区与会话 | 已有：每个岗位一个 tokio task |
| 上下文监控与自动压缩 | 监控用量，超限自动压缩 | 已有：context_budget_chars + Body::Summary |
| 消息队列 | FIFO，支持暂停/恢复/取消，崩溃后保留 | 待做：显式暂停/恢复，持久化队列 |
| 群聊频道与委派 | 单一任务负责人，Stop/Resume/Reassign/Archive/Restore | 部分：已有任务与委派工具，缺 Reassign/Archive |
| 多引擎适配 | 通过 ACP 接入多种外部 CLI | 待做：agent-client-protocol 作为可选引擎 |
| 本地模型服务检测 | 自动发现 Ollama、LM Studio | 待做：推理端点可配置，可加自动发现 |
| 每 agent 的模型/通知级别 | 每个 agent 单独配置 | 部分：角色已有模型槽，缺通知级别 |
| 自定义主题 | `themes/<id>.json` 调色板 | M1 控制台后续可做，优先级低 |
| 密码管理连接器 | 1Password、Bitwarden | 经 MCP 接入，优先级低 |
| 工作流画布 | Routine Flows / Workflows | 对应岗位的任务编排，M2 之后再看 |
| 无屏幕 Linux 部署 | Docker、Nix | 已有：`rust/Dockerfile` 和 `docker-compose.yml`，见[《用 Docker 部署》](deploy-docker.md)；Nix 暂未做 |

## 不参考的部分

账号体系（邮箱验证码、Cloudflare Workers/D1）、iOS 与 WebRTC 远程、Dynamic Island 等桌面端特性，与"云主机上的数字员工"方向无关。
