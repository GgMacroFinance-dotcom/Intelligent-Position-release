# 运维测试岗位：模型中转站巡检

运维测试岗位（`ops-tester`）的首要职责是维护服务器上的模型中转站：一个 [New API](https://github.com/QuantumNous/new-api) 类的网关，把客户端的 OpenAI 兼容请求转给上游模型，负责令牌、额度和扣费。这个目录是岗位的工具和演练，不含任何具体部署的地址和密钥。

New API 是 AGPL-3.0。这里只通过它的公开 HTTP 接口和主机上的 `docker`、`systemctl` 观察它，不复制、不链接它的代码。

| 文件 | 用途 |
| --- | --- |
| `gateway_check.py` | 巡检，只读。查状态和版本、不带令牌是否 401、模型和计价有没有漂移、逐个模型发一次极短的请求、TLS 证书、主机上的容器、系统和用户级服务、磁盘、最近一次备份。退出码：0 正常，1 警告，2 故障 |
| `release_check.py` | 看上游 New API 有没有比当前版本更新的发布（GitHub 公开接口），去掉定制后缀再比。退出码：0 最新，1 有更新 |
| `upstream_models.py` | 对比上游（比如 OpenCodex）的模型目录和中转站对外的模型，列出上游新增、上游已没有的；`--probe` 对新增的各试调一次 |
| `usage_report.py` | 一天的用量和报错统计（调用、token、扣费、各模型、前几名用户、报错分组、按小时），只对 `logs` 表做 SELECT |
| `remote_probe.sh` | 在中转站主机上跑的只读探针，由 `gateway_check.py --remote-cmd` 调用，每行输出一个 JSON |
| `fake_gateway.py` | 假中转站，能注入故障（版本不对、入口 502、鉴权丢失、模型下架、计价变化、上游鉴权失败、524、变慢）；顺带假扮 GitHub 的发布列表和上游的模型目录 |
| `fake-bin/` | 假的 `docker`、`systemctl`、`df`、`hostname`，以及在 SQLite 上执行 SQL 的假 `psql`，演练时代替主机命令 |
| `drill.sh` | 演练：巡检 16 个场景、新版本 3 个、上游模型 6 个、日报 6 个。CI 每次都跑 |
| `AGENTS.example.md` | 岗位手册的模板。复制到员工的工作目录改成 `AGENTS.md`，填上自己的部署 |
| `fixtures/` | 假中转站的模型和计价基线、短名映射、演练用的一天日志 |

只要 Python 3.9 以上和 POSIX sh，不装第三方包。

## 演练

```sh
sh examples/ops-gateway/drill.sh
```

只连本机的假中转站，探测令牌是假的 `sk-fake-probe`。

## 对着真中转站巡检

```sh
# 第一次：把当前的模型和计价记成基线（之后改了价格、上下架模型，确认无误后重新写一次）
python3 gateway_check.py --base-url https://api.example.com --write-baseline baseline.json

# 日常巡检。探测令牌建一个额度很低、只开要探测的模型的令牌，放进环境变量
export GATEWAY_PROBE_TOKEN=...      # 不写进任何文件
python3 gateway_check.py --base-url https://api.example.com \
  --expect-version v1.0.0-rc.46 --baseline baseline.json \
  --probe-token-env GATEWAY_PROBE_TOKEN --probe-model gpt-x --probe-model claude-y \
  --remote-cmd "sudo -n sh -s" \
  --remote-env PROBE_COMPOSE_PROJECT=new-api \
  --remote-env "PROBE_SERVICES=nginx" \
  --remote-env PROBE_BACKUP_DIR=/srv/backups/database \
  --json reports/check-$(date +%F).json
```

harness 和中转站在同一台主机上时，`--remote-cmd` 写 `sudo -n sh -s`；在别的主机上时写 `ssh <主机别名> sudo -n sh -s`。探针只读：`docker ps`、`systemctl is-active`、`df`、列备份目录，不读密钥文件。

每次探测会产生一次很短的调用（输入几个 token，输出 1 个），会扣一点额度。不给 `--probe-token-env` 就不探测模型。

## 新版本、上游新模型、日报

```sh
# New API 有没有新发布（每周）
python3 release_check.py --base-url https://api.example.com

# 上游有没有新模型（每天）。上游密钥放环境变量；mapping.json 是渠道的 model_mapping（短名 → 上游名）
export UPSTREAM_KEY=...
python3 upstream_models.py --base-url https://api.example.com --upstream-url http://127.0.0.1:<端口>/v1 \
  --upstream-key-env UPSTREAM_KEY --mapping mapping.json --ignore upstream-ignore.json --probe

# 当天的用量和报错（每晚）
python3 usage_report.py --sql-cmd "docker exec -i <postgres 容器> psql -X -q -U <用户> -d <库> -At" \
  --utc-offset +08:00 --out reports/$(date +%F)-日报.md
```

三个脚本都只读。发现新版本或新模型以后，真正的升级和上架由员工按手册准备，人确认后再做：
- 升级：先在隔离的预备环境里合并定制、跑测试和数据库升级回滚演练；
- 上架：先找到官方价格并写明出处。

`usage_report.py` 的 SQL 在 PostgreSQL 16 上核对过；演练里用 SQLite 加假 `psql` 跑同一套 SQL。

## 接到岗位上

岗位走外部引擎 Codex（见 `ip.example.yaml` 的 `engines.ops-tester`），Codex 自带 shell，命令要联网（`network: true`）。工作目录里放 `AGENTS.md`（从 `AGENTS.example.md` 改）、`baseline.json` 和这几个脚本，Codex 每轮都会读 `AGENTS.md`。定时任务写在 `schedules` 里，见 `ip.example.yaml` 的 `daily-gateway-check`（每天巡检）、`daily-upstream-models`（上游新模型）、`weekly-new-api-release`（新版本）、`nightly-gateway-report`（每晚 23:00 日报），默认都关着。

这几种操作会改生产，手册要求先用 `ask_human` 说明影响、给出推荐选项，人选定后再做，做之前先备份：

- 升级和回滚；
- 重启容器或服务；
- 改配置、改 Nginx；
- 动数据库；
- 改渠道、模型、价格；
- 删除文件。

没在真中转站上跑过之前，不写“已验证”。
