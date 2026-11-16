# 宏观研报岗位真实验收记录（2026-11-11）

> 真实模型（OpenAI `gpt-6-astra`、`gpt-6-luna`）+ 真实数据（Finance Macro Data API 的 Render 测试部署，0.2.0，12 个工具）。
> 环境：云主机，无 bubblewrap，`sandbox.mode: none`；配置即 `rust/ip.example.yaml`（只改了沙箱一项）。Slack、Codex 不在本次范围。

## 结果

| 步骤 | 结果 | 花费（美元） |
| --- | --- | --- |
| `curl` 两个接口 | ✓ `api.openai.com/v1/models` 200，列表里有 gpt-6-astra、gpt-6-luna；`whichapi-macro.onrender.com/v1/tools` 200 | 0 |
| `ip check` | ✓ 0 项失败；2 项提示：沙箱已关闭、`数据源 public` 未拉取 | 0 |
| 研究员第一次调用 | ✗ 400，原始报错见下；修复后通过 | 0 |
| 研究员单独回答“美国 10 年期国债收益率最近一周怎么走” | ✓ 调 `get_treasury_curve`、`get_fred_series`（DGS10）→ 写报告 → 回答，数字与工具结果一致，带观测日期 | 0.0023 |
| 协调员派活：“下周一投资例会要一页材料：美国最近一个月的通胀和利率怎么走，资金面紧不紧” | ✓ 协调员 `create_task` → 研究员取数写报告并 `update_task` 交回 → 协调员追加核验任务 → 发选择卡片（`ask_human`）→ 选推荐项后再派一次 → 汇总一页材料 | 当天合计约 0.57 |
| M0 其余项 | ✓ shell 工具跑 `uname -a`、`ls`（0.0008）；中文搜索“资金面”搜到历史消息；`ip bench startup/store/agents --check` 都是 `"pass":true`（启动中位 5.3 ms，搜索 P95 4.9 ms，500 员工空闲 32.8 KB/个）；沙箱断网没验（主机无 bubblewrap） | 0.0008 |
| 语义分派（`gpt-6-luna`，backlog E-3） | ✓ “下周一投资例会要一页材料：这周美国物价和工业品价格怎么走”→ 宏观研究员，“服务器的定时脚本昨晚没跑”→ 运维测试，把握都是 0.98 | 很少 |
| 国家统计局数据 | 跳过：接口拒绝境外 IP | — |

当天费用（`spend_today`）：协调员 0.503，宏观研究员 0.046，运维测试 0.025。协调员 7 轮共约 0.50，平均每轮 0.07；研究员一次完整取数写报告约 0.01–0.02。

报告原文在 [`acceptance-2026-11-11-reports/`](acceptance-2026-11-11-reports/)，最终交付的是《美国通胀利率资金面一页材料-可核实数据版》。抽查：DGS2、DGS10、RRPONTSYD 的数字与工具返回逐项一致；ON RRP 单位换算（0.252 十亿 = 2.52 亿美元）正确。

## 修复：OpenAI 推理模型必须走 Responses 接口

第一次调用的原始报错：

```
模型服务返回 400：{"error": {"message": "Function tools with reasoning_effort are not supported for gpt-6-luna in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.", "type": "invalid_request_error", "param": "reasoning_effort"}}
```

逐个试过：gpt-6-luna 设 `reasoning_effort: none` 能在 Chat Completions 里带工具，但等于关掉推理；gpt-6-astra 不接受 `none`，在 Chat Completions 里完全不能带工具。所以给 `ip-models` 加了 Responses 接口：端点配置 `api: responses`（可选 `reasoning_effort`），默认仍是 `chat_completions`，DeepSeek、vLLM 等不受影响。要点：

- 每次全量发送历史（`store: false`），不依赖服务端保存的上下文；工具调用转成 `function_call` / `function_call_output`。
- 工具发 `strict: false`：Responses 默认严格模式，外部 MCP 工具的 schema 多半不满足。
- 不发 `temperature`（推理模型一律拒绝）；`max_tokens` 映射为 `max_output_tokens`，它也计入推理 token。
- 没收到 `response.completed` 就断开时报错，不当作正常结束。

## 发现的问题（未修，留待评测阶段）

1. **通胀率拿不到，结论只能留空。** `get_fred_series` 只返回指数水平，没有同比/环比（FRED 的 `units=pc1` 没有暴露）。岗位说明要求“不要心算”，研究员于是如实写了“未取得通胀率”。可以在 which-api 的 `get_fred_series` 加 `units` 参数，或在 harness 里给 FRED 结果也提供 `transform=yoy/mom`。
2. **协调员过度核验。** 它要求“官方新闻稿和发布日期”才肯下通胀结论，又追加一轮核验任务，这类要求工具做不到，只能再走一次选择卡片。协调员 4 次派活里有 2 次是这种追加。可以在协调员说明里写明“工具返回的数据即为已核验来源”。
3. **协调员把核验数据的任务派给了运维测试岗位。** 运维测试员工在 16 步上限内没做完，这轮失败；按设计任务标为受阻、没有自动重派，所以没有循环花钱。协调员说明里可以写明各岗位的边界。
4. 报告文件名用的日期有一次是 11-10（研究员按最新观测日期起名），不影响内容。
