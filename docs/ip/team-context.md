# 接团队上下文系统（TCMS）

> 2026-11-23 写。对应 [`backlog.md`](backlog.md) 的 D-5，也是团队上下文系统（Team Context Management System，简称 TCMS，仓库 `Team-context-management-system`）分步实现的第 6 步。
> 状态：I.P. 这一侧的代码已合并（release PR #58）。2026-11-23 用真实模型（`gpt-6-luna`、`gpt-6-astra`）和本机起的真实 `tcms serve` 做过一次评测，见第 7 节。2026-11-23 起 Docker 部署可以带上 TCMS（compose 里的 `tcms` 服务，见 [`deploy-docker.md`](deploy-docker.md)“团队上下文”），只用回声假模型在本机 Docker 里跑通过。2026-11-24 起可以把岗位记忆里对整个团队都成立的内容提给 TCMS（第 8 节），只用假模型测过。

## 1. 是什么

TCMS 是一个独立的 MCP 服务，放团队共享的上下文：规则、事实、术语、流程。正文是 git 里的 Markdown，人批准过才生效。I.P. 是它的第一个用户。它取代了 D-5 原来的设计（[`company-context-design.md`](company-context-design.md) 里部署内置知识库和 `kb_*` 工具那一套），所以 I.P. 里不再做知识库。

岗位记忆（[`context-memory-design.md`](context-memory-design.md)，D-1）还留在 I.P. 里，放进岗位自己的提示词。开了 `share_memory` 的岗位，岗位记忆每有新的一版，就把里面对整个团队都成立的内容作为提议发给 TCMS，人批准以后进团队范围，团队里别的岗位也用得上（第 8 节）。

## 2. 怎么配

先在 TCMS 那边给每个要接的岗位发一个令牌：`tcms token add --name "I.P. 宏观研报" --role macro`。令牌决定这个岗位能看哪些范围，明文只显示这一次，存进环境变量。

然后在 `ip.yaml` 里给这个岗位配一项 `mcp_servers`，再在 `team_context` 里写上它：

```yaml
mcp_servers:
  tcms-macro:
    url: http://127.0.0.1:7341/mcp           # tcms serve 的默认地址；不是本机地址时必须是 https
    # allow_http: true                       # 用 docker compose 的服务名连（http://tcms:7341/mcp）时要写，见 deploy-docker.md
    bearer_token_env: TCMS_TOKEN_MACRO       # tcms token add --role macro 发的令牌
    roles: [macro-researcher]                # 必须只写这一个岗位
    timeout_secs: 30
  # 同机也可以由 I.P. 起子进程：
  # tcms-macro:
  #   command: tcms
  #   args: [serve, --stdio]
  #   env_from: { TCMS_TOKEN: TCMS_TOKEN_MACRO, TCMS_DATABASE_URL: TCMS_DATABASE_URL }
  #   env_inherit: [TCMS_REPO]
  #   roles: [macro-researcher]

team_context:
  budget_tokens: 4000          # 整个包的 token 预算（TCMS 按字数估），100 到 200000，默认 4000
  roles:
    macro-researcher:          # I.P. 的岗位 id
      server: tcms-macro       # 上面那一项
      role: macro              # TCMS 里的岗位名，要和令牌一致；省略时用 I.P. 的岗位 id
      # team: research         # TCMS 里的团队，可以不写；写了要和令牌一致；开 share_memory 时要写
      # share_memory: true     # 把岗位记忆里对整个团队都成立的提给 TCMS（第 8 节），默认不开
```

启动时会检查：岗位要存在，`server` 要在 `mcp_servers` 里，那一项的 `roles` 必须正好是这一个岗位，预算要在范围内。`roles` 这一条是因为令牌带着岗位的权限，同一项给了别的岗位，别的岗位就能用这个令牌读不该读的东西。几个岗位都要接，就各发一个令牌、各配一项。

## 3. 每一轮做什么

接了团队上下文的员工，每一轮第一次调模型之前，调一次 TCMS 的 `context_get`。参数是岗位、团队（写了才带）、任务描述和预算。返回的包分两部分，放在两个地方：

- **固定前缀**（规则和常驻条目）放进系统提示词，紧跟在组织规则（`org_prompt`）后面，后面再补一句工具在这里叫什么名字。同一个岗位的员工前缀一样，前缀没变时提示词缓存能命中。人在 TCMS 里批准了新的常驻条目，下一轮就带上。
- **按任务挑的部分**（相关条目的全文，加上放不下的条目的标题目录）作为一条用户消息放在对话最前面，开头写明是系统附上的。同一个对话第一次取到以后就不再换，进程里记着，这样对话前面的内容不变，提示词缓存能命中。这条消息不存进库，也不会被上下文压缩掉。进程重启后，按对话里的第一条消息重新取一次，条目在这期间改过的话，内容会和重启前不同。

