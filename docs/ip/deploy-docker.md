# 用 Docker 部署

> 一台 Linux 云主机上，用 `docker compose` 起 harness。镜像里带工具沙箱 bubblewrap，可选带 Codex。
> 相关文件都在 `rust/` 下：`Dockerfile`、`docker-compose.yml`、`deploy/env.example`。
> 团队上下文系统（TCMS）是可选的第三个服务，见“团队上下文”一节。

## 要求

- Linux 主机，Docker Engine 24 以上，带 Compose v2.20 以上（`docker compose` 命令；`additional_contexts` 要 2.17，`depends_on` 里的 `required: false` 要 2.20）。
- 主机内核允许普通用户建用户命名空间，工具沙箱靠它隔离。Ubuntu 24.04 起默认不允许，要放开：

  ```bash
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
  ```

  想重启后也生效，就把这一行写进 `/etc/sysctl.d/99-ip.conf`。

## 步骤

以下都在仓库的 `rust/` 目录里执行。

1. **配置**：复制示例配置。

   ```bash
   cp ip.example.yaml ip.yaml
   ```

   打开 `ip.yaml`，先取消 `auth_token_env: IP_TOKEN` 那行的注释（容器里监听 `0.0.0.0`，必须有访问令牌，否则拒绝启动），再按需改岗位、`channels`（Slack）和 `engines`（Codex）。镜像里的工作目录是 `/srv/ip`，所以示例里 `./data`、`./examples/…` 这些相对路径不用改。

   一定要先建好 `ip.yaml` 再执行后面的命令：文件不存在时，Docker 会在那个位置建一个空目录，服务读不到配置就退出。遇到这种情况，删掉那个目录再复制一次。

2. **密钥**：复制示例，填上真实的值。`.env` 已被 git 忽略。

   ```bash
   cp deploy/env.example .env
   ```

   要用 Codex 的话，在 `.env` 里写 `INSTALL_CODEX=true`，构建时会把 Codex 装进镜像。

3. **构建**：

   ```bash
   docker compose build
   ```

4. **自检**：逐项检查配置里的东西能不能用，详见[验收手册](m2-acceptance.md)第 1 节。

   ```bash
   docker compose run --rm ip check --config /srv/ip/ip.yaml
   ```

5. **拉取公开数据**（可选；只有在 `ip.yaml` 里打开了 `public` 数据源、自己登记了 FRED 序列时才需要。宏观研报岗位的美国、全球和中国数据都走 Finance Macro Data API 的工具）：

   ```bash
   docker compose run --rm ip data pull --config /srv/ip/ip.yaml
   ```

6. **启动**：

   ```bash
   docker compose up -d
   ```

   `docker compose ps` 里显示 `healthy` 就说明起来了；看日志用：

   ```bash
   docker compose logs -f
   ```

7. **打开控制台**：`http://127.0.0.1:7878/?token=<IP_TOKEN>`，令牌第一次用后会记在浏览器里。端口只开放给本机；服务在远程主机上时用 SSH 转发：

   ```bash
   ssh -L 7878:127.0.0.1:7878 <主机>
   ```

## 团队上下文（可选）

团队上下文系统（TCMS，[`team-context.md`](team-context.md)）作为 compose 里的 `tcms` 服务起，和 I.P. 共用 `postgres`，用自己的角色 `tcms_app` 和 schema `tcms`。
镜像直接从 TCMS 的公开仓库构建；不开它时 compose 和以前一样，只有 `postgres` 和 `ip`。

1. **开启**：在 `.env` 里写 `COMPOSE_PROFILES=tcms`，并用 `openssl rand -hex 24` 生成 `TCMS_DB_PASSWORD`。

   `TCMS_DB_PASSWORD` 只在 `pg-data` 卷第一次建出来时生效。已经起过、当时没填的，进库里补建角色和 schema：

   ```bash
   docker compose exec -T postgres psql -U postgres -d ip -v pw="<TCMS_DB_PASSWORD>" <<'SQL'
   CREATE ROLE tcms_app LOGIN PASSWORD :'pw';
   GRANT CONNECT ON DATABASE ip TO tcms_app;
   CREATE SCHEMA tcms AUTHORIZATION tcms_app;
   ALTER ROLE tcms_app SET search_path = tcms;
   SQL
   ```

