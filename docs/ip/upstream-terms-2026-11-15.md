# 上游数据源使用条款核对（发布清单 R-3）

> 2026-11-15。读了 FRED、纽约联储、SEC、国家统计局四家的条款原文（页面地址和读到的日期见各节）。
> 这是工程上的核对，不是法律意见。第 6 节列出的要维护者或律师定。
> 财政部 Fiscal Data 2026-11-12 已读过原文，见[开源前的核对](macro-mcp-open-source.md)第 3 节；财政部官网（收益率曲线）、TIC、Federal Register 这次仍没核对。

## 1. 结论

| 来源 | 开源代码里调用 | 经托管服务转发、缓存 | 署名或声明 | 频率 | 结论 |
| --- | --- | --- | --- | --- | --- |
| FRED | 可以，但每个用户要用自己的 key | **和条款冲突**：一个共用 key 服务所有用户、缓存后提供给第三方都写明不允许 | 必须带固定声明、条款链接；数据写明原始来源“via FRED” | 每分钟 120 次 | **现在的做法要改**，见第 2 节和第 6 节第 1、2 条 |
| FRED 上的 ICE BofA 等版权序列 | 不行，未经版权方书面许可不得复制 | 不行 | — | — | 开源包已拒绝 `BAML`；**托管服务还在返回**；其他标了 “Pre-Approval Required” 的序列没拦 |
| 纽约联储参考利率 | 可以，允许自动访问 | 可以，个人或商业用途都许可 | 必须带一段固定的声明和免责；SOFR、BGCR 另有 DTCC 的免责 | 不得影响网站运行，没给数字 | 可以用，**要补声明** |
| SEC（data.sec.gov） | 可以，不要 key | 可以，网站信息是公开信息，可复制再分发 | 请求注明 SEC 为来源（“请考虑”，不是硬性要求） | 每个用户合计每秒 10 次，不论几台机器 | 可以用，**要补限速**；User-Agent 已经要求 |
| 国家统计局 | 网站允许下载和使用统计数据；用的是网站自己的页面接口，条款既没允许也没禁止程序调用 | **要人定**：转载须是“公共免费信息为使用目的”的合理引用，且不得“擅自同意他人继续转载” | 必须在显著位置注明来自国家统计局网站和网址 `www.stats.gov.cn` | 没给数字；禁止探测、扫描、干扰网站运行 | 自己用没问题；**托管转发和商业用途要人定**；要补署名 |

最急的一件：Render 上的托管服务现在仍然返回 ICE BofA 序列（2026-11-15 请求 `BAMLC0A0CM` 取到了数，见[发布前清单](release-checklist.md) R-4）。ICE 的条款写明它可以作为第三方受益人直接向使用者主张权利。这件事不用等开源，Claude 建议先在 which-api 那边拦掉。**维护者 2026-11-16 定：先不停，写信向 ICE Data Indices 申请授权。**

## 2. FRED

读的页面（2026-11-15，页面上都没有“最后更新”日期）：

