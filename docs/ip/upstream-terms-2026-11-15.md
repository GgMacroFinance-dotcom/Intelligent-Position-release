# 上游数据源使用条款核对（发布清单 R-3）

> 2026-11-15。读了纽约联储、SEC、国家统计局三家的条款原文（页面地址和读到的日期见各节）。
> 这是工程上的核对，不是法律意见。第 5 节列出的要维护者或律师定。
> 财政部 Fiscal Data 2026-11-12 已读过原文，见[开源前的核对](macro-mcp-open-source.md)第 3 节；财政部官网（收益率曲线）、TIC、Federal Register 这次仍没核对。

## 1. 结论

| 来源 | 开源代码里调用 | 经托管服务转发、缓存 | 署名或声明 | 频率 | 结论 |
| --- | --- | --- | --- | --- | --- |
| 纽约联储参考利率 | 可以，允许自动访问 | 可以，个人或商业用途都许可 | 必须带一段固定的声明和免责；SOFR、BGCR 另有 DTCC 的免责 | 不得影响网站运行，没给数字 | 可以用，**要补声明** |
| SEC（data.sec.gov） | 可以，不要 key | 可以，网站信息是公开信息，可复制再分发 | 请求注明 SEC 为来源（“请考虑”，不是硬性要求） | 每个用户合计每秒 10 次，不论几台机器 | 可以用，**要补限速**；User-Agent 已经要求 |
| 国家统计局 | 网站允许下载和使用统计数据；用的是网站自己的页面接口，条款既没允许也没禁止程序调用 | **要人定**：转载须是“公共免费信息为使用目的”的合理引用，且不得“擅自同意他人继续转载” | 必须在显著位置注明来自国家统计局网站和网址 `www.stats.gov.cn` | 没给数字；禁止探测、扫描、干扰网站运行 | 自己用没问题；**托管转发和商业用途要人定**；要补署名 |

## 2. 纽约联储

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

## 3. SEC

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
2. 托管服务若也提供这个工具，它的 User-Agent 要写真实的联系人和邮箱（第 5 节第 2 条）。
3. README 写明来源 “SEC EDGAR” 和每秒 10 次的限制。

## 4. 国家统计局

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

## 5. 要维护者或律师定的

| # | 事项 | 谁定 | 建议 |
| --- | --- | --- | --- |
| 1 | 国家统计局数据经托管服务转发，尤其收费时，是否符合“公共免费信息为使用目的”和“不得擅自同意他人继续转载” | 律师 | **已定（维护者 2026-11-16）：统一替用户注明来源即可。** 工具返回里带署名“数据来自国家统计局网站（www.stats.gov.cn）”，岗位说明要求报告照写。收费用途是否符合“公共免费信息为使用目的”仍没请律师看 |
| 2 | 托管服务请求 SEC 时 User-Agent 写谁的联系方式 | 维护者 | 用项目的公共联系邮箱，不用个人邮箱 |
| 3 | 纽约联储要求用户承担补偿责任。托管服务对外提供时是否要写自己的服务条款和隐私政策 | 维护者，可请律师看 | 托管服务继续对外就要写 |
| 4 | `data.stats.gov.cn` 自己有没有另外的条款 | 维护者在境内网络打开看一眼 | 有的话补进第 4 节 |

## 6. 这次没核对的

- 财政部官网（收益率曲线 CSV）、TIC、Federal Register 的条款。它们是联邦政府作品，一般没有版权，但没读原文。
- `data.stats.gov.cn` 的页面（见第 5 节第 4 条）。
- which-api 仓库里托管服务的代码。这次的会话没有它的访问权限，托管服务的现状用的是发布前清单 2026-11-15 的记录。
