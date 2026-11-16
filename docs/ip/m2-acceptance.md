# M2 真实验收手册

> 用真实的模型、Slack 工作区和 Codex，把 M2 验收一遍（[《M2 规格》](m2-spec.md) 的 M2-5、M2-6）。M0 和 M1 一直没做的真实验收，也一并做掉。
> 分工：维护者在自己的云主机上按步骤做，每步对照“应该看到”打勾；有不对的地方，把对应的输出贴给 Claude，由 Claude 修好再推 PR。

## 0. 准备

**主机**：Linux x86_64，装有 Node.js 18.17 以上（宏观研究员的取数工具 `macro-mcp/` 在本机启动），能访问 `slack.com`、`api.openai.com`、各官方数据源和 `whichapi-macro.onrender.com`。

> **也可以用 Docker**：按[《用 Docker 部署》](deploy-docker.md)起服务，不用在主机上装 Rust。
> - 下面的 `./target/release/ip <命令> --config ip.yaml` 都换成 `docker compose run --rm ip <命令> --config /srv/ip/ip.yaml`；
> - `ip serve` 换成 `docker compose up -d`；
> - 密钥写进 `rust/.env`，不用 `export`；
> - 第 5 步构建时在 `.env` 里写 `INSTALL_CODEX=true`，再按下面的提示，把 `ops-demo` 拷进数据卷。

```bash
sudo apt-get install -y bubblewrap git build-essential
```

Ubuntu 24.04 默认不允许普通用户建命名空间，bubblewrap 需要放开：

```bash
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
```

```bash
git clone https://github.com/GgMacroFinance-dotcom/Intelligent-Position-release.git
```

```bash
cd Intelligent-Position-release/rust && cargo build --release --locked
```

**账号和密钥**（只放环境变量，不写进 `ip.yaml`）：

| 用途 | 环境变量 | 从哪里拿 |
| --- | --- | --- |
| 模型（OpenAI：协调员用 `gpt-6-astra`，执行岗位用 `gpt-6-luna`） | `OPENAI_API_KEY` | OpenAI 平台 |
| WhichAPI 的 key（宏观研究员取 FRED 数据；没有时其余数据照常） | `WHICHAPI_API_KEY` | 现在是 Render 上 whichapi-macro 服务 Environment 页里的 `MACRO_API_KEY`。设了以后取消 `ip.yaml` 里 `mcp_servers.macro.env_from` 的注释 |
| Slack 机器人令牌（`xoxb-`） | `SLACK_BOT_TOKEN` | 用 `examples/slack/manifest.yaml` 建应用，“Install App” 后复制 Bot User OAuth Token |
| Slack 应用级令牌（`xapp-`） | `SLACK_APP_TOKEN` | 应用的 “Basic Information” → “App-Level Tokens”，权限选 `connections:write` |
| Codex（只有第 5 步要） | `CODEX_API_KEY` | OpenAI 平台 |
| 控制台访问令牌（监听非本机地址时） | `IP_TOKEN` | 自己生成，比如 `openssl rand -hex 24` |

**`ip.yaml`**：从 `ip.example.yaml` 复制一份，然后改三处：

1. 取消 `channels.slack` 的注释。`allow.channels` 先只填一个测试频道的 id；频道 id 在频道详情的最下面。
2. 核对 `endpoints.openai.prices` 里的价格（示例里是 2026-11-14 按 OpenAI 的模型文档核对的，含缓存命中和缓存写入价），不对就按官网改；费用和每日上限都按它算。协调员用的 `gpt-6-astra` 比执行岗位的 `gpt-6-luna` 贵约 100 倍，示例给它设了每日 10 美元的上限。
3. 第 5 步再配 `engines`，前面几步先不用。

在测试频道里输入 `/invite @ip`，把机器人拉进来。

## 1. 自检

```bash
./target/release/ip check --config ip.yaml
```

**应该看到**：每项都是 ✓。`数据源 public` 在拉取数据前会是 !，属正常，第 3 步拉完后再跑一次。

