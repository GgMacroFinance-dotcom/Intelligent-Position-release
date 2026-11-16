# Finance Macro Data API（开源版，MCP 服务 + HTTP API + 命令行）

宏观研报岗位取宏观数据用的只读工具包，13 个工具。美国数据来自纽约联储、美国财政部（收益率曲线、TIC、每日国库报表）、Federal Register、SEC，以及经 WhichAPI 托管服务取的 FRED；中国数据来自国家统计局。

- **零运行时依赖**：只要 Node.js 18.17 或更高版本，不用 `npm install`。
- **三种用法**：MCP（stdio 或 HTTP）、REST 接口、命令行。
- **许可**：MIT，见本目录的 [LICENSE](LICENSE)。仓库其余部分是 Apache-2.0。

它来自 which-api 仓库的 `packages/macro-mcp` 0.2.0（分支 `claude/macro-mcp-product`，提交 `965b243`）。和那里的托管版相比，开源版有三处不同：

| | 托管版（WhichAPI） | 开源版（本目录） |
| --- | --- | --- |
| 能力目录 `discover_apis`、`get_api`、`call_api` | 有，187 个免 key API | 没有。它是 WhichAPI 网关代码的打包产物，不随这个包公开 |
| FRED | 托管服务自己取 | 不直接连 FRED，转发到 WhichAPI 托管服务，要 WhichAPI 的 key |
| 中国数据 | 有（同一套统计局工具，which-api PR #14） | 国家统计局 4 个工具：按中文名搜指标、按目录浏览、按指标取数（月度、季度、年度），以及 CPI、PPI、PMI、M2 的快捷入口 |
| ICE BofA 序列（`BAML` 开头，如投资级、高收益 OAS） | 快照里有三个面板 | 快照里没有这三个面板；序列用 `get_fred_series` 按 id 取，如 `BAMLH0A0HYM2`（高收益 OAS）、`BAMLC0A0CM`（投资级 OAS） |

## 快速开始

在仓库根目录：

```bash
node macro-mcp/bin/whichapi-macro.js snapshot
```

```bash
node macro-mcp/bin/whichapi-macro.js tools
```

```bash
node macro-mcp/bin/whichapi-macro.js call get_treasury_curve '{"limit":3}'
```

没有 WhichAPI 的 key 时，快照里“广义美元指数”和“美元兑人民币”两个面板会报 `requires_configuration`，其余面板照常；快照整体状态是 `partial`。

## 环境变量

| 变量 | 作用 |
| --- | --- |
| `WHICHAPI_API_KEY` | WhichAPI 的 key。`get_fred_series` 和快照里的两个 FRED 面板要它 |
| `WHICHAPI_URL` | 用哪个 WhichAPI 服务，默认 `https://whichapi-macro.onrender.com`。必须是 https |
| `SEC_USER_AGENT` | `get_company_concept` 要它，格式 `名字 邮箱`，SEC 要求自动请求带联系方式 |
| `MACRO_API_KEY` | 用 `serve` 起 HTTP 服务时，客户端要带的 Bearer key（见下文） |

FRED 为什么要经 WhichAPI：FRED 的条款要求用注册 key 的官方 API，不要抓取网页数据。WhichAPI 托管服务持有 FRED 的 key，这个包只把请求转发过去（`GET <WHICHAPI_URL>/v1/tools/get_fred_series`，带 `Authorization: Bearer <WHICHAPI_API_KEY>`）。托管服务冷启动要等 1 分钟左右，转发的超时是 90 秒。

## 工具

