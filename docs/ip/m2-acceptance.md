# M2 真实验收手册

> 对应 [backlog A-5](backlog.md) 和 [《M2 规格》](m2-spec.md) 的 M2-5、M2-6。用自己的 Linux 主机、真实模型和数据、真实 Slack 测试工作区、真实 Codex，独立复跑 M0、M1、M2。
> 按第 0–6 节顺序做。每步记「通过 / 失败 / 受阻 / 未跑」：受阻要写缺什么。用假服务、只在控制台绕过 Slack，或取数只能用示例数据时，分别记录实际通过的部分。

## 0. 准备

- [ ] **主机。** 选一台独立 Linux x86_64 测试机或 Linux 虚拟机。原生运行需要 Git、Rust（仓库声明最低 1.85）、C/C++ 构建工具、Node.js 18.17 以上、bubblewrap 和 PostgreSQL 14 以上。Docker 路线按[部署说明](deploy-docker.md#要求)准备 Docker Engine 和 Compose，数据库由 Compose 启动。两条路线都要让普通用户能建用户命名空间；Ubuntu 24.04 遇到限制时，按部署说明在专用测试机上处理。保留 `sandbox.mode: bubblewrap` 与 `sandbox.network: false`。
- [ ] **网络。** 从服务实际运行的位置确认能访问 OpenAI、Slack（包括 Socket Mode 的 WebSocket）、WhichAPI 和要用的官方数据源。记下直连或代理。国家统计局在境外不一定能直连，准备一个能取数的环境，或按第 4 节核对美国数据并记录差异。
- [ ] **账号。** 真实 OpenAI 账号可用示例中的 `gpt-6-astra` 和 `gpt-6-luna`，并为 Codex 准备可用的 API 凭证。Slack 准备一位同事、两个测试频道（一个在准入名单内、一个在名单外）和一个专用测试应用。用 [Slack manifest](../../rust/examples/slack/manifest.yaml) 建并安装应用，确认 Socket Mode、事件订阅、交互和 bot 权限已启用。在两个频道都把机器人拉进去，这样第 4 节才能验证准入规则。
- [ ] **凭证。** 原生运行把凭证注入启动终端的环境；Docker 把凭证填到被 Git 忽略的 `rust/.env`。配置文件里只写变量名。

| 环境变量 | 用途 |
| --- | --- |
| `OPENAI_API_KEY` | 真实协调员和宏观研究员 |
| `WHICHAPI_API_KEY` | FRED 真实数据；使用时取消 `mcp_servers.macro.env_from` 的注释。拿不到时仍可测其他数据源，FRED 项记受阻 |
| `SLACK_BOT_TOKEN` / `SLACK_APP_TOKEN` | 安装应用后的 Bot User OAuth Token（`xoxb-`）/ 具备 `connections:write` 的 App-Level Token（`xapp-`） |
| `CODEX_API_KEY` | 第 5 节的真实 Codex |
| `IP_TOKEN` | Docker 或监听非回环地址时需要；用 `openssl rand -hex 24` 生成，取消配置中 `auth_token_env: IP_TOKEN` 的注释 |
| `IP_DATABASE_URL` / `IP_TEST_DATABASE_URL` | 原生运行的独立验收 PostgreSQL / 基准库。基准账号须能在验收库中建临时 schema；可与前者共用一个专用数据库 |
| `POSTGRES_PASSWORD` / `IP_DB_PASSWORD` | Docker PostgreSQL 管理员 / 应用账号。按部署说明生成并填入 `.env`，Compose 自动设置 `IP_DATABASE_URL` |

在一个全新的检出目录里运行：

```bash
git clone https://github.com/GgMacroFinance-dotcom/Intelligent-Position-release.git
cd Intelligent-Position-release/rust
git rev-parse HEAD
cp ip.example.yaml ip.yaml
```

记下提交号、系统版本、CPU/内存、部署方式和 PostgreSQL 版本。编辑 `ip.yaml`：开启 `channels.slack`，`allow.channels` 只加一个测试频道 ID，示例里的 `allow.users` 换成你和同事的 ID，保留 `approvers: requester`，保持按路由分派。核对示例的模型端点、价格和每日预算。定时任务保持关闭。第 5 节开始前再加 `engines`。

- **原生：** 在 Ubuntu/Debian 上可用 `sudo apt-get install -y git build-essential cmake pkg-config bubblewrap util-linux curl` 准备系统工具。Rust 和 Node.js 装好后，按 [Rust 快速开始](../../rust/README.md#快速开始)启动专用 PostgreSQL（已有 PostgreSQL 时，用独立验收库），设置上表两个数据库变量。确认 `bwrap --version`、`prlimit --version`、`node --version` 能运行，然后执行 `cargo build --release --locked -p ip-cli`。后续命令均在 `rust/` 下运行。
- **Docker：** 在 `rust/` 执行 `cp deploy/env.example .env`，填变量并设置 `INSTALL_CODEX=true` 和 `RUST_LOG=info`，再运行 `docker compose build ip`。建议为这轮设置独立的 `COMPOSE_PROJECT_NAME`，用新数据卷。控制台映射到本机 7878；先确认没有其他实例占用。

## 1. 自检

```bash
# 原生
./target/release/ip check --config ip.yaml
# Docker
docker compose run --rm ip check --config /srv/ip/ip.yaml
```

- [ ] **通过：** 退出码为 0，配置、PostgreSQL、OpenAI、工具沙箱、宏观 MCP 和 Slack 均为 ✓。默认配置关闭 `public` 数据源；只在开启它且尚未拉取的数据项上容许提示。若沙箱或上限未启用，记为相应项受阻，不能用关闭沙箱后的成功替代。保留全量脱敏输出。
- [ ] **遇到卡点：** 先按失败项定位：连库失败就核对连接串和 schema 权限；MCP 连不上就核对 Node、`rust/` 路径和凭证；Slack 令牌错误就核对应用安装、Socket Mode 和令牌权限。自检通过只表明连通性，模型调用、取数和收发消息仍要在后面验证。

## 2. M0：服务和控制台

```bash
# 原生：这个终端保留服务输出，命令另开终端跑
RUST_LOG=info ./target/release/ip serve --config ip.yaml
# Docker
docker compose up -d
docker compose ps
docker compose logs --tail=100 --timestamps ip
```

- [ ] **服务：** 在服务主机执行 `curl -fsS http://127.0.0.1:7878/healthz`，得到 `"status":"ok"`。用浏览器打开 `http://127.0.0.1:7878/`；远程主机先运行 `ssh -L 7878:127.0.0.1:7878 <主机>`。若设置了访问令牌，用 `?token=<IP_TOKEN>` 登录一次。
- [ ] **流式回复：** 在控制台选“小宏”，发 `点评一下本月 CPI`。回复逐步出现，轮次正常结束并显示费用。
- [ ] **Shell 与断网：** 请小宏运行 `uname -a` 和 `ls /`，在控制台展开工具事件，确认命令真正执行成功。然后请它运行 `timeout 8s bash -c 'exec 3<>/dev/tcp/example.com/443'`。先在沙箱外同一运行环境执行这条命令作为正对照；Docker 路线在这条命令前加 `docker compose exec -T ip`。沙箱外成功、员工的 shell 工具内连接失败才算通过。镜像没有 `curl`，用这个 TCP 探测；正对照失败、命令没执行或工具缺失均记受阻。
- [ ] **真实取数：** 问 `美国 10 年期国债收益率最近一周怎么走？请用 FRED DGS10，不要用示例数据。` 展开真实取数工具 `mcp__macro__get_fred_series` 的结果，抽查回答中的数值、单位和观测日期与工具一致，并记录来源。工具拿不到数时确实应回复“未取得”，但这一项仍记受阻。
- [ ] **中文搜索：** 用“搜索全部消息”找一个刚才用过的中文词，要能找到对应的历史消息。
- [ ] **性能：** 跑 `startup`、`store`、`agents`，分别记下 JSON 和退出码，每项需有 `"pass":true` 且退出码为 0。基准会创建临时 schema，用测试库。Docker 默认应用账号没有建 schema 的数据库权限，先在这轮独立 Compose 的 PostgreSQL 中建一个验收库（只需执行一次）。

```bash
# 原生：在同一终端事先设置 IP_TEST_DATABASE_URL
for name in startup store agents; do
  ./target/release/ip bench "$name" --check
  printf '%s 退出码：%s\n' "$name" "$?"
done

# Docker：在本轮独立 Compose 环境中建基准库
docker compose exec -T postgres createdb -U postgres -O ip_app ip_bench
for name in startup store agents; do
  docker compose exec -T ip sh -c 'IP_TEST_DATABASE_URL="${IP_DATABASE_URL%/*}/ip_bench" exec ip bench "$1" --check' sh "$name"
  printf '%s 退出码：%s\n' "$name" "$?"
done
```

## 3. 数据

```bash
# 原生
./target/release/ip data pull --config ip.yaml
# Docker
docker compose run --rm ip data pull --config /srv/ip/ip.yaml
```

- [ ] **通过：** 默认配置输出“没有写了 fetch 的序列”并正常退出。若专门开启了 `public` 和要拉取的 FRED 序列，预期看到对应序列拉取成功，再跑第 1 节确认 `数据源 public` 为 ✓。这一项检查数据拉取命令；真实数据内容以上一节的工具结果为证据。

## 4. M2-5：Slack 端到端

只在测试频道验收。发送 `@ip` 时用 Slack 的提及选择器选中机器人。在准入频道先跑正向流程，再验证名单外频道。每步记结果。

| 步骤 | 怎么跑 | 通过标准 |
| --- | --- | --- |
| 4.1 | 发 `@ip 点评一下本月 CPI` | 宏观研究员回在原线程，数字注明来源和观测日期。统计局取不到时，该来源记受阻；可另发 `@ip 请用 FRED 的美国 CPI 数据写一句点评，注明观测日期和来源`，单独证明 Slack 上真实数据可达 |
| 4.2 | 原线程追问 `再看看 PPI`，不用 @ | 仍是同一研究员、同一对话，未重新分派 |
| 4.3 | 另起一条 `@ip 下周一投资例会要一页材料：这周国内物价和工业品价格怎么走` | 走默认路由，由协调员派活给研究员，控制台“任务”可查，研究员交回报告和摘要，协调员在原线程汇总。报告在 `data/workspaces/macro-1/reports/`（Docker 为 `/srv/ip/data/workspaces/macro-1/reports/`）。如国内数据受阻，可另试“这周美国国债收益率和资金面怎么走”，避免带上触发直派的“CPI”“研报”等词，并记录改了输入 |
| 4.4 | 另起一条 `@ip 帮我准备一下下周的会`，信息不足时可明确请它先发选择卡片 | 出现有按钮、推荐项高亮的 `ask_human` 卡片；仅有文字追问不算通过 |
| 4.5 | 让同事先点卡片 | 原卡片和任务状态不变，同事收到仅自己可见的提示 |
| 4.6 | 由你本人点卡片 | 原按钮撤下，卡片显示谁选了哪项，工作继续 |
| 4.7 | 在名单外且机器人已加入的测试频道 @ 它 | 无回复，也未新建对话或任务 |
| 4.8 | 重启同一实例并保留同一数据库，在 4.1 的线程追问 | 接续原对话，仍由同一研究员回复。原生在服务终端用 Ctrl-C 后重跑第 2 节的启动命令；Docker 用 `docker compose restart ip` |
| 4.9 | 进控制台“岗位”页 | 页面显示各岗位当天费用，和本轮真实调用对应 |
| 4.10 | 查看 Slack 回复的标题、列表和表格 | Markdown 正常显示。若 Slack 退回纯文字，记录工作区和样式差异，单独判断收发链路是否通过 |

若 `ip check` 通过但 Slack 完全静默，核对事件订阅、bot 是否入频道，以及是否用真正的 @ 提及；改应用权限后重新安装。代理环境再核对服务日志的 Socket Mode 连接模式、`HTTPS_PROXY` 和 `NO_PROXY`；407 多为代理要求认证，403 应核对代理是否放行 Slack WebSocket 域名。用转发器跑通时在记录里注明。附件上传、私信和私有频道如另测，单独记录通过范围，参考[上次 Slack 验收记录](acceptance-2026-11-16-slack.md)。

## 5. M2-6：Codex 跑运维测试

只让 Codex 修改仓库演示项目的独立副本。每次重跑用一份仍留有原始 bug 的干净副本。

```bash
# 原生：仍在 rust/，把打印出的绝对路径填到 ops-1.workspace
npm install -g @openai/codex
codex --version
ops_demo="$(mktemp -d /tmp/ip-ops-demo.XXXXXX)"
cp -R examples/ops-demo/. "$ops_demo/"
printf '%s\n' "$ops_demo"
(cd "$ops_demo" && sh test.sh)

# Docker：已在第 0 节用 INSTALL_CODEX=true 构建；首次执行
docker compose exec -T ip codex --version
docker compose exec -T ip cp -R /srv/ip/examples/ops-demo /srv/ip/data/ops-demo
docker compose exec -T ip sh -c 'cd /srv/ip/data/ops-demo && sh test.sh'
```

这里 `sh test.sh` **应当先失败**，记录原始失败用例。若初始就全过，换一份干净副本再验。编辑 `ip.yaml`，在现有 `agents` 中给 `ops-1` 设置刚打印的绝对路径（Docker 为 `/srv/ip/data/ops-demo`），保留其余员工。加入：

```yaml
engines:
  ops-tester:
    kind: codex
    api_key_env: CODEX_API_KEY
    sandbox: workspace-write
    network: false
    turn_timeout_secs: 1800
```

- [ ] 重跑第 1 节，看到“岗位 ops-tester 的 Codex 引擎”检查通过，再按 4.8 重启。
- [ ] 在 Slack 发 `@ip ops-demo 的测试报错了，帮忙修一下`。控制台要能看到路由给“小维”，Codex 先跑测试失败、定位并修改 `growth.sh`，再跑测试通过；Slack 原线程收到结论。
- [ ] 在同一副本再次运行 `sh test.sh`，确认现有用例全部通过、测试断言未改，`reports/` 下有写明复现、修改和验证的报告。
- [ ] 在原 Slack 线程问 `改了哪一行？`，确认接续原 Codex 会话并能说清修改。
- [ ] 在控制台“任务”页给“小维”新建一条正式任务，要求复跑 `sh test.sh`、记录结果并调用组织工具交回。任务通过 MCP 进入“已完成”，带有结论和报告路径。与 Slack 线程里的自由对话分别记录。

遇到失败，记录 `codex --version`、控制台的轮次失败提示、触发它的原话和服务日志。需要区分 Codex CLI 与集成故障时，可在同一个测试副本目录单独跑 `codex exec --json --skip-git-repo-check '运行 sh test.sh'`，保留脱敏后的开头和终止事件。仅能经代理出网时，还要核对 Codex 子进程能否使用代理，参考[上次 Codex 验收记录](acceptance-2026-11-14-codex.md)的“代理”项。

## 6. 结果交回

在 A-5 的跟踪问题（项目使用 Linear 时为 INT-5）填一轮结果。每个卡点按下面模板记录；一般问题可另建 issue 并回链，安全问题按 [SECURITY.md](../../SECURITY.md) 私下报。截图、配置片段和日志先去掉密钥、令牌、数据库连接凭证、Slack 私密内容和主机敏感信息。

```text
复跑时间和时区：
提交号 / 系统及架构 / CPU 与内存：
部署方式 / PostgreSQL、Node.js、Codex 版本：
出网方式：直连 / 代理 / 转发器
使用的真实服务与替代服务：
验收结果：0 __；1 __；2 __；3 __；4.1–4.10 __；5 __
各阶段花费和耗时（有记录时）：

卡点标题：[A-5 / 步骤号] 一句话说明
状态：失败 / 受阻
复现：运行目录、命令或输入、脱敏配置、重试次数
预期：
实际：退出码、报错原文、是否影响后续项
证据：脱敏截图或输出、同一时段的 ip serve 日志、ip check 输出
影响和待补条件：
修复关联及复验结果：
```

全程都用真实服务并达到当前手册的通过标准后，才能关闭 A-5。原生用 Ctrl-C 停服务；本轮独立的 Docker 栈可用 `docker compose stop` 停服务，保留日志和数据用于复验。
