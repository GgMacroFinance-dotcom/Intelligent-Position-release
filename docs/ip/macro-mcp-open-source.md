# Finance Macro Data API 开源前的核对（backlog C-2）

> 2026-11-12。只做了核对，没有改 which-api 仓库里的任何东西。
> 结论：现在拆不出来，卡在第 4 节的三件事上，都要维护者定。上游条款只有 1 项读到了原文，其余要人打开原文再确认。
> 2026-11-13 更新：维护者定了，本仓库这边已经做完，见第 6 节。

## 1. 这个包现在在哪

- 在 which-api 仓库的 `packages/macro-mcp`，包名 `@whichapi/macro-mcp`。**不在 `main` 上**：which-api 的 `main` 没有 `packages/` 目录。
- 它在一串还没合并的 PR 顶上：#8 → #9 → #10 → #12，每个 PR 的目标分支是前一个。

| 版本 | 分支 | PR | 内容 |
| --- | --- | --- | --- |
| 0.2.0 | `claude/macro-mcp-product` | which-api #10（未合并） | 12 个工具，目录 187 个 API。Render 测试部署和 I.P. 的真实验收用的是这一版 |
| 0.3.0 | `claude/stoic-maxwell-b7u2yd` | which-api #12（未合并） | 目录扩到 256 个 API，加了按分类的 MCP 入口 |

- `package.json` 里现在是 `"license": "UNLICENSED"`、`"private": true`。
- 没有运行时依赖，只要 Node.js 18.17 以上。共 24 个文件，约 0.75 到 0.85 MB，其中 `vendor/catalog.js` 占 0.59 到 0.67 MB。

## 2. 它由两部分组成

| 部分 | 工具 | 代码 | 和 which-api 其余部分的关系 |
| --- | --- | --- | --- |
| 美国宏观数据 | 9 个：`macro_snapshot`、`list_macro_panels`、`get_reference_rates`、`get_fred_series`、`get_treasury_curve`、`get_tic_holdings`、`get_debt_limit`、`search_federal_register`、`get_company_concept` | `src/sources.js`、`src/snapshot.js`、`src/upstream.js` 等 | 自成一体，不依赖仓库里别的代码 |
| WhichAPI 能力目录 | 3 个：`discover_apis`、`get_api`、`call_api` | `vendor/catalog.js` | **是 which-api 网关 `apps/web/lib` 的目录和适配器打包出来的**。重新生成要用 `scripts/build-catalog.mjs`，它依赖 `apps/web` |

`vendor/catalog.js` 里另外打包了 5 个第三方库：ajv、ajv-formats、fast-deep-equal、json-schema-traverse（MIT）和 fast-uri（BSD-3-Clause），许可证全文在 `vendor/THIRD_PARTY_LICENSES.txt`。它们和 MIT 兼容。

I.P. 这边实际用到了什么：真实验收和两遍评测里，研究员调了 `get_fred_series` 10 次、`get_treasury_curve` 3 次、`macro_snapshot` 3 次。目录的三个工具一次都没调过，只在 `rust/ip.example.yaml` 的岗位说明里作为“别的工具都没有时”的后备提了一句。

## 3. 上游数据源的使用条款

> 2026-11-15 更新：纽约联储、SEC 已读原文，另加了国家统计局，结论和要定的事见[条款核对](upstream-terms-2026-11-15.md)。下表是 2026-11-12 的记录，留作对照。

“核对程度”一栏：**原文** 表示读了官方页面；**摘要** 表示只看到搜索结果里的片段，没读到原文；**没核对** 表示什么都没查到。
这台机器取不到 SEC、Federal Register 的页面（站点拒绝或中断了自动访问），纽约联储的条款页地址没找到。

