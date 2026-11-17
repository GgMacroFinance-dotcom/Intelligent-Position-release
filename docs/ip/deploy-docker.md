# 用 Docker 部署

> 一台 Linux 云主机上，用 `docker compose` 起 harness。镜像里带工具沙箱 bubblewrap，可选带 Codex。
> 相关文件都在 `rust/` 下：`Dockerfile`、`docker-compose.yml`、`deploy/env.example`。

## 要求

- Linux 主机，Docker Engine 24 以上，带 Compose v2.17 以上（`docker compose` 命令；`additional_contexts` 要 2.17）。
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

## 容器里的路径

| 路径 | 是什么 | 怎么来的 |
| --- | --- | --- |
| `/srv/ip/ip.yaml` | 你的配置 | 从 `rust/ip.yaml` 只读挂进来 |
| `/srv/ip/data` | 数据库、员工的工作目录、Codex 的配置目录 | 命名卷 `ip-data` |
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
- 只缓存了控制台的页面和脚本，断网时能打开但显示“已断开”，数据都要连上主机才有。推送还没做（见 [App 方案](apps-plan.md) F-9、F-10）。

**给手机、电脑配对**（不用把共用令牌发给每个人）：每台设备换一个自己的令牌，消息和审批署设备名，丢了可以单独吊销。

1. 在主机上生成配对码（2 分钟内有效，只能用一次）。`--url` 写控制台对外的地址，终端里会画出二维码：

   ```bash
   docker compose run --rm ip device pair --config /srv/ip/ip.yaml --name 张三的手机 --url https://ip.example.com
   ```

   也可以在用共用令牌打开的控制台里，到“设备”页点“生成配对码”。
2. 手机相机扫二维码，或者打开控制台点“令牌”输入 6 位数字。
3. 控制台“设备”页能看到谁在线、改名、吊销；吊销后那台设备立即断开，要用得重新配对。

猜错配对码 10 次，所有待用的配对码都会作废。

**不想把控制台放在公网：用 Tailscale**。主机和手机、电脑装上 Tailscale 并登进同一个 tailnet，打开 MagicDNS，就能用 `http://<主机名>.<tailnet>.ts.net:7878` 访问，流量在 WireGuard 隧道里加密。这时 `docker-compose.yml` 里的端口映射可以只绑 tailnet 的地址，`ip.yaml` 里照样要设 `auth_token_env`（监听非本机地址时必须设）。想要 `https://`（装 PWA 需要）可以用 `tailscale serve --https=443 http://127.0.0.1:7878`，Tailscale 会给 `*.ts.net` 签证书。以上 Tailscale 的用法照官方文档写，没有在这个项目里实测过。

## 日常维护

**升级**：

```bash
git pull && docker compose up -d --build
```

**每天拉取公开数据**（打开了 `public` 数据源时）：在主机的 crontab 里加一行（路径换成你的）：

```cron
0 9 * * * cd /path/to/intelligent-position/rust && docker compose run --rm ip data pull --config /srv/ip/ip.yaml
```

**备份**：先停服务，再把 `ip-data` 卷打包。卷名前面会带上 compose 项目名，默认是目录名 `rust`，可以用 `docker volume ls` 确认。

```bash
docker compose stop
```

```bash
docker run --rm -v rust_ip-data:/data -v "$PWD":/backup debian:trixie-slim tar czf /backup/ip-data.tgz -C /data .
```

```bash
docker compose start
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

还没测的：真的用 Codex 改代码（要 `CODEX_API_KEY`），以及 Slack 和真实模型。这些在[验收手册](m2-acceptance.md)第 4、5 节里，用 Docker 起服务也照那里的步骤做。
