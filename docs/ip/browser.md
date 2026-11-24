# 给岗位配浏览器（Playwright MCP）

> 2026-11-24 写。对应 [`backlog.md`](backlog.md) 的 E-5，决定见 [M2 规格](m2-spec.md) D-39。
> 状态：本机（macOS，Google Chrome，`@playwright/mcp` 0.0.86）经 `ip-mcp` 用真的 Playwright MCP 跑过第 5 节的测试：只交出白名单里的工具，打开本机网页、读快照、点链接、页内查找都通，`file://` 和元数据地址被挡住。没让模型用过它（真模型、假模型都没有），没在 Linux 主机和 Docker 里跑过。现在没有哪个岗位要用浏览器，示例配置里这一段是注释掉的。

## 1. 为什么用 Playwright MCP

I.P. 不自己写浏览器自动化。需要上网页查东西、填表的岗位，在 `mcp_servers` 里配一项 [Playwright MCP](https://github.com/microsoft/playwright-mcp)（微软，Apache-2.0），员工就有了 `mcp__browser__browser_navigate` 这类工具。它把页面转成可访问性树的文字快照交给模型，元素带编号（`ref=e3`），模型照编号点击、输入，不用看截图，便宜的模型也用得了。

原来占位的 `ip-browser` crate（“共享一个 Chrome、每岗位一个上下文、截图预处理”）一直是空的，这次删掉了。以后 Playwright MCP 不够用、真要自己写，再按那时的需求重新加。

## 2. 先装好

- Node.js 20 以上（`@playwright/mcp` 0.0.86 要求）。
- 浏览器：默认用 Google Chrome（`--browser chrome`）。主机上没有的，用 Playwright 自带的安装命令装，比如 `npx -y playwright@1.66.0-alpha-2026-11-18 install --with-deps chrome`（版本跟 `@playwright/mcp` 依赖的那一版走）。这一步只在 macOS 上用现成的 Chrome 试过，Linux 上的装法没验。
- 给浏览器一个单独的目录当工作根目录（下面的 `cwd`），要先建好，比如 `mkdir -p data/browser/web-researcher`。相对路径和 `args` 一样，以启动 `ip serve` 时所在的目录为准。不要用员工的工作目录，也不要用 `data_dir` 本身。

## 3. 怎么配

```yaml
mcp_servers:
  browser:
    command: npx
    args:
      - -y
      - "@playwright/mcp@0.0.86"   # 写死版本，升级时先跑第 5 节的测试
      - --headless
      - --isolated                 # 浏览器资料只放内存，不留登录状态和 cookie
      - --no-webmcp                # 不让网页自己往工具列表里加工具
      - --image-responses
      - omit                       # 不回传图片；ip-mcp 也只把文字交给模型
      - --blocked-origins=http://169.254.169.254;http://metadata.google.internal
    cwd: ./data/browser/web-researcher   # 浏览器能读写的唯一目录，要先建好
    roles: [web-researcher]              # 一个岗位一项，见第 4 节
    tools:                               # 只交这些工具给员工，见下面
      - browser_navigate
      - browser_navigate_back
      - browser_snapshot
      - browser_find
      - browser_click
      - browser_type
      - browser_fill_form
      - browser_select_option
      - browser_press_key
      - browser_hover
      - browser_wait_for
      - browser_tabs
      - browser_handle_dialog
      - browser_close
    timeout_secs: 90                     # 打开页面默认最多等 60 秒，留些余量
```

`tools` 是 `mcp_servers` 每一项都能写的白名单（D-39）：写服务器上的原名，不在单子里的工具员工看不到、也调不了，Playwright MCP 以后新加的工具也一样，要用得自己加进来。省略时交出服务器列出的全部工具，所以配 Playwright MCP 一定要写。

0.0.86 一共有 25 个工具，上面留了 14 个，工具定义从约 22 KB 降到约 8.6 KB。去掉的和原因：

| 工具 | 为什么不给 |
| --- | --- |
| `browser_run_code_unsafe` | 在 Playwright MCP 自己的 Node 进程里执行任意代码，官方说明写的是等同远程执行。这个进程不在沙箱里（第 4 节），给了就等于把主机交给模型 |
| `browser_evaluate` | 在网页里执行任意 JavaScript。不是越权，但用不着，留着让模型更容易被网页内容带偏 |
| `browser_file_upload`、`browser_drop` | 把本机文件交给网页，能把目录里的东西传出去 |
| `browser_take_screenshot` | 图片不回传，截图只落在浏览器目录里，模型看不到 |
| `browser_network_requests`、`browser_network_request` | 能看到请求头和正文，用不着 |
| `browser_console_messages`、`browser_resize`、`browser_emulate_media`、`browser_drag` | 用不着，省工具定义 |

`browser_navigate` 和点击的结果里只有页面地址、标题和一个快照文件名，快照文件在浏览器目录里，员工读不到。岗位说明里要写明：打开页面后用 `browser_snapshot` 看整页，或用 `browser_find` 找某段文字和元素编号（更省）。快照过长时按内置工具的规矩截到 64 KB。

## 4. 风险和限制

**进程不在沙箱里。** stdio 的 MCP 服务器是 harness 直接起的子进程，权限和 harness 一样（`ip-mcp` 模块说明和 M2 规格第 1 节都写了）。I.P. 这一侧能做的是：子进程只带 `PATH`、`HOME` 等几个基本变量，模型和渠道的密钥不交给它；`tools` 白名单挡住执行代码和传文件的工具。Playwright MCP 自己默认只让工具读写工作根目录（`cwd`）、挡住 `file://`，但它的说明也写了这只是防误操作，不是安全边界。

**浏览器能连到主机能连到的地方。** 包括本机和内网的服务（控制台、TCMS、数据库、中转站管理页）和云主机的元数据地址。`--blocked-origins` 挡住了常见的元数据地址，但 Playwright MCP 写明它不是安全边界、不管重定向。所以：

- 云主机上把元数据服务改成要令牌的版本（AWS 用 IMDSv2，跳数限 1），不要指望 `--blocked-origins`。
- 本机和内网的服务都要鉴权。I.P. 控制台本来就要令牌；别的服务没有鉴权的，不要和要用浏览器的岗位放在同一台主机上。
- 网页内容会原样进模型，网页里写的“忽略之前的指示”这类话会影响员工。要用浏览器的岗位不要同时给它危险的权限（比如能改服务器的外部引擎），员工要做有后果的事照常发审批卡片。

**不留登录状态。** `--isolated` 每次启动都是干净的浏览器，cookie 只活到服务重启，或者闲置一小时、浏览器被 Playwright MCP 关掉为止。要登录的网站先不接；真要接，先想清楚账号权限和密码放哪，再定。

**同一项配置共用一个浏览器。** 一个 stdio 服务器只有一个浏览器上下文，这一项给了的员工共用标签页。所以一个岗位配一项（`roles` 只写一个岗位）；一个岗位有几个员工同时上网时，标签页会互相干扰，可以让员工用 `browser_tabs` 开自己的标签页，现在没有更好的办法。

**Docker 部署里还没有浏览器。** 镜像里有 Node.js 20，没有浏览器。打算的做法是把浏览器放进单独的容器、只连一个和数据库、TCMS 隔开的网络，见 backlog E-16。

## 5. 怎么验证

`crates/ip-mcp/tests/playwright.rs` 用真的 Playwright MCP 按第 3 节的配置跑一遍：工具列表正好是白名单、别的岗位看不到、`browser_run_code_unsafe` 调不了、打开本机起的测试页、读快照、照编号点链接、`browser_find` 找到第二页的内容、`file://` 和元数据地址被挡住。要装好 Node.js 和浏览器，所以 CI 不跑，没设变量时直接跳过：

```sh
cd rust
IP_TEST_PLAYWRIGHT_MCP="npx -y @playwright/mcp@0.0.86" cargo test -p ip-mcp --test playwright -- --nocapture
```

升级 `@playwright/mcp` 时先跑这个测试：工具改了名或删了，白名单那条会失败；新加的工具不会自动交出去，看一眼它的说明再决定要不要加。