| 来源 | 包怎么取数 | 条款要点 | 核对程度 |
| --- | --- | --- | --- |
| 财政部 Fiscal Data（债务上限） | 官方开放 API，不要 key | “The data is offered free, without restriction, and available to copy, adapt, redistribute, or otherwise use for non-commercial or commercial purposes.”（[API 文档](https://fiscaldata.treasury.gov/api-documentation/)的 License and Authorization 一节） | 原文 |
| 纽约联储参考利率（SOFR、EFFR 等） | `markets.newyorkfed.org` 的公开 API | 有一份专门的 “Terms of Use for Select Rate Data”，转载这些利率的机构都注明受它约束。条文本身没读到 | 摘要 |
| SEC（公司财务概念） | `data.sec.gov` 的 XBRL 接口，带 `SEC_USER_AGENT` | 每秒最多 10 次请求；请求头要声明 User-Agent（[Accessing EDGAR Data](https://www.sec.gov/os/accessing-edgar-data)）。包里已经要求填 `SEC_USER_AGENT`，没看到限速 | 摘要 |
| 财政部官网（收益率曲线）、TIC | 官网的 CSV 和文本文件 | — | 没核对 |
| Federal Register | 官方 API | — | 没核对 |

纽约联储多半只是要带一段声明，但条文当时没读到。

如果目录部分也公开，还要再核对它覆盖的几十个来源（0.2.0 的 README 写的是 40 个），其中有 OKX、Bitstamp、Coinbase、Kraken、CoinGecko 等商业站点。这一步没有做。

## 4. 要维护者定的三件事

| 事项 | 为什么要定 | 建议 |
| --- | --- | --- |
| 目录部分（3 个工具和 `vendor/catalog.js`）要不要一起公开 | [项目总览](project-spec.md)写的是“which-api 的其余部分不随之公开”，而目录正是 which-api 网关代码的打包产物 | 先只开源 9 个宏观工具。I.P. 的真实运行没用到目录；去掉以后包只剩 0.1 MB 出头，也不用再核对目录覆盖的几十个来源的条款 |
| 开源哪个版本，放在哪 | 两个版本都没合进 which-api 的 `main`；“拆出来”要么新建一个仓库，要么放进 I.P. 的公开仓库 | 放进 I.P. 仓库的一个目录（自带 MIT 的 `LICENSE`）。示例配置可以直接用相对路径本机启动，也少建一个仓库。只开源宏观工具的话选哪个版本差别不大：取数的代码（`sources.js`、`snapshot.js`、`upstream.js`）两个版本完全一样 |
| FRED 怎么取，快照带哪些面板 | `get_fred_series` 是 I.P. 调得最多的工具，当时下载的是网页图表用的 `fredgraph.csv` | `get_fred_series` 改成走官方 API |

## 5. 定了以后 Claude 能接着做的

- 按决定整理出要公开的那部分代码，把 `package.json` 改成 `"license": "MIT"`、去掉 `"private"`，加 `LICENSE`，改 README。
- `rust/ip.example.yaml` 的 `mcp_servers.macro` 默认改成本机 stdio 启动；岗位说明里去掉或保留目录工具那一句，跟着第一件事的决定走。
- 根目录 `README.md` “跑起来”一节里“没有 Finance Macro Data API 的密钥”那一行跟着改。
- 重跑 `ip eval`，确认换成本机启动、换了 FRED 取数方式之后两个问题仍然通过（要 OpenAI 密钥，以及 FRED 的 key）。

## 6. 维护者的决定和做了什么（2026-11-13）

| 事项 | 决定 |
| --- | --- |
| 目录部分要不要一起公开 | 不公开，先只开源 9 个宏观工具 |
| 开源哪个版本，放在哪 | 放进本仓库根目录的 `macro-mcp/`。用的是 0.2.0（which-api 分支 `claude/macro-mcp-product`，提交 `965b243`），取数代码和 0.3.0 一样 |
| FRED 怎么取 | 要求用户走 WhichAPI：开源包不直连 FRED，转发到 WhichAPI 托管服务，用户申请 WhichAPI 的 key（`WHICHAPI_API_KEY`） |
| ICE BofA 三个序列 | 开源版去掉 |

做了的：

- `macro-mcp/`：9 个工具、MCP（stdio 和 HTTP）、REST、命令行、测试页，`LICENSE` 是 MIT，`package.json` 的许可证字段改成 MIT（`private` 还留着，发不发 npm 另定）。
- `get_fred_series` 改成请求 `<WHICHAPI_URL>/v1/tools/get_fred_series`，带 `Authorization: Bearer <WHICHAPI_API_KEY>`；没设 key 时返回 `requires_configuration`，不发请求。`fred.stlouisfed.org` 从上游白名单里删掉了，包里没有任何地方直连 FRED。
- `BAML` 开头的序列 id 直接拒绝；快照去掉投资级 OAS、高收益 OAS、投资级有效收益率三个面板，剩 9 个。
- 去掉 `vendor/catalog.js`、`discover_apis`、`get_api`、`call_api` 和生成目录的脚本，打包进目录的 5 个第三方库也随之去掉。
- 测试改成模拟 WhichAPI 的回复，新增没 key、key 被拒、错误转发、ICE 序列、改 `WHICHAPI_URL` 的用例；新增 CI 工作流 `macro-mcp`，在 Node 18.17 和 22 上跑。
- `rust/ip.example.yaml` 的 `mcp_servers.macro` 改成本机启动 `node ../macro-mcp/bin/whichapi-macro.js mcp`，没有 key 也能启动；岗位说明去掉目录工具，写明 ICE 序列和没 key 时写“未取得”。

还没做的（which-api 那边，等人）：

1. ~~**托管服务取 FRED 改走官方 API。**~~ 完成（2026-11-14，which-api PR #13）：托管服务改调 `api.stlouisfed.org`，FRED 的 key 配在 Render 的 `FRED_API_KEY`（`render.yaml` 里 `sync: false`），还加了 `units` 参数；本仓库的开源包随后也把 `units` 转发过去。用维护者的 key 在本机跑过真实调用；Render 上部署后的调用 2026-11-15 直接请求过一次，`get_fred_series` 取到了数（见[发布前清单](release-checklist.md)）。
2. **给用户发 WhichAPI key。** 托管服务现在只有一个共用的 `MACRO_API_KEY`，没有按用户发 key 的办法。

经 WhichAPI 取 FRED 这条路，写这份核对时只用模拟测过。之后真实跑过：2026-11-14 的评测用本机 `macro-mcp` 经 WhichAPI 取 FRED，配的是真实模型（[记录](evals/2026-11-14-e8/README.md)）。

