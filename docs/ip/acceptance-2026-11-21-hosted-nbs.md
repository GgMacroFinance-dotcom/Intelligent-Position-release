# 托管版 Finance Macro Data API 访问国家统计局验收记录（2026-11-21）

> 对应 backlog E-2、发布前清单 R-24 里“境外能否访问没验”的一项。
> 被测服务：Render 上的托管版 Finance Macro Data API（`https://whichapi-macro.onrender.com/mcp`，`serverInfo` 报 `whichapi-macro` 0.2.0），统计局 4 个工具来自 which-api PR #14。
> 真实服务、真实数据，没有用假服务。

## 环境

- 日期：2026-11-21（响应头 `Date: Sat, 21 Nov 2026 21:09:56 GMT`）。
- 发起方：Claude Code 的云容器，经出站代理出网。出口不在境内，具体在哪个地区不确定；到 Cloudflare 的那一跳 `Cf-Ray` 后缀是 `IAD`（美国弗吉尼亚阿什本的节点）。代理自动给这个主机名注入凭据，请求里没有手写 key，也没有发任何个人信息（`clientInfo` 只写了 `ip-check`）。
- 真正去访问统计局的是 Render 上的托管服务，不是这个容器。Render 的部署区域都在境外，这个服务具体在哪个区域、怎么出网，这次没查。
- 协议：MCP Streamable HTTP，`POST /mcp`，`Accept: application/json, text/event-stream`，协议版本 `2025-06-18`。服务直接回 JSON，没有返回 `Mcp-Session-Id`。
- 工具：`curl`。所有取数调用都传了 `max_age_seconds: 0`，返回里 `cached` 都是 `false`，即服务这次实际请求了统计局，不是缓存。
- 只发了下表这几次请求，没有压测。

## 结果

| 步骤 | 结果 | 耗时 |
| --- | --- | --- |
| `initialize` | ✓ 200，第一次就通（服务没在休眠，或很快醒了） | 11.6 s |
| `notifications/initialized` | ✓ 202 | 0.2 s |
| `tools/list` | ✓ 200，16 个工具，统计局 4 个都在：`search_china_indicators`、`browse_china_catalog`、`get_china_indicator`、`get_china_series`（另有 9 个美国工具和 `discover_apis`、`get_api`、`call_api`） | 0.7 s |
| `search_china_indicators` `{"query":"CPI","limit":5}` | ✓ CPI 换成“居民消费价格指数”，共 92 条月度指标，返回前 5 条，例如“居民消费价格指数 (上年同月=100)”最新 2026-10 = 100.7 | 3.5 s |
| `browse_china_catalog`（目录根，月度） | ✓ 14 个顶层目录：价格指数、工业、能源、固定资产投资 (不含农户)、服务业生产指数、城镇调查失业率、房地产、国内贸易、对外经济、交通运输、邮电通信、采购经理指数、财政、金融 | 0.9 s |
| `get_china_indicator`（CPI 同比：`catalog_id` `5c7452825c7c4dcba391db5ca7f335c5`，`indicator_id` `53180dfb9c14411ba4b762307c85920c`，3 期） | ✓ 居民消费价格指数 (上年同月=100)，全国：2026-10 100.7，2026-09 100.9，2026-08 100.8 | 1.7 s |
| `get_china_series` `{"indicator":"ppi_yoy"}`（3 期） | ✓ 工业生产者出厂价格指数 (上年同月=100) 减 100：2026-10 5.7，2026-09 5，2026-08 3.8；`resolved` 是 `stored_ids` | 0.5 s |
| `get_china_series` `{"indicator":"pmi_mfg"}`（3 期） | ✓ 制造业采购经理指数 (%)：2026-10 49.3，2026-09 50.1，2026-08 49.8；`resolved` 是 `stored_ids` | 0.5 s |

每次取数的返回都带来源署名“数据来自国家统计局网站（www.stats.gov.cn）”，`source` 是 `National Bureau of Statistics of China`，`url` 指向 `data.stats.gov.cn/dg/website/...`（2026 年改版后的新站）。没有一次被统计局拒绝，没有 4xx/5xx，`isError` 都是 `false`。

数值照工具返回原样记录，没有和统计局发布稿逐个核对。

## 结论

2026-11-21，Render 上的托管版能从境外访问国家统计局：4 个统计局工具都在，搜指标、浏览目录、按指标取数、快捷入口各调一次都实时取到了数。

2026-11-11 验收里“接口拒绝境外 IP”说的是统计局旧接口 `easyquery.htm`，那个接口 2026-11-15 已确认停用（backlog A-2），和这次的新接口不是一回事。

## 还没验的

- 只验了托管版。在境外主机上本机启动开源版 `macro-mcp`（stdio）直连统计局，这次没测。
- 只测了一个时间点、一个出口。统计局以后会不会按地区或频率限制，不知道。
- 季度、年度频率，`m2`、`m2_yoy`、`cpi_yoy` 快捷入口，以及 `resolved: by_name` 的回退路径，这次没调。
- 没让真实模型经岗位走一遍（这次是 `curl` 直接调工具）。