| 工具 | 内容 | 来源 |
| --- | --- | --- |
| `macro_snapshot` | 9 个面板：SOFR、EFFR（含目标区间）、广义美元指数、USD/CNY、10 年期美债（含整条曲线和 2s10s）、中国大陆和日本持有美债、债务上限余量、最新 232 公告。每个面板有最新值、观测日期、较前值变化、短序列；失败的面板单独报错 | 下列各来源 |
| `list_macro_panels` | 面板清单、单位与口径说明（不联网） | — |
| `get_reference_rates` | SOFR、TGCR、BGCR、EFFR、OBFR 日度，含分位数和成交量 | 纽约联储 |
| `get_fred_series` | FRED 序列（如 `DGS10`、`T10Y2Y`、`CPIAUCSL`、`UNRATE`），缺失日为 null；ICE BofA 的信用利差序列（`BAML` 开头）也从这里取。`units` 让 FRED 做变换，如 `pc1` 是同比 %，算通胀率用它 | FRED 官方 API，经 WhichAPI |
| `get_treasury_curve` | 1 个月到 30 年期面值收益率曲线（2025 年起） | 美国财政部 |
| `get_tic_holdings` | 某国持有美债、机构债、公司债、股票，月度，单位百万美元 | 财政部 TIC |
| `get_debt_limit` | 法定上限、受限债务、余量（由每日国库报表推算） | Fiscal Data |
| `search_federal_register` | 关税、232、出口管制等公告检索，可按机构、类型、日期过滤 | Federal Register |
| `get_company_concept` | 公司 XBRL 财务概念，如资本开支 `PaymentsToAcquirePropertyPlantAndEquipment` | SEC |
| `search_china_indicators` | 按中文名搜统计局指标（如 `社会消费品零售总额`、`国内生产总值`；`CPI`、`PPI`、`PMI`、`GDP` 会换成中文名），返回 `catalog_id`、`indicator_id`、指标名和最新值。站内搜索是模糊匹配，最新一期还没发布的指标搜不到，搜不到时用浏览 | 国家统计局 |
| `browse_china_catalog` | 一层一层浏览统计局数据库目录，叶子目录列出指标；浏览是完整的 | 国家统计局 |
| `get_china_indicator` | 按 `catalog_id` + `indicator_id` 取全国数据，新到旧，数值按统计局原样。期次：月度 `2026-10`、季度 `2026-Q3`、年度 `2025` | 国家统计局 |
| `get_china_series` | 快捷入口：`cpi_yoy`、`ppi_yoy`（已减 100 换成同比 %）、`pmi_mfg`（50 为荣枯线）、`m2`（亿元）、`m2_yoy`（同比 %）。存的 id 失效时按目录名和指标名重新找到（`resolved` 是 `by_name`） | 国家统计局 |

所有联网的工具都接受 `max_age_seconds`：默认用几分钟到几小时的内存缓存（看来源更新频率），传 `0` 强制重新请求。返回给模型的内容分两部分：一段可读摘要，加完整的结构化数据。

## 接入 I.P.

`rust/ip.example.yaml` 默认就用这个目录本机启动（stdio），路径相对于启动 `ip serve` 时所在的目录：

```yaml
mcp_servers:
  macro:
    command: node
    args: [../macro-mcp/bin/whichapi-macro.js, mcp]
    env_inherit: [SEC_USER_AGENT, WHICHAPI_URL]
    # env_from: { WHICHAPI_API_KEY: WHICHAPI_API_KEY }
    roles: [macro-researcher]
```

有 WhichAPI 的 key 时，取消 `env_from` 那一行的注释，并在启动 `ip serve` 前设置 `WHICHAPI_API_KEY`。

I.P. 启动这个包时不把自己的环境变量交给它，只带 `PATH`、`HOME` 等几个基本变量。上表里的变量要经 `env_inherit`（设了就带过去）或 `env_from`（必须设置）写明才会传进来。

## 接入其他 MCP 客户端

Claude Code：

```bash
claude mcp add whichapi-macro -e WHICHAPI_API_KEY=你的key -- node /path/to/intelligent-position/macro-mcp/bin/whichapi-macro.js mcp
```

Claude Desktop、Cursor 等支持 stdio MCP 的客户端，配置方式相同：命令 `node`，参数是脚本路径和 `mcp`，环境变量里放 `WHICHAPI_API_KEY`。

## HTTP 服务

```bash
node macro-mcp/bin/whichapi-macro.js serve --port 3220
```