| 看到 | 说明和处理 |
| --- | --- |
| ✗ 端点 openai：key 无效 | key 填错了，或者已经过期 |
| ✗ MCP 服务器 macro：连接 MCP 服务器 macro 失败 | 没装 Node.js（要 18.17 以上），或者不是在 `rust/` 目录下启动的（示例配置里的路径是 `../macro-mcp/...`）。不修也能启动服务，只是研究员暂时用不了宏观数据工具；服务会在后台重连 5 次（约 15 分钟内），都失败就修好后重启 |
| ✗ 工具沙箱 | 没装 bubblewrap，或者第 0 步的 sysctl 没做 |
| ✗ 渠道 slack：机器人令牌不能用 | `SLACK_BOT_TOKEN` 不对，或者应用没装进工作区 |
| ✗ 渠道 slack：应用级令牌不能用 | `SLACK_APP_TOKEN` 不对，或者应用没打开 Socket Mode |

- [ ] `ip check` 全部 ✓（`数据源 public` 除外）

## 2. M0：服务和控制台

先启动服务。它占着这个终端，后面的命令另开一个终端执行：

```bash
./target/release/ip serve --config ip.yaml
```

浏览器打开 `http://127.0.0.1:7878/`。如果服务在远程主机上，用 SSH 端口转发：

```bash
ssh -L 7878:127.0.0.1:7878 <主机>
```

| 步骤 | 应该看到 |
| --- | --- |
| 在控制台给“小宏”发：`点评一下本月 CPI` | 回复是逐字流出来的；轮次结束时显示这一轮的费用 |
| 发：`在工作目录里运行 uname -a 和 ls /`，再发 `运行 curl https://example.com` | 第一条能看到 shell 调用和结果；第二条应该失败，因为沙箱默认断网 |
| 给“小宏”发：`美国 10 年期国债收益率最近一周怎么走` | 能看到 `mcp__macro__get_fred_series`（或 `macro_snapshot`）的调用和结果；回答里的数字带观测日期，和工具结果一致 |
| 在“搜索”里搜一个之前出现过的中文词 | 能搜到那条消息 |
| 另开终端，跑下面三项性能基准 | 每项都是 `"pass":true` |

```bash
./target/release/ip bench startup --check
```

```bash
./target/release/ip bench store --check
```

```bash
./target/release/ip bench agents --check
```

- [ ] M0 验收通过

## 3. 数据

```bash
./target/release/ip data pull --config ip.yaml
```

**应该看到**：命令正常结束，没有要拉的序列。示例配置默认不开 `public` 数据源：美国、全球和中国国家统计局的数据都走 Finance Macro Data API 的工具。要定时拉 FRED 序列时，在 `ip.yaml` 里打开 `public`，序列写进 `examples/public-data/catalog.yaml`，拉完再跑一次 `ip check`，`数据源 public` 应该是 ✓。

- [ ] `ip data pull` 正常结束（示例配置下没有要拉的序列）

## 4. M2-5：Slack 端到端

以下都在测试频道里做，`@ip` 是机器人的名字。

| 步骤 | 应该看到 |
| --- | --- |
| 1. 发 `@ip 点评一下本月 CPI` | 机器人在这条消息的线程里回复。回复来自宏观研究员，数字注明来源和数据截至日期 |
| 2. 在同一线程里接着说 `再看看 PPI`（不用 @） | 还是宏观研究员在线程里回复，不会重新分派 |
| 3. 另起一条：`@ip 下周一投资例会要一页材料：这周国内物价和工业品价格怎么走` | 这句话不命中任何路由规则，交给协调员（`fallback`）。协调员派活给宏观研究员（派活的对话在控制台“任务”页可见），研究员写好报告并交回，协调员在 Slack 线程里给出汇总。报告文件在研究员工作目录的 `reports/` 下。注意不要用含 CPI、PPI、研报等词的说法，否则会命中规则“宏观数据”，直接交给研究员 |
| 4. 另起一条含糊的需求，比如 `@ip 帮我准备一下下周的会` | 协调员多半会发一张选择卡片（`ask_human`），带按钮，推荐项高亮 |
| 5. 让另一位同事点卡片上的按钮 | 卡片不变；那位同事看到一条只有他自己能看见的提示：“只有提需求的人能选这张卡片” |
| 6. 自己点按钮 | 原卡片的按钮消失，改成“<你的名字>（slack:U…） 选了「…」”，工作接着进行 |
| 7. 在 `allow.channels` 以外的频道 @ip | 没有任何反应 |
| 8. 停掉服务再启动，然后在第 1 步的线程里接着问一句 | 还是宏观研究员回复：会话绑定存在数据库里，重启后还在 |
| 9. 打开控制台“岗位”页 | 看得到各岗位今天的花费 |