2. **构建、起 tcms**：

   ```bash
   docker compose build tcms
   docker compose up -d tcms
   ```

   上下文仓库在 `tcms-context` 卷里，第一次启动时是空的，会自动 `git init`。

3. **放内容**：把一个 Markdown 目录（比如 Notion、AFFiNE 导出的）导进某个范围，再同步一次索引。范围是 `org`、`team-<团队>`、`role-<岗位>`、`project-<项目>`：

   ```bash
   docker compose run --rm -v "$PWD/资料:/import:ro" tcms import /import --into team-research
   docker compose run --rm tcms reindex
   ```

   以后员工用 `context_propose` 提修改，人在审核页批准。换成自己的 git 仓库等其他办法见 TCMS 仓库的 `docs/deploy-docker.md`。

4. **发令牌**：每个岗位一个，明文只打印这一次，填进 `.env` 的 `TCMS_TOKEN_MACRO`。上下文仓库里还没有 `teams.yaml` 时，用 `--team` 写明岗位属于哪个团队：

   ```bash
   docker compose run --rm tcms token add --name "I.P. 宏观研报" --role macro --team research
   ```

   审核的人用另一种令牌：

   ```bash
   docker compose run --rm tcms token add --reviewer --name "王审核"
   ```

5. **配 `ip.yaml`**：取消 `mcp_servers.tcms-macro` 和 `team_context` 两段的注释，地址改成 compose 里的服务名，并加上 `allow_http: true`。两个容器在同一台主机的 compose 网络里，走 http，不出这台主机：

   ```yaml
   mcp_servers:
     tcms-macro:
       url: http://tcms:7341/mcp
       allow_http: true
       bearer_token_env: TCMS_TOKEN_MACRO
       roles: [macro-researcher]
   ```

6. **检查、启动**：`ip check` 里“团队上下文 macro-researcher”一项是 ✓，就说明取到包了。`ip` 会等 `tcms` 变成 `healthy` 再起。

   ```bash
   docker compose run --rm ip check --config /srv/ip/ip.yaml
   docker compose up -d
   ```

7. **审核页**：`http://127.0.0.1:7341/review`，用审核令牌登录。端口只开放给本机，远程主机上用 `ssh -L 7341:127.0.0.1:7341 <主机>`。

8. **推到自己的 git 远端（可选）**：上下文仓库默认只在 `tcms-context` 卷里。配了远端以后，批准、改单条、导入之前先从远端拉，提交以后推上去；tcms 每 60 秒再同步一次，人在自己的克隆里改了推到远端，就这样进来、进索引。在 GitHub、Gitea 上建一个仓库（空的或者已经放了上下文都行），发一个只对这个仓库有读写权限的令牌（GitHub 用 fine-grained token，Contents 读写），在 `.env` 里写：

   ```bash
   TCMS_REMOTE=https://github.com/<组织>/<仓库>.git
   TCMS_GIT_TOKEN=<令牌>
   # Gitea、GitLab 还要写用户名：
   # TCMS_GIT_USERNAME=<用户名>
   ```

   令牌不要写进地址。`.env` 里的变量 `ip` 容器也读得到，和数据库密码一样；员工的 shell 命令在沙箱里，读不到服务进程的环境变量。

   重新起：`docker compose up -d tcms`。手动同步一次、看结果：

   ```bash
   docker compose exec tcms tcms sync
   docker compose logs tcms | grep 远端
   ```

   卷里有内容、远端是空的时推上去；卷是空的、远端有内容时拉下来；两边都有内容、又不是从同一个仓库来的时，git 不合并，日志里一直报错，先定以哪边为准。两边同时改了同一处、合并不了时，服务器这边的提交推到远端的旁支 `tcms/main-unmerged`，在自己的克隆里合并它、推回 `main`；合并好之前，审核页上的批准会停下来，提议还是待审核。ssh 远端、分支、同步间隔（`TCMS_REMOTE_BRANCH`、`TCMS_SYNC_SECS`）和别的情况见 TCMS 仓库 `docs/deploy-docker.md` 的“推到自己的 git 远端”。

   人推上去以后默认最多等 60 秒才进来。想推上去马上同步：在 `.env` 里写 `TCMS_WEBHOOK_SECRET=$(openssl rand -hex 32)`，重新起 tcms，再在 GitHub、Gitea 或 GitLab 的仓库设置里加一个 webhook，地址 `https://<域名>/hooks/push`，密钥填一样的，只选推送事件。tcms 的端口只给本机，要让远端连得到，在主机的 HTTPS 反向代理里只把 `/hooks/push` 转给 `127.0.0.1:7341`（这个地址靠签名认人，不回数据）。三家各怎么填见 TCMS 仓库 `docs/deploy-docker.md` 的“推上去马上同步”。没在这套部署里试过。

