# A-4 真实验收：Codex 跑 ops-demo（2026-11-14）

> 真实 Codex（`codex-cli 0.169.0`，默认模型 `gpt-6.1-sol`）+ 真实 harness（`ip serve`），按[验收手册](m2-acceptance.md)第 5 节。
> 环境：Claude 云端开发容器，没有 bubblewrap，`sandbox.mode: none`。Codex 的 key 就是环境里存好的 OpenAI key，由网络代理在请求 `api.openai.com` 时带上。
> 没有 Slack 工作区，所以不经 Slack，改从控制台的 `/ws` 发消息；Slack 那一半留给 A-3。

## 怎么跑的

- **配置**：照手册在 `ip.yaml` 里加 `engines.ops-tester`（`kind: codex`），`ops-1` 的 `workspace` 指向 `examples/ops-demo` 的一份副本。
- **代理**：这个容器只能经代理出网，而 harness 启动 Codex 时会清空环境变量。所以这里 `command` 指向一个两行的包装脚本，补上 `HTTPS_PROXY` 和证书路径再执行 `codex`。能直接出网的主机不需要这一步。
- **A 段**（照手册）：发“ops-demo 的测试报错了，帮忙修一下”，不指定员工，让路由分派；干完后在同一对话里追问“改了哪一行？”。
- **B 段**（手册里“任务交回”那一项）：把 bug 还原，用 `create_task` 正式派一个任务给 `ops-1`，看它能不能经 MCP 的 `update_task` 交回。

## 结果

| 步骤 | 结果 |
| --- | --- |
| `ip check` | ✓ `岗位 ops-tester 的 Codex 引擎`：`codex-cli 0.169.0` 可以运行 |
| 路由 | ✓ 命中规则“线上故障”（含“报错”），交给 `ops-1` |
| 修 bug | ✓ `codex.shell` 跑 `sh test.sh` 失败（2 例）→ 定位到 `growth.sh` 第 6 行分母用错 → `codex.edit` 改一行 → 重跑全部通过 → 写 `reports/2026-11-15-growth.md` |
| 追问“改了哪一行？” | ✓ 续接原来的 Codex 会话，1 步、25 秒答出第 6 行，附了 diff，没有重新读文件 |
| 任务交回（B 段） | ✓ 开工时经 MCP 调 `update_task` 改成“进行中”，干完再调一次标成“已完成”，带上结论和报告路径 |

报告里的日期是 2026-11-15：E-9 之后每条消息都带了发送时间，示例配置的 `utc_offset` 是北京时间，Codex 照它写了日期。

## 验收中发现并修掉的问题

Codex 引擎的命令行参数和输出解析原来是按资料写的，这次第一次在真实 Codex 上跑。改动都在 `rust/crates/ip-engines/src/codex.rs`：

1. **MCP 工具调用被 Codex 自己拒掉。** 报错是 `MCP tool call requires approval, but approval policy is never`：Codex 0.169 默认要人批准 MCP 工具调用，而 harness 传的是 `approval_policy="never"`，所以直接拒，任务交不回去。现在给组织工具的 MCP 服务器加上 `default_tools_approval_mode="approve"`。合法取值是 `auto`、`prompt`、`writes`、`approve`；组织工具已经由本轮令牌限定范围，所以一律放行。
2. **活干完了，这一轮却被记成失败。** Codex 中途会发顶层 `error` 事件，比如这个容器里 WebSocket 被代理拦下时的 `Reconnecting... 5/5`，随后改走 HTTPS 照样跑完。harness 原来把任何 `error` 事件都当成整轮失败。现在只当警告记下；这一轮成没成，看 `turn.failed` 和 `turn.completed`，没有 `turn.completed` 时把最后一条警告附在失败原因里。顺带的问题：被记成失败的一轮不会保存 Codex 的会话编号，下一轮就开了新会话，续接测不出来。
3. **MCP 调用成功也被记成失败。** Codex 成功时也带 `"error": null`，harness 原来见到 `error` 字段就算出错，输出成了 `null`。现在空的 `error` 不算出错。
4. **缓存写入没有记账。** Codex 的用量里有 `cache_write_input_tokens`（#54 时我按资料写了“Codex 没有”，不对）。现在读进来，按 `cache_write` 价格计费。

## 当时没修的：续接会话时用量重复记账（backlog E-10）

> 2026-11-16 已修：引擎会话表存会话的累计用量，每轮只记差值（`docs/ip/m2-spec.md` D-23）。只用假 codex 测过，没有用真实 Codex 重跑。下面是当时的记录。

续接会话（`codex exec resume`）时，`turn.completed` 报的是整个会话的累计用量，不是这一轮的。用 `gpt-6-luna` 先跑一轮、再续接一轮核对过：

| | 输入 | 命中缓存 | 写入缓存 | 输出 |
| --- | ---: | ---: | ---: | ---: |
| 第一轮 | 12,492 | 0 | 12,489 | 5 |
| 续接的第二轮（报的数） | 25,006 | 12,489 | 12,511 | 23 |
| 第二轮减第一轮 | 12,514 | 12,489 | 22 | 18 |

harness 把每一轮报的数都加进账本，所以续接一次，前面几轮就被再算一次，费用偏高、预算提前用完。要修就得在引擎会话表里存上一轮的累计用量，每轮记差值，涉及 `ip-store` 的表结构，留给工程师定。

## 花费

示例里的 Codex 配置没写价格，账本记的是 0。按 `gpt-6.1-sol` 的官方价（输入 2、命中缓存 0.1、写入缓存 2.5、输出 10 美元/百万 token）手算：修 bug 的一轮约 0.06 美元；续接追问一轮按差值算约 0.003 美元。整个验收（含排查时直接跑 Codex 的几次）约 0.3 美元。示例配置的 Codex `price` 注释已换成这个价格。