任务描述取对话里第一条不是这位员工自己写的文字消息（人的请求，或派活时的任务说明），最多 4000 字。对话里找不到的（比如只剩摘要的长对话），用岗位说明代替。

外部引擎（Codex、Claude Code）也一样：前缀放进给引擎的系统提示词，按任务挑的部分只在开新会话的那一轮加在输入前面，续接会话时引擎自己的会话里已经有了。

每一轮都会调一次 `context_get`，因为前缀可能变了。返回的按任务挑的部分只在对话第一次用到，之后的直接丢掉。

## 4. 员工用的工具

`mcp_servers` 里那一项的工具照常交给员工，名字是 `mcp__<服务器>__<工具>`：

- `context_search`：按词找条目。
- `context_read`：按 id 读条目全文。按任务挑的部分里只列了标题的，用它读。
- `context_propose`：发现条目过时、写错了，或者学到了值得记下的东西时，提一条修改，写明理由和证据。人在 TCMS 的审核页或命令行批准以后才写进上下文仓库。

`context_get` 和 `context_changes` 由 harness 调，不交给模型。模型硬要调的话，返回一句说明，让它用 `context_search`。

## 5. 出错时

TCMS 连不上、超时（10 秒）或返回错误时，记一条警告日志，这一轮不带团队上下文，照常进行，下一轮再试。失败的结果不记进缓存，所以对话开头那部分会在下一次取到时补上。TCMS 返回的包里有提醒（`warnings`，比如预算太小放不下）时，也记一条警告日志。

## 6. 检查和基准

- `ip check --config ip.yaml` 会用每个岗位的令牌试取一次包，报前缀和按任务挑的部分各多少 token。令牌和岗位名对不上、服务没连上时报失败。
- `ip bench team --check` 量 I.P. 这一侧每轮取包的开销：MCP 客户端发请求、收一个约 1 万字的包、解析 JSON，跑 1000 轮。服务端是内存管道上的假服务，不含 TCMS 编包的时间，那部分在 TCMS 自己的 `tcms bench serve` 里量。目标是 p95 不超过 5 毫秒，开发机上约 0.2 毫秒。CI 每个 PR 都跑。

## 7. 测了什么，没测什么

测了：

- 单元测试，TCMS 是测试里的假服务。测了前缀进系统提示词、对话开头那部分在同一个对话里不变、隐藏的工具不交给模型、员工能用 `context_search`、TCMS 出错时这一轮照常进行、外部引擎第一轮拿到两部分，还有配置检查的几种错误写法。模型是假的。
- 本机起真实的 `tcms serve`（PostgreSQL 16，造的小上下文仓库），I.P. 用假模型对话：前缀进了系统提示词，对话开头是相关条目和目录，工具列表里没有 `context_get`，`context_search` 搜到了条目，`context_propose` 提的修改在 `tcms proposals` 里看得到。`ip check` 报对了正确的令牌，也报出了岗位名对不上。
- 设了 `IP_TEST_TCMS_URL` 和 `IP_TEST_TCMS_TOKEN` 时，`ip-runtime` 的测试会连真实的 TCMS，搜一次、提一条修改。CI 里不设，跳过。
- 真实模型评测（TCMS 第 7 步，2026-11-23）：10 道必须用到团队上下文才能答对的题，比较不接、接上（预算 4000 token）、小预算（只列标题，员工要自己查）三种配置。`gpt-6-luna` 每种 3 遍，不接 0/30，接上 30/30，小预算 30/30；`gpt-6-astra` 每种 1 遍，不接 1/10（判分误判），接上 10/10，小预算 10/10。两个模型都照前缀里的规则写了声明、写了“广义美元指数”；小预算时问按任务挑的条目的题都自己调了 `context_search`，有时再 `context_read`。合计约 0.48 美元。题目是专门造的，只说明链路通了，不说明真实工作里帮了多少。记录在 [`evals/2026-11-23-team-context/`](evals/2026-11-23-team-context/README.md)，跑法 `rust/evals/team-context/run.sh`。

没测：