TCMS 的镜像默认从 `main` 构建。`.env` 里的 `TCMS_REF` 可以换成分支或标签；`TCMS_BUILD_CONTEXT` 可以指向本机的一份源码。

## 容器里的路径

| 路径 | 是什么 | 怎么来的 |
| --- | --- | --- |
| `/srv/ip/ip.yaml` | 你的配置 | 从 `rust/ip.yaml` 只读挂进来 |
| `/srv/ip/data` | 员工的工作目录、Codex 的配置目录 | 命名卷 `ip-data` |
| `/srv/ip/examples/public-data` | `ip data pull` 拉下来的公开数据（示例配置默认不开，见 `ip.example.yaml` 的 `public`） | 命名卷 `public-data`；第一次创建时 Docker 会把镜像里的 `catalog.yaml` 拷进去 |
| `/srv/ip/examples/macro-data` 等 | 示例数据和示例项目 | 镜像自带 |

服务在容器里以 `ip` 用户运行，uid 是 10001。

**让运维测试岗位维护主机上的项目**：在 `docker-compose.yml` 的 `volumes` 里加一行，把项目挂进容器：

```yaml
      - /srv/my-project:/srv/ip/projects/my-project
```

再在 `ip.yaml` 里给员工指定工作目录：

```yaml
- { id: ops-1, role: ops-tester, name: 小维, workspace: /srv/ip/projects/my-project }
```

项目目录要让 uid 10001 可写：

```bash
sudo chown -R 10001 /srv/my-project
```

**用 stdio 启动的 MCP 服务器**（`mcp_servers` 里写 `command` 的那种）：命令在容器里执行，镜像里要有它的运行环境。镜像里装了 Node.js（Debian trixie 自带的 20），并带着仓库里的 `macro-mcp/`，放在 `/srv/macro-mcp`。示例配置的 `mcp_servers.macro` 写的是 `../macro-mcp/bin/whichapi-macro.js`，容器的工作目录是 `/srv/ip`，正好指到它，不用改，也不用挂东西。FRED 数据要 WhichAPI 的 key：在 `.env` 里填 `WHICHAPI_API_KEY`，再取消 `ip.yaml` 里 `env_from` 那一行的注释。`.env` 里的变量 ip 都能读到，但不会整个交给 MCP 子进程：子进程只带 `PATH`、`HOME` 等几个基本变量，加上 `ip.yaml` 里 `env_inherit`、`env_from` 写明的。

别的 stdio 服务器要挂代码进来，比如：

```yaml
      - /srv/my-mcp-server:/srv/ip/projects/my-mcp-server:ro
```

`ip.yaml` 里的 `args` 就写容器里的路径。

## 安全上的取舍

`docker-compose.yml` 给容器放开了三项限制：

```yaml
security_opt:
  - seccomp=unconfined
  - apparmor=unconfined
  - systempaths=unconfined
```

原因是工具沙箱 bubblewrap 要在容器里新建用户命名空间、挂一个新的 `/proc`，Docker 默认的三道限制分别会拦住它：

| 选项 | Docker 默认会拦住什么 |
| --- | --- |
| `seccomp=unconfined` | 默认的 seccomp 规则，不让没有特权的进程建新的命名空间 |
| `apparmor=unconfined` | Docker 默认的 AppArmor 规则，不让容器里挂文件系统 |
| `systempaths=unconfined` | Docker 遮住了 `/proc` 下的一些文件；只要 `/proc` 被部分遮住，内核就不让在新的命名空间里再挂一个 `/proc` |

