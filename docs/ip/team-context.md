# 接团队上下文系统（TCMS）

> 2026-11-23 写。对应 [`backlog.md`](backlog.md) 的 D-5，也是团队上下文系统（Team Context Management System，简称 TCMS，仓库 `Team-context-management-system`）分步实现的第 6 步。
> 状态：I.P. 这一侧的代码已写，**只用假模型测过**。TCMS 用的是测试里的假服务和本机起的真实 `tcms serve`。没用真实模型跑过，没在 Docker 部署里和 TCMS 一起跑过。

## 1. 是什么

TCMS 是一个独立的 MCP 服务，放团队共享的上下文：规则、事实、术语、流程。正文是 git 里的 Markdown，人批准过才生效。I.P. 是它的第一个用户。它取代了 D-5 原来的设计（[`company-context-design.md`](company-context-design.md) 里部署内置知识库和 `kb_*` 工具那一套），所以 I.P. 里不再做知识库。

岗位记忆（[`context-memory-design.md`](context-memory-design.md)，D-1）先留在 I.P. 里不动。等 TCMS 的晋升接口做好以后，再考虑把值得共享的记忆提给 TCMS。

## 2. 怎么配

先在 TCMS 那边给每个要接的岗位发一个令牌：`tcms token add --name "I.P. 宏观研报" --role macro`。令牌决定这个岗位能看哪些范围，明文只显示这一次，存进环境变量。

然后在 `ip.yaml` 里给这个岗位配一项 `mcp_servers`，再在 `team_context` 里写上它：

```yaml
mcp_servers:
  tcms-macro:
    url: http://127.0.0.1:7341/mcp           # tcms serve 的默认地址；不是本机地址时必须是 https
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
      # team: research         # TCMS 里的团队，可以不写；写了要和令牌一致
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

没测：

- 真实模型会不会照前缀里的规则做事，会不会用 `context_read`、`context_propose`。这是 TCMS 的第 7 步（评测），要花费用，开工前问维护者。
- Docker 部署里和 TCMS 一起跑。`docker-compose.yml` 里还没有 TCMS 的服务。