- 真实模型用 `context_propose` 提修改。它要人批准，不适合放进自动评测；上面第二条里是假模型提的。
- 真实模型在 Docker 部署里和 TCMS 一起跑。compose 里的 `tcms` 服务 2026-11-23 在本机 Docker 里只用回声假模型跑通过：`ip check` 取到包，团队上下文评测 9/10（和不用 Docker 时一样），提议在审核页批准后写进卷里的上下文仓库（[`deploy-docker.md`](deploy-docker.md) 的实测结果）。

## 8. 把岗位记忆提给团队（2026-11-24）

岗位记忆是一个岗位自己的经验。里面有些东西不只对这个岗位成立，比如数据从哪里取、口径上踩过的坑、报告的格式，团队里别的岗位也用得上。开了 `share_memory` 的岗位，会把这部分作为提议发给 TCMS，人在 TCMS 的审核页批准以后，写进团队范围（`team:<团队>`），团队里的岗位下一轮就带上。这就是 TCMS 的“晋升”（TCMS `docs/spec.md` 第 5 节）里，以 I.P. 岗位记忆为来源的那一种。

怎么配：在 `team_context.roles` 里这个岗位下面写 `team`（TCMS 里的团队名，要和令牌一致）和 `share_memory: true`。这个岗位还要开着岗位记忆（`memory.enabled`，写了 `memory.roles` 时要包括它）。少一样启动时报错。默认不开，因为每有新的一版岗位记忆，要多调一次模型。

什么时候做：后台记忆任务每一轮先提炼、再合并，然后看开了 `share_memory` 的岗位。岗位记忆摘要有了新的一版（合并出来的，或者人在控制台改过的）才做：

1. 调一次模型（`memory.consolidation_model`，没写时用岗位自己的模型），材料是这一版岗位记忆和以前提过的标题。模型挑出对整个团队都成立的，最多 3 条，每条写类型、标题、正文、为什么对团队成立、能作证据的对话编号。挑的规矩写在提示词里：写成惯例的才挑，只出现过一次的不挑；只和这个岗位有关的不挑；对话目录不挑；提过的、换个说法的都不再提；不写数值和密钥。没有合适的就一条不提。
2. 每条用这个岗位的令牌调 `context_propose`：新建，范围 `team:<团队>`。理由写明是哪个岗位的岗位记忆里来的、为什么对团队成立；证据写岗位记忆的哪一版，加上岗位记忆里确实写到了的对话编号。`rule` 只能人写，模型挑成规则的改成 `memory`。正文里的一级、二级标题降成三级，送出去之前照提炼时的规矩脱敏。
3. 记下看过这一版、提了哪些标题（最多记 100 条）。

不会重复花钱：同一版岗位记忆只看一次。模型调成功以后就算看过了，提议没提成（TCMS 出错、这个令牌没处理的提议攒满了 20 条）也不再为这一版调模型，等下一版再说，日志里记一条警告。TCMS 没连上时不调模型，下一轮再试。岗位当天预算所剩不多时也跳过（和合并用同一个门槛）。

不会重复提：提过的标题告诉模型别再提，模型还是提了、标题一样（不算空白、不分大小写）的也不发。TCMS 那边，一样的、还没处理的提议只留一条，团队里已经有相近的条目时列给审核的人对照着看。只是提议，人不批准就不会生效；批准的人可以先改再接受，也可以拒绝。

测了什么，没测什么：

- 单元测试，模型和 TCMS 都是假的：挑出的几条提到 `team:research`，理由、证据、标题降级、脱敏、规则换成经验都对；同一版不再调模型；第二版时告诉模型提过什么，标题一样的不再发；模型输出看不懂时这一版算看过了；没有 MCP 服务器、没开 `share_memory` 时什么都不做；TCMS 拒了提议时记一次失败，不再为这一版调模型。配置检查测了没写团队、没开岗位记忆两种错误写法。
- 本机起真实的 `tcms serve`（PostgreSQL 16，普通账号），I.P. 用假模型跑一轮：`tcms proposals` 里看得到这一条，范围 `team:research`，理由和证据是上面说的样子；`tcms accept` 以后写进了上下文仓库的 `team-research/流程.md`。
- 设了 `IP_TEST_TCMS_URL`、`IP_TEST_TCMS_TOKEN`（团队不是 research 时再设 `IP_TEST_TCMS_TEAM`）时，`ip-runtime` 的测试会连真实的 TCMS 提一条。CI 里不设，跳过。
- 没测：真实模型挑得准不准、挑出来的值不值得进团队，要等有真实的岗位记忆以后看。没有加基准：后台任务，一版岗位记忆调一次，不在每轮的热路径上。