放开这三项后，容器和主机之间的隔离比默认弱一些。服务仍然以普通用户运行，没有任何额外的 capability，也不是 `--privileged`。

**不要改成 `sandbox: { mode: none }`。** 不开沙箱时，员工的 shell 命令和服务进程是同一个用户，能读 `/proc/1/environ`，那里有所有密钥。bubblewrap 给每条命令一个新的进程命名空间，看不到服务进程。

**程序和数据放在 `/srv/ip`，不放 `/opt`。** 工具沙箱会把 `/usr`、`/etc`、`/opt` 只读挂给员工的 shell，放在这三处，每个员工都能读到数据库、`ip.yaml` 和别的员工的工作目录。挂主机上的项目时也一样，挂到 `/srv/ip/projects/` 下面。

缺了哪一项，`ip check` 的“工具沙箱”一行和服务启动时都会直接报错，并指向这一节。

Codex 自带一份 bubblewrap，也靠这三项才能在容器里跑；不用 Codex 时也要这三项，因为工具沙箱本身就需要。

**其他**：
- 端口只绑定到 `127.0.0.1`。要对外提供，放在带 HTTPS 的反向代理后面。代理改写了 `Host`（nginx 默认会改成 `proxy_pass` 的地址）时，在 `ip.yaml` 里写 `allowed_origins: [https://<对外域名>]`，否则浏览器连 `/ws` 会得到 403；也可以让代理保留原来的 `Host`（nginx：`proxy_set_header Host $host;`）。
- 密钥只放在 `.env`，不进镜像，也不写进 `ip.yaml`。
- Slack 用 Socket Mode，只需要容器能连出去，不需要开放任何入站端口。

**装到手机和电脑上**：控制台可以“安装”成应用（PWA），图标放在主屏幕或程序坞，打开后没有浏览器地址栏。前提是用 `https://` 访问（本机的 `http://localhost` 也行），明文 `http://` 连外网地址时浏览器不给装。
- 手机：iPhone 用 Safari 打开控制台，点“分享”再点“添加到主屏幕”；Android 用 Chrome 打开，菜单里点“安装应用”。
- 电脑：Chrome 或 Edge 打开控制台，地址栏右侧有安装图标；Mac 上的 Safari 用“文件 > 添加到程序坞”。
- 装好的应用和浏览器不共用存储，第一次打开要点右上角“令牌”再填一次访问令牌，或者用下面的配对码。
- 只缓存了控制台的页面和脚本，断网时能打开但显示“已断开”，数据都要连上主机才有。

**推送**：控制台“设备”页的“这台设备的通知”里点“开启推送”，页面关着、手机锁屏时也能收到审批卡、员工提问、一轮失败这些通知（哪些事要通知在同一张卡片里设）。
- 要用 `https://` 打开控制台。iPhone 要 iOS 16.4 以上，先“添加到主屏幕”，再从主屏幕打开的应用里开启；Safari 网页里没有这个按钮。
- 主机要能连出去访问推送服务：`fcm.googleapis.com`（Chrome、Edge、Android）、`web.push.apple.com`（iPhone、Mac Safari）、`*.push.services.mozilla.com`（Firefox）、`*.notify.windows.com`。
- 在 `ip.yaml` 里写上联系方式，推送服务有问题时找得到人；苹果的推送服务要求带：

  ```yaml
  push:
    contact: mailto:ops@example.com
  ```

- 手机 App 的锁屏推送经推送中转送到 APNs、FCM（m2-spec D-35，还在开发）。自己搭中转的，把中转的域名写进 `push.hosts`，主机才肯往那里发：

  ```yaml
  push:
    contact: mailto:ops@example.com
    hosts: [push.example.com]
  ```

- 主机第一次用到推送时生成自己的密钥，存在数据库里。删了数据库或换了主机，各设备打开控制台时会自动重新订阅。
- 以上只用假推送服务测过，没在真的手机上收过推送。

**给手机、电脑配对**（不用把共用令牌发给每个人）：每台设备换一个自己的令牌，消息和审批署设备名，丢了可以单独吊销。