- [Legal 页](https://fred.stlouisfed.org/legal/)，全文
- [API Terms of Use](https://fred.stlouisfed.org/docs/api/terms_of_use.html)
- [API Keys](https://fred.stlouisfed.org/docs/api/api_key.html)、[Errors](https://fred.stlouisfed.org/docs/api/fred/errors.html)
- [BAMLC0A0CM 序列页](https://fred.stlouisfed.org/series/BAMLC0A0CM)的说明

注意：Legal 页里的 API 禁止条款比单独的 API Terms 页多几条（缓存、存储、AI 训练）。下面按更严的 Legal 页算。

### 2.1 条款原文要点

**key 是每个用户自己的。**

- API Keys 页：“Developers should request a distinct API key for each application they build.”
- 同页：“All users of an application shall use their own API key.”
- Legal 页说 key 是 “uniquely associated with you”。

**不许缓存、存储后提供给第三方。**

- Legal 页 API 禁止条款 (l)：“Use the FRED® API in connection with storing, caching, or archiving any portion of the FRED® Services or FRED® Content”，中间还禁止把存储或缓存的部分提供给任何第三方，结尾是 “or incorporating any FRED® Content in any database, compilation, archive, cache, or other medium.”
- 一般禁止条款 (p) 内容相同，放在 “You may not, without the Bank's prior written consent:” 之下。唯一的例外是圣路易斯联储的书面同意。没找到临时缓存的例外。
- 还禁止 (m) 用 API 批量下载（“wholesale downloading or accessing”）。

**商业用途。**

- 一般许可是 “solely for your personal, non-commercial use”。
- 禁止条款 (j)：未经书面同意，不得把 FRED 服务用于任何商业用途（“Except as expressly authorized in the FRED® Terms of Service”）。
- 摘要第 IV 节：标为 “Public Domain: Citation requested” 和 “Copyrighted: Citation required” 的序列可以内部商用，也可以放进给客户的报告，要署名。

**AI 训练。** API 禁止条款 (k)：不得把 API 用于 “the development or training of any software program or system or machine learning”。摘要第 II 节对所有 FRED 使用都有同样的限制。

**署名和声明。**

- 应用里要醒目放这句：“This product uses the FRED® API but is not endorsed or certified by the Federal Reserve Bank of St. Louis.”
- 给别人用的应用要向用户展示 FRED API 条款的链接，自己的条款里写明用户同意受 FRED API 条款约束，并公开一份隐私政策。
- 应用的主机名里不能出现 “FRED®”、“ALFRED®”、“Federal Reserve Bank”。
- 数据要写原始来源，再写经 FRED 取得，例如 “Source: BLS via FRED”。

**频率。** Errors 页：“Up to 120 requests per minute are allowed before being served a 429 error code.” 不遵守可能被暂时封禁。

**第三方版权序列。**

- 序列分三档：“Public Domain: Citation requested”、“Copyrighted: Citation required”、“Copyrighted: Pre-Approval Required”。API Terms 页说版权序列的说明里含 “Copyright” 一词。
- `BAMLC0A0CM` 标的是 “Copyrighted: Pre-Approval Required”。说明里写：“Reproduction of this data in any form is prohibited except with the prior written permission of ICE Data Indices.”，并写明 ICE 可以作为 FRED 条款的第三方受益人直接主张权利。
- Legal 页：圣路易斯联储和 FRED 不能代版权方给许可，要直接找版权方。
- Visa 消费动能指数（SMI）另有条款：只许个人非商业下载，不得提供给第三方。

### 2.2 对照现在的做法

现在（维护者 2026-11-13 定）：开源包不直连 FRED，转发到 WhichAPI 托管服务；托管服务用一个 FRED key（配在 Render 的 `FRED_API_KEY`）替所有用户取数，用户另申请 WhichAPI 的 key。

| 现在的做法 | 和条款的关系 |
| --- | --- |
| 托管服务一个 key 服务所有用户 | 和 “All users of an application shall use their own API key” 冲突 |
| 托管服务和开源包都有内存缓存（开源包取 FRED 默认缓存 1 小时） | 字面上和 (l)、(p) 冲突。单用户进程内的短时缓存算不算，条款没说 |
| 托管服务把数据提供给别的用户 | 等于把取到的 FRED 内容提供给第三方，(l) 写明不许；若收费还碰到 (j) |
| 只拦 `BAML` 开头的序列 | 其他 “Pre-Approval Required” 序列没拦（FRED 上不止 ICE 一家） |
| 返回的 `source` 是 “Federal Reserve Bank of St. Louis (FRED)” | 应写原始来源 + via FRED，例如美元指数写联储理事会 |
| `note` 里已带 FRED 的声明 | 符合 |
| 托管服务地址 `whichapi-macro.onrender.com` | 主机名里没有 FRED 字样，符合 |

### 2.3 要改的地方

1. **取数方式**（要维护者定，第 6 节第 1 条）。建议开源包改回直连 FRED 官方 API（`api.stlouisfed.org`），用用户自己的 `FRED_API_KEY`，免费申请。这样 FRED 不再依赖 WhichAPI 发 key（R-4 的一半也就不用做了）。
2. **版权序列**：取数前先读序列说明（`fred/series` 接口的 `notes`），含 “Copyright” 的默认拒绝，或至少拒绝 “Pre-Approval Required” 的，不再只认 `BAML` 前缀。
3. **署名**：`source` 写原始来源加 “via FRED”；README 写明 FRED 条款链接和用户须遵守它。
4. **限速**：客户端按每分钟 120 次以内控制。
5. **缓存**：直连之后，进程内缓存是否保留要定（第 6 节第 3 条）。保守做法是 FRED 默认不缓存。
6. **训练数据**：以后后训练岗位小模型时，经 FRED API 取到的数据不能进训练集。建议写进项目规则（第 6 节第 4 条）。

## 3. 纽约联储

读的页面：[Terms of Use](https://www.newyorkfed.org/privacy/termsofuse)，“Last Updated: 6/9/2023”。`newyorkfed.org/termsofuse` 是 404，正确地址在 `/privacy/` 下。

要点：

- 许可：“The New York Fed grants you a non-exclusive license, subject to the Terms, to use, copy, and distribute Content for your personal or business purposes.” 可以 “manually or through an automated process or device” 访问，只要不影响网站运行；可以下载、存储、复制、分发、修改。条款写明 API 取到的内容同样适用。
- 再分发时要带上同样的条款，不得加更严的限制。
- 不得暗示纽约联储背书；不得把它的名字用于广告或商业目的。
- **参考利率专门的限制**：使用或分发参考利率数据，必须附这段声明：“The [NAME OF DATA or CONTENT] is subject to the Terms of Use posted at newyorkfed.org. The New York Fed is not responsible for publication of the [DATA NAME] by [NAME OF PUBLISHER], does not [sanction] or [endorse] any particular republication, and has no liability for your use.”
- 在产品名称或描述里用 SOFR 等利率名称时，要醒目写明 “[User] is not affiliated with the New York Fed. The New York Fed does not sanction, endorse, or recommend any products or services offered by [user].”
- SOFR 和 BGCR 用了 DTCC Solutions 授权的数据，条款里有一句 DTCC 及其数据提供方不承担责任的说明。
- 用户须自负风险，并对因使用内容产生的第三方索赔给纽约联储补偿（Indemnification）。适用美国联邦法和纽约州法。

要改的地方：

1. `get_reference_rates` 和快照的 SOFR、EFFR 面板返回里加 `notice` 字段，放上面那段声明（发布方填 “Finance Macro Data API” 或用户自己的产品名）；SOFR、BGCR 另加 DTCC 那句。
2. README 加“数据来源与条款”一节，写明这段声明的要求。
3. 主页和文档里提到 SOFR 时，不写成和纽约联储有关系。主页、根 README 里搜不到 SOFR。

## 4. SEC

读的页面：

- [Accessing EDGAR Data](https://www.sec.gov/os/accessing-edgar-data)，“Last Reviewed or Updated: June 26, 2024”
- [EDGAR APIs](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)，“Last Reviewed or Updated: April 8, 2025”
- [Privacy and Security Policy](https://www.sec.gov/privacy)，“Last Reviewed or Updated: Nov. 29, 2023”
- [Developer Resources](https://www.sec.gov/developer-resources)，“Last Reviewed or Updated: March 10, 2025”

要点：

- 再分发：“Information presented on sec.gov is considered public information and may be copied or further distributed by users of the web site without the SEC's permission.” 页面没有单独说缓存、托管转发、商业用途，也没有限制它们。
- 署名：“Please consider appropriate citation to the SEC as the source.” 是请求，不是硬性要求。
- 频率：“Current max request rate: 10 requests/second.” 隐私页写明是每个用户合计，不论用几台机器；超过的 IP 会被暂时限制，降到阈值以下 10 分钟后恢复。
- User-Agent：“Please declare your user agent in request headers”，示例是 “Sample Company Name AdminContact@<sample company domain>.com”。
- 不允许 “unclassified” 的机器人爬站；要 “efficient scripting”，只下需要的。
- 不要 key：“These APIs do not require any authentication or API keys to access.”
- 商标：不能在 SEC 或 EDGAR 相关业务的商号、商标、域名里用 EDGAR；不能用 SEC 的徽章和标志。

对照现在的做法：`get_company_concept` 已经要求 `SEC_USER_AGENT`，且必须带邮箱，符合；结果缓存 1 小时，条款没限制。

要改的地方：

1. 在 `data.sec.gov` 的请求前加客户端限速，例如每秒不超过 5 次，留出余量。
2. 托管服务若也提供这个工具，它的 User-Agent 要写真实的联系人和邮箱（第 6 节第 6 条）。
3. README 写明来源 “SEC EDGAR” 和每秒 10 次的限制。

## 5. 国家统计局

读的页面：国家统计局网站[服务条款](https://www.stats.gov.cn/wzgl/202302/t20230217_1912857.html)，页面日期 2022/09/02。`data.stats.gov.cn`（数据库，2026 年改版后的新网站）这台云主机连不上（连接被重置），它自己有没有另外的条款**没核对**。

原文要点：

- 署名：“转载或引用本网内容，务必在显著位置注明转自（或引自）国家统计局网站（或中国统计信息网）字样，并标明本网网址：www.stats.gov.cn。”
- 版权：“本网所有内容，未经注明，版权一律归国家统计局网站（中国统计信息网）所有。”
- 可以转载，但除外的有 “本网中特有的图形、标志、页面风格、编排方式、程序等”。
- 用途：“转载或引用本网内容必须是以新闻性或资料性公共免费信息为使用目的的合理、善意引用，不得对本网内容原意进行曲解、修改。”
- 转载引用时不得 “擅自同意他人继续转载、引用本网内容”。
- 数据：“用户可以在本网站下载和使用国家统计局发布的统计数据”；“个人和组织机构应依法使用国家统计局网站发布的统计数据”。
- 安全：“严禁非法探测、扫描、测试本网系统”；“严禁企图干涉、破坏网站的正常运行”。
- 没有正式开放的 API，也没有数据使用协议；条款里没有专门说程序访问或爬虫。

对照现在的做法：

- `macro-mcp/src/china.js` 调的是网站自己的页面接口（`queryIndexTreeAsync`、`esData` 等），不是公开的 API。条款只禁止探测、扫描和干扰运行，正常频率的只读请求不算这些；但这些接口也没有对外承诺，随时可能变（2026 年改版已经变过一次）。
- 返回的 `source` 是英文 “National Bureau of Statistics of China”，没有写网址。
- 托管服务（which-api 那份 `china.js`）把统计局数据转给别的用户。条款要求“公共免费信息为使用目的”，且不得同意他人继续转载；托管服务若收费，或允许下游再分发，就要人定。

要改的地方：

1. ~~返回里加署名，README 同样写明。~~ 完成（2026-11-16，本 PR）：统计局工具的结果都带 `attribution`“数据来自国家统计局网站（www.stats.gov.cn）”，可读摘要最后一行也是它；MCP 说明和示例配置的岗位说明要求引用时照写；README 写明了条款要求。只用模拟的统计局回复测过。which-api 那份 `china.js` 没改（这次没有那个仓库的权限），要一起改。
2. 统计局的请求加限速（条款没给数字，建议每秒不超过 1 到 2 次），只读。
3. 文档写明这是网站页面接口，没有对外承诺，可能失效。现在 `china.js` 开头的注释和 README 已经写了。

## 6. 要维护者或律师定的

| # | 事项 | 谁定 | 建议 |
| --- | --- | --- | --- |
| 1 | FRED 取数方式：开源包改回直连官方 API、用户自己申请 FRED key；还是保留经 WhichAPI 转发，另写信请圣路易斯联储书面同意 | 维护者 | **已定（维护者 2026-11-16）：保留经 WhichAPI 转发，另写信请圣路易斯联储书面同意。** 拿到书面同意之前，这一条仍是开源门槛上的风险（Claude 当时建议改直连） |
| 2 | Render 上的托管服务现在用共用 key 转发 FRED、仍返回 ICE 序列：先停掉 FRED 转发，还是至少先拦 ICE 和其他版权序列 | 维护者 | **已定（维护者 2026-11-16）：ICE 序列先不停，写信向 ICE Data Indices 申请授权（范围、缓存、收费用途、费用）。** 拿到授权之前，托管服务继续返回 ICE 数据是已知风险（Claude 当时建议马上拦） |
| 3 | 单用户进程内的短时缓存算不算 FRED 条款 (l)、(p) 禁止的 “caching” | 律师，或写信问 FRED | 问清楚之前，FRED 默认不缓存 |
| 4 | 以后后训练岗位小模型，训练数据能不能用经 FRED API 取的数 | 维护者记成规则 | 不能用，FRED 条款 (k) 写明禁止；需要宏观数据训练时，从原始发布方（BLS、BEA、联储理事会等）取 |
| 5 | 国家统计局数据经托管服务转发，尤其收费时，是否符合“公共免费信息为使用目的”和“不得擅自同意他人继续转载” | 律师 | **已定（维护者 2026-11-16）：统一替用户注明来源即可。** 工具返回里带署名“数据来自国家统计局网站（www.stats.gov.cn）”，岗位说明要求报告照写。收费用途是否符合“公共免费信息为使用目的”仍没请律师看 |
| 6 | 托管服务请求 SEC 时 User-Agent 写谁的联系方式 | 维护者 | 用项目的公共联系邮箱，不用个人邮箱 |
| 7 | 托管服务若给别人用 FRED，FRED 要求公开隐私政策、展示 FRED 条款链接；纽约联储要求用户承担补偿责任。对外服务是否要写自己的服务条款和隐私政策 | 维护者，可请律师看 | 第 1 条改直连后，开源包不用；托管服务继续对外就要写 |
| 8 | `data.stats.gov.cn` 自己有没有另外的条款 | 维护者在境内网络打开看一眼 | 有的话补进第 5 节 |

## 7. 这次没核对的

- 财政部官网（收益率曲线 CSV）、TIC、Federal Register 的条款。它们是联邦政府作品，一般没有版权，但没读原文。
- `data.stats.gov.cn` 的页面（见第 6 节第 8 条）。
- FRED API 第 2 版的文档。
- which-api 仓库里托管服务的代码。这次的会话没有它的访问权限，托管服务的现状用的是发布前清单 2026-11-15 的记录。