| 地址 | 用途 |
| --- | --- |
| `POST /mcp` | MCP Streamable HTTP（返回 JSON，不开 SSE 流） |
| `GET /v1/snapshot?panels=sofr,effr&max_age_seconds=0` | 宏观快照 |
| `GET /v1/panels`、`GET /v1/tools` | 面板与工具清单 |
| `GET` 或 `POST /v1/tools/{name}` | 调用单个工具，GET 用查询参数，POST 用 JSON |
| `GET /`（浏览器） | 测试页：看快照、调用任一工具、复制 MCP 接入命令 |
| `GET /health` | 健康检查（不需要 key） |

安全默认值：

- 默认只监听 `127.0.0.1`；本机监听时只接受 Host 为 localhost、127.0.0.1 或 [::1] 的请求，并拒绝非本机 Origin（防 DNS 重绑定）。格式错误的请求返回 400，不会让进程退出。
- 设置 `MACRO_API_KEY` 后，除 `/health` 外都要带 `Authorization: Bearer <key>`。
- 监听非本机地址（如 `--host 0.0.0.0`）时必须设置 `MACRO_API_KEY`，否则拒绝启动；确实要匿名开放时加 `--allow-anonymous`。
- 每个 IP 每分钟最多 120 次请求；上游只允许访问上表列出的官方域名，外加 `WHICHAPI_URL` 指向的那一个。

## 在代码里直接用

```js
import { snapshot, ticHoldings } from './macro-mcp/src/index.js';
const s = await snapshot({ panels: ['sofr', 'ust_10y'] });
const japan = await ticHoldings({ country: 'Japan', limit: 6 });
```

## 口径说明（写报告时请照抄）

- `broad_dollar` 是美联储名义广义美元指数（2006 年 1 月 = 100），**不是 ICE DXY**，只能写成“广义美元指数”。
- 债务上限余量是用每日国库报表的公开行相减算出来的，**不是官方 X 日估计**，也不含非常规措施。
- TIC 的国家是托管机构所在地，不等于最终持有人；比利时、开曼等托管中心的持仓可能属于别国投资者。
- 每个数值都附带来源观测日期（`as_of`），请和数值一起引用；它不是抓取时间。
- 国家统计局的 CPI、PPI 原值是“上年同月=100”的指数，`get_china_series` 已经减去 100 换成同比 %。

## 国家统计局的限制

统计局网站的[服务条款](https://www.stats.gov.cn/wzgl/202302/t20230217_1912857.html)要求转载或引用时在显著位置注明来自国家统计局网站，并标明网址 www.stats.gov.cn。所以统计局工具的每个结果都带 `attribution` 字段：“数据来自国家统计局网站（www.stats.gov.cn）”，可读摘要的最后一行也是它。用这些数据写报告或再转给别人时，照写这一句。

统计局没有正式开放的 API。这几个工具用的是统计局数据库网站（2026 年改版后的 `data.stats.gov.cn/dg/website/`）自己取数的接口，不用登录和 key，网站再改版时可能要调整。旧接口 `easyquery.htm` 和 `A01010101` 这类代码已经停用，2026-11-15 从境内网络访问也返回 403。

指标用目录 id 和指标 id 定位，id 没有含义，所以先搜或浏览，再取数。有的序列按基期分在几个目录里（如 CPI 的 2016-2020、2021-2025、2026-），`browse_china_catalog` 的 `from_year`、`to_year` 标出年份；`get_china_series` 会自己拼起来。2026-11-15 在境内网络上真实调用过四个工具；境外网络能否访问还没测，被拒时工具报 `upstream_error`。

代码和 which-api 仓库 `packages/macro-mcp/src/china.js` 是同一份，改的时候两边一起改。

## 网络与代理

工具用 Node 自带的 `fetch` 访问上游。公司网络要走代理时，Node 22.21+/24.5+ 设置 `NODE_USE_ENV_PROXY=1` 和 `HTTPS_PROXY` 即可；更早的版本不读代理变量。

## 测试

```bash
cd macro-mcp && node --test test/*.test.js
```

测试不联网，上游全部用 `test/fixtures.js` 里录好的格式模拟。对真实上游跑一遍：

```bash
cd macro-mcp && node test/live.js
```

设了 `WHICHAPI_API_KEY` 才会测 FRED，设了 `SEC_USER_AGENT` 才会测 SEC。