1. 在主机上生成配对码（2 分钟内有效，只能用一次）。`--url` 写控制台对外的地址，终端里会画出二维码：

   ```bash
   docker compose run --rm ip device pair --config /srv/ip/ip.yaml --name 张三的手机 --url https://ip.example.com
   ```

   也可以在用共用令牌（或有“管理”权限的设备）打开的控制台里，到“设备”页点“生成配对码”。
2. 手机相机扫二维码，或者打开控制台点“令牌”输入 6 位数字。
3. 控制台“设备”页能看到谁在线、改名、改权限、吊销；吊销后那台设备立即断开，要用得重新配对。

新配对的设备是“日常”权限：能对话、审批、处理任务、看报告。要在手机上改路由规则、定时任务、记忆或管理设备，到“设备”页把那台设备改成“管理”。

猜错配对码 10 次，所有待用的配对码都会作废。

**桌面 App**：不想开浏览器的，可以装桌面 App（[`apps/desktop`](../../apps/desktop/README.md)）。它关掉窗口后留在托盘里，有事弹系统通知，托盘上显示待办数。打开后填控制台的对外地址和上面生成的配对码就能连上，令牌存在系统钥匙串里。主机必须配了 `auth_token_env`。安装包还没正式发布，要在仓库的 Actions 里手动运行 `desktop` 工作流打出来，只在 Linux 上试过。

**手机和电脑 App（Flutter）**：[`apps/flutter`](../../apps/flutter/README.md) 是 Android、iOS、macOS、Windows、Linux 都能用的 App（2026-11-21 起，backlog F-13）。打开后填控制台的对外地址和上面生成的配对码（或者粘贴配对链接）就能连上，令牌存在系统的安全存储里；主机必须配了 `auth_token_env`，不用改别的。现在能看待办、处理审批卡、和员工对话；通知、托盘、任务页还在做。安装包还没发布，要在仓库的 Actions 里手动运行 `flutter` 工作流打出来；只在 Linux 上跑过。

**不想把控制台放在公网：用 Tailscale**。主机和手机、电脑装上 Tailscale 并登进同一个 tailnet，打开 MagicDNS，就能用 `http://<主机名>.<tailnet>.ts.net:7878` 访问，流量在 WireGuard 隧道里加密。这时 `docker-compose.yml` 里的端口映射可以只绑 tailnet 的地址，`ip.yaml` 里照样要设 `auth_token_env`（监听非本机地址时必须设）。想要 `https://`（装 PWA 需要）可以用 `tailscale serve --https=443 http://127.0.0.1:7878`，Tailscale 会给 `*.ts.net` 签证书。以上 Tailscale 的用法照官方文档写，没有在这个项目里实测过。

## 资源上限

工具沙箱隔离了文件系统、网络和进程，但现在不限资源：员工的一条 shell 命令在超时之前可以不停 fork、吃光内存，或者往 `/tmp` 写大文件（沙箱里的 `/tmp` 是 tmpfs，占的是内存）。命令是模型写的，模型的输入里有网页、MCP 工具结果、渠道消息这些外部内容，所以不只是模型自己写错的问题（backlog E-15）。

`docker-compose.yml` 给容器设了三项上限，防的是这些命令拖垮整台主机：

| 项 | 默认值 | 防什么 | 在 `.env` 里用哪个变量改 |
| --- | --- | --- | --- |
| `pids_limit` | 1024 | fork 炸弹把主机的进程数用光 | `IP_PIDS_LIMIT` |
| `mem_limit`、`memswap_limit` | 2g | 吃光内存、往 `/tmp` 写大文件，触发主机的 OOM、杀掉主机上别的进程；两项取同一个值，超出的部分不会换到 swap 上 | `IP_MEM_LIMIT` |
| `cpus` | 1 | 死循环占满主机所有核 | `IP_CPUS` |

**怎么按主机调**：