- [ ] 第 1–9 步都符合
- [ ] 回复里的 Markdown（标题、列表、表格）在 Slack 里显示正常；如果显示成纯文字，说明工作区不认 `markdown` 块，代码已退回纯文字发送，记下来即可

## 5. M2-6：Codex 跑运维测试

```bash
npm install -g @openai/codex
```

```bash
codex --version
```

```bash
cp -r examples/ops-demo /tmp/ops-demo
```

用 Docker 时，镜像里的 `examples/` 是只读的，要先把 `ops-demo` 拷进数据卷，后面的 `workspace` 改成 `/srv/ip/data/ops-demo`：

```bash
docker compose exec ip cp -r /srv/ip/examples/ops-demo /srv/ip/data/ops-demo
```

在 `ip.yaml` 里改两处：把 `agents` 里 `ops-1` 那一行换成下面这行（指向要维护的项目），再加上 `engines`：

```yaml
agents:
  - { id: ops-1, role: ops-tester, name: 小维, workspace: /tmp/ops-demo }
engines:
  ops-tester:
    kind: codex
    api_key_env: CODEX_API_KEY
    turn_timeout_secs: 1800
```

再跑一次 `ip check`，确认 `岗位 ops-tester 的 Codex 引擎` 是 ✓，然后重启服务。

主机只能经代理出网时：harness 启动 Codex 时会清空环境变量，`HTTPS_PROXY` 带不过去，Codex 连不上。可以让 `command` 指向一个小脚本，先 `export HTTPS_PROXY=...`（需要的话再加 `SSL_CERT_FILE`），再 `exec codex "$@"`。能直接出网的主机不用管这一条。

| 步骤 | 应该看到 |
| --- | --- |
| 在 Slack 里发 `@ip ops-demo 的测试报错了，帮忙修一下` | 交给运维测试岗位（规则“线上故障”命中“报错”）。在控制台打开这个对话，能看到 `codex.shell` 跑测试失败 → `codex.edit` 改文件 → 再跑测试通过。Slack 线程里收到结论 |
| 看 `/tmp/ops-demo` | `growth.sh` 被修好了；`reports/` 下有报告 |
| 在同一线程里追问 `改了哪一行？` | Codex 接着原来的会话回答，不用从头再看一遍 |

**如果这一步失败**：命令行参数和输出解析在 2026-11-14 用 `codex-cli 0.169.0` 跑通过（[记录](acceptance-2026-11-14-codex.md)），换了 Codex 版本后可能对不上。把下面三样贴给 Claude：

1. `codex --version` 的输出；
2. 控制台里那条“这轮工作失败”的消息；
3. 下面这条命令的输出：

```bash
cd /tmp/ops-demo && codex exec --json --skip-git-repo-check "运行 sh test.sh" 2>&1 | head -40
```

- [ ] Codex 跑通修 bug 的全过程，报告已写出，任务交回

## 6. 结果交回

把每节的勾选结果贴到 GitHub issue 或对话里。任何一步不符合，附上三样东西：

- 那一步的截图或输出；
- 服务的日志（`ip serve` 的终端输出，用 `RUST_LOG=info` 启动）；
- `ip check` 的输出。

Claude 按这些修好，推 PR，再请你复验那一步。全部通过后，Claude 在规格里把 M2-5、M2-6 标为已完成。