- 默认值按 2 核 4 GB 的小主机取，给主机自己和 Docker 留出一半。主机更大、要跑 Codex 或编译项目时调高，比如 4 核 16 GB 的主机可以设 `IP_MEM_LIMIT=8g`、`IP_CPUS=3`。
- `IP_CPUS` 不能大于主机的核数（`nproc` 看），否则容器起不来。
- `IP_PIDS_LIMIT` 算的是容器里所有的进程和线程：服务自己的线程、MCP 子进程（比如 macro-mcp 的 Node.js）、Codex、员工的命令。员工多、Codex 并发跑编译时，`docker stats` 的 `PIDS` 一列接近上限就调高。
- `IP_MEM_LIMIT` 至少给服务、MCP 子进程和最大的一条命令留够；`docker stats` 的 `MEM USAGE` 经常接近上限，或者 `docker inspect` 里 `OOMKilled` 是 `true`，就调高。
- 改完 `.env` 后执行 `docker compose up -d` 让新值生效；`docker inspect --format '{{.HostConfig.Memory}} {{.HostConfig.PidsLimit}} {{.HostConfig.NanoCpus}}' <容器名>` 看实际生效的值。

**这是容器整体的兜底，不是每条命令的上限。** 服务进程和所有员工的命令共用这一份额度，所以：

- 一条命令把进程数或内存用光时，服务自己也可能开不了新线程，或者被容器里的 OOM 杀掉（所有员工都在这一个进程里）。容器设了 `restart: unless-stopped`，被杀后会自动重启，但正在进行的那几轮会中断。上限保住的是主机，保不住服务本身。
- 员工的工作目录在 `ip-data` 卷里，写在主机磁盘上，这三项都管不到磁盘；数据库在 `pg-data` 卷里，和它在同一块磁盘上，写满磁盘会影响数据库。
- 每条命令自己的上限（用 `prlimit` 限内存、文件大小和进程数，`/tmp` 的 tmpfs 加 `--size`，`ip check` 报告上限是否生效）是 backlog E-15 的第二步，还没做，见 backlog E-14。

compose 的写法用 `docker compose config` 在 Compose v5.7.1 上校验过，默认值和用变量覆盖都能解析；还没在真的容器里跑过耗尽资源的命令。

## 日常维护

**升级**：

```bash
git pull && docker compose up -d --build
```

**每天拉取公开数据**（打开了 `public` 数据源时）：在主机的 crontab 里加一行（路径换成你的）：

```cron
0 9 * * * cd /path/to/intelligent-position/rust && docker compose run --rm ip data pull --config /srv/ip/ip.yaml
```

**数据库**：存储在 compose 里的 `postgres` 服务上（PostgreSQL 17，数据卷 `pg-data`，不映射到主机端口）。第一次启动时
`deploy/postgres-init/01-roles.sh` 建 `ip_app` 角色和 `ip` schema（填了 `TCMS_DB_PASSWORD` 时另建团队上下文系统的 `tcms_app` 和 `tcms`），
两个角色互相没有权限。`.env` 里的 `POSTGRES_PASSWORD`、`IP_DB_PASSWORD` 只在数据卷第一次建出来时生效。
I.P. 启动时自己建表、跑迁移。改用自己已有的 PostgreSQL（RDS、Supabase 等）时，在库里照 `01-roles.sh` 建好角色和 schema，
把 `IP_DATABASE_URL` 换成那个库的连接串，去掉 compose 里的 `postgres` 服务。现在只支持不加密的连接，
连外部库要 TLS 的情况还没做（backlog D-6）。Supabase 用直连地址或 session 模式的连接池地址，不能用 transaction 模式（6543 端口）。

**从 SQLite 版升级**：以前的数据在 `ip-data` 卷的 `ip.db` 里。先停服务、备份 `ip-data` 卷（见下面），拉新代码后
`docker compose up -d postgres`，再导入，成功后启动：

```bash
docker compose stop ip
docker compose run --rm ip store import-sqlite --config /srv/ip/ip.yaml
docker compose up -d --build
```

目标 schema 里已经有数据时导入会拒绝；行数对不上整个回滚。`ip.db` 不会被改动，确认没问题后自己删。

**备份**：数据库每天用 `pg_dump` 导一份，按 schema 分开（团队上下文系统的用 `-n tcms`）。不用停服务。
开了团队上下文时，上下文仓库（`tcms-context` 卷，是真相）另外打一个 git bundle（配了 git 远端时，远端就是一份异地备份）：
`docker compose exec -T tcms git -C /var/lib/tcms/context bundle create - --all > context-$(date +%F).bundle`。

```bash
docker compose exec -T postgres pg_dump -U postgres -d ip -n ip -Fc > ip-$(date +%F).dump
```

恢复到一个空库：`pg_restore -U postgres -d ip --no-owner --role=ip_app ip-2026-11-22.dump`。

员工的工作目录在 `ip-data` 卷里，先停服务再打包。卷名前面会带上 compose 项目名，默认是目录名 `rust`，可以用 `docker volume ls` 确认。

```bash
docker compose stop ip
```

```bash
docker run --rm -v rust_ip-data:/data -v "$PWD":/backup debian:trixie-slim tar czf /backup/ip-data.tgz -C /data .
```

```bash
docker compose start ip
```

## 实测结果

2026-11-11 在 Docker 29.9.2、Compose v5.6.1、Linux 6.18 内核上实测。这台机器没有 AppArmor，所以 `apparmor=unconfined` 这一项没测到；它是给 Ubuntu 这类默认开 AppArmor 的主机准备的。

| 测了什么 | 结果 |
| --- | --- |
| 镜像大小 | 不带 Codex 291 MB；带 Codex 1.07 GB，其中 Codex 本身的程序就有 320 MB。2026-11-13 加了 Node.js 和 macro-mcp 以后，不带 Codex 是 434 MB（带 Codex 的没重测） |
| Node.js 和 macro-mcp（2026-11-13） | 镜像里 `node --version` 是 v20.19.2，没有 npm；`/srv/macro-mcp` 里有 `bin`、`src`、`package.json`、`LICENSE`、`README.md`；在 `/srv/ip` 下跑 `node ../macro-mcp/bin/whichapi-macro.js tools` 列出 9 个工具 |
| 示例配置本机启动 macro-mcp（2026-11-13） | `ip.example.yaml` 只取消 `auth_token_env` 的注释：`ip check` 里“MCP 服务器：1 个都连上了，共 9 个工具”，工具沙箱 ✓；`ip serve` 6 秒后 `healthy`，日志里 macro 服务器连上 9 个工具；`docker stop` 退出码 0 |
| 只改了 `Dockerfile` 再构建 | 编译缓存命中，一个 crate 都没有重新编译 |
| 合并 `main` 的新代码后再构建 | 只重新编译了改动涉及的 11 个 crate，用时 3 分 51 秒 |
| 容器里的用户 | uid 10001；两个命名卷里的文件属主都是 10001 |
| `ip check`，三项都放开 | 工具沙箱 ✓ |
| `ip check`，缺 `systempaths=unconfined` | 工具沙箱 ✗：`Can't mount proc on /proc`，后面跟着指向本文的提示 |
| `ip check`，缺 `seccomp=unconfined` 或三项都不放开 | 工具沙箱 ✗：不能建命名空间 |
| `docker compose up -d` | 5 到 8 秒后变成 `healthy`（起步期设的是 150 秒，第一次检查通过就算）；端口只在 `127.0.0.1:7878` |
| 访问令牌 | `/ws` 不带令牌或令牌不对都是 401，带对了才能连上；`/mcp` 不带令牌是 401 |
| `docker compose stop` | 收到信号就退出，退出码 0 |
| 员工真的跑一条 shell（用假模型触发） | 在沙箱里以 uid 10001 运行；看不到 `/srv`；没有任何能看到的进程带着 `IP_TOKEN`；断网；能写自己的工作目录，写下的文件在 `ip-data` 卷里 |
| 照“步骤”走一遍：`ip.example.yaml` 只取消 `auth_token_env` 的注释，`.env` 只填 `IP_TOKEN` | `ip check`：配置、令牌、数据目录、工具沙箱、示例数据都是 ✓；`OPENAI_API_KEY`、`MACRO_API_KEY` 没填，对应两项 ✗，提示的就是这两个变量 |
| `ip bench store --check`（在容器里） | 通过，搜索 p95 6.25 ms（目标 10 ms） |
| `ip bench startup --check`（在容器里） | 通过，启动中位数 3.9 ms，内存 16 MB |
| 带 Codex 的镜像 | `codex --version` 是 `codex-cli 0.167.1`；`ip check` 里“岗位 ops-tester 的 Codex 引擎” ✓；Codex 自带的 bubblewrap 三项都放开时能用，缺 `systempaths=unconfined` 时报同样的错 |
| 不带 Codex 的镜像配了 Codex 引擎 | `ip check` 里这一项 ✗：找不到 codex |

2026-11-23 加团队上下文时，在 Docker 29.10.0、Compose v5.7.1 上又试了一遍。这台机器只能经 HTTPS 代理出网，构建时临时多加了几行，让 cargo 和 apt 信任代理的证书、apt 走 https；仓库里的 `Dockerfile` 没有这几行。模型用的是不花钱的回声假模型（`evals/team-context/echo_model.py`），放在另一个容器里。

| 测了什么 | 结果 |
| --- | --- |
| 不开 `tcms` | `docker compose config --services` 只有 `postgres`、`ip`，`ip` 的 `depends_on` 里的 `tcms` 不起作用 |
| 开了 `tcms`，空卷第一次启动 | `pg-data` 新建时建了 `tcms_app` 和 `tcms`。旧版 TCMS 在这里起不来（`tcms_app` 没有在库里建 schema 的权限），TCMS 改成先查再建以后几秒 `healthy` |
| 导入、发令牌 | 造的上下文仓库（`evals/team-context/context/`）按三个范围导入，15 条；发了岗位令牌和审核令牌 |
| `ip check` | “团队上下文 macro-researcher：经 tcms-macro 取到了包：固定前缀约 366 token，按任务挑的约 280 token”。没写 `allow_http` 时，第一项“配置文件”就报错：http 只能连本机地址 |
| 团队上下文评测，回声假模型 | 在 `ip` 容器里跑 `ip eval --set evals/team-context.yaml`：9/10，和不用 Docker 时一样（没过的那题是规则原文“不写 DXY”被原样回了出来） |
| `docker compose up -d` | `ip` 等 `tcms` 变成 `healthy` 才起，日志里 `tcms-macro` 连上了 5 个工具 |
| 提议和批准 | 用岗位令牌调 `context_propose`，审核页接口批准后，`tcms-context` 卷里多了一个提交，提交者是审核的人；索引更新到 16 条，`context_search` 搜得到 |
| 审核页 | 本机 `http://127.0.0.1:7341/review` 是 200，别的 `Host` 是 403 |
| `docker compose stop tcms` | 退出码 0 |

同一天加了推到 git 远端（第 8 步），又试了一遍。TCMS 镜像在本机构建，代码和 TCMS 仓库合并推送以后的 `main` 一样。远端是主机上起的带 Basic 认证的假 git 服务（`git http-backend` 外面包一层），**不是真的 GitHub、Gitea**；模型还是回声假模型。

| 测了什么 | 结果 |
| --- | --- |
| 不填 `TCMS_REMOTE` | 和以前一样：日志里没有远端那一行，导入、`ip check` 照常 |
| 已经有内容的卷，再填上 `TCMS_REMOTE`、`TCMS_GIT_TOKEN`，`docker compose up -d tcms` | 日志“远端 …：每 3 秒同步一次”（试的时候把 `TCMS_SYNC_SECS` 设成了 3），马上推上去：“拉：远端还没有这个分支；推：推到了 b2d89d8eed6a”。远端多了导入那次的提交 |
| 人在自己的克隆里改一条、推到 `main` | 几秒内服务快进过去，索引更新，`tcms search` 搜到人改的内容 |
| 用岗位令牌调 `context_propose`，审核页接口批准 | 回答里 `remote.pushed_new` 是真，“推到了 4247cf350d26”；远端 `main` 依次是导入、人改、批准三个提交，批准那次的提交者是审核的人 |
| `docker compose exec tcms tcms sync` | “拉：和远端一样；推：远端已经是最新的”，退出码 0 |
| `ip check` | 照常取到包，按任务挑的部分从约 76 token 变成约 95 token |
| 新建的 `ip` 容器 | 读得到 `TCMS_GIT_TOKEN`（`env_file: .env`），和数据库密码一样 |

还没测的：真的用 Codex 改代码（要 `CODEX_API_KEY`），以及 Slack 和真实模型。这些在[验收手册](m2-acceptance.md)第 4、5 节里，用 Docker 起服务也照那里的步骤做。
