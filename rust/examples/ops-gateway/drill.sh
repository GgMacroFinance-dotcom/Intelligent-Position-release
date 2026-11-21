#!/bin/sh
# 故障演练：对着假中转站（fake_gateway.py）和假主机命令（fake-bin/）逐个注入故障，
# 检查 gateway_check.py 能不能报出来；再演练 release_check.py（新版本）、upstream_models.py（上游新增模型）
# 和 usage_report.py（日报）。全部场景都符合预期时退出码为 0。
# 用法：sh drill.sh。只用本机，不连真中转站，不要密钥（探测令牌是假的 sk-fake-probe）。
set -u
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'kill $gw 2>/dev/null; rm -rf "$work"' EXIT INT TERM
gw=
fail=0
port=$((20000 + $$ % 20000))

# 场景名 期望退出码 输出里应有的字 假中转站的故障（空格分隔） 假主机的环境变量（空格分隔）
scenario() {
  name=$1 want=$2 needle=$3 faults=$4 hostenv=$5
  args=
  for f in $faults; do args="$args --fault $f"; done
  # shellcheck disable=SC2086
  python3 "$here/fake_gateway.py" --port "$port" $args >"$work/gw.log" 2>&1 &
  gw=$!
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    grep -q 假中转站 "$work/gw.log" 2>/dev/null && break
    sleep 0.2
  done
  backups="$work/backups"
  rm -rf "$backups" && mkdir -p "$backups"
  age=${BACKUP_AGE:-3}
  printf 'x' >"$backups/ai-$name.dump"
  touch -d "$age hours ago" "$backups/ai-$name.dump"
  out=$(GATEWAY_PROBE_TOKEN=sk-fake-probe python3 "$here/gateway_check.py" \
    --base-url "http://127.0.0.1:$port" --expect-version v1.0.0-fake \
    --baseline "$here/fixtures/baseline.json" \
    --probe-token-env GATEWAY_PROBE_TOKEN --probe-model model-a --probe-model model-b --probe-timeout 5 --slow-secs 1 \
    --remote-cmd "env PATH=$here/fake-bin:$PATH $hostenv sh -s" \
    --remote-env "PROBE_SERVICES=nginx opencodex" --remote-env "PROBE_BACKUP_DIR=$backups" 2>&1)
  got=$?
  kill $gw 2>/dev/null
  wait $gw 2>/dev/null
  gw=
  if [ "$got" = "$want" ] && printf '%s' "$out" | grep -q -- "$needle"; then
    echo "ok   $name（退出码 $got，报了“$needle”）"
  else
    echo "FAIL $name：退出码 $got（应为 $want），输出里应有“$needle”"
    printf '%s\n' "$out" | sed 's/^/     /'
    fail=1
  fi
  port=$((port + 1))
}

scenario 全部正常      0 "正常"                 ""                         ""
scenario 版本不对      2 "应为 v1.0.0-fake"     "version=v1.0.0-rc.47"     ""
scenario 入口挂了      2 "/api/status 回 502"   "status_down"              ""
scenario 鉴权丢了      2 "应为 401"             "open_models"              ""
scenario 模型下架      2 "下架 model-b"         "drop_model=model-b"       ""
scenario 计价变了      1 "计价变了 model-a"     "price=model-a:2.5"        ""
scenario 上游鉴权失败  2 "upstream authentication failed" "upstream_auth=model-a" ""
scenario 代理超时524   2 "524"                  "timeout524=model-b"       ""
scenario 模型偏慢      1 "偏慢"                 "slow=model-a:1.5"         ""
scenario 容器不健康    2 "unhealthy"            ""                         "FAKE_UNHEALTHY=new-api"
scenario 容器退出      2 "Exited"               ""                         "FAKE_EXITED=redis"
scenario 服务停了      2 "opencodex：failed" ""                       "FAKE_INACTIVE=opencodex"
scenario 磁盘快满      1 "已用 85%"             ""                         "FAKE_DISK_PCT=85"
BACKUP_AGE=30 scenario 备份过期 1 "30.0 小时前"  ""                         ""
BACKUP_AGE=60 scenario 备份停了 2 "60.0 小时前"  ""                         ""
# 跑一个命令，核对退出码和输出里的字。参数：场景名 期望退出码 应有的字 命令...
expect() {
  name=$1 want=$2 needle=$3
  shift 3
  out=$("$@" 2>&1)
  got=$?
  if [ "$got" = "$want" ] && printf '%s' "$out" | grep -q -- "$needle"; then
    echo "ok   $name（退出码 $got，报了“$needle”）"
  else
    echo "FAIL $name：退出码 $got（应为 $want），输出里应有“$needle”"
    printf '%s\n' "$out" | sed 's/^/     /'
    fail=1
  fi
}

start_gw() {
  # shellcheck disable=SC2068
  python3 "$here/fake_gateway.py" --port "$port" $@ >"$work/gw.log" 2>&1 &
  gw=$!
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    grep -q 假中转站 "$work/gw.log" 2>/dev/null && break
    sleep 0.2
  done
}
stop_gw() { kill $gw 2>/dev/null; wait $gw 2>/dev/null; gw=; port=$((port + 1)); }

# 新版本：当前 v1.0.0-rc.46 加定制后缀；上游之后出了 rc.47（预发布）和 v1.0.0（正式版）。
start_gw --fault version=v1.0.0-rc.46+custom.20261118 --release v1.0.0-rc.45:pre --release v1.0.0-rc.46:pre \
  --release v1.0.0-rc.47:pre --release v1.0.0
gh="http://127.0.0.1:$port/github"
expect 新版本发布   1 "v1.0.0-rc.47" python3 "$here/release_check.py" --base-url "http://127.0.0.1:$port" --github-api "$gh"
expect 只看正式版   1 "有 1 个更新的发布" python3 "$here/release_check.py" --base-url "http://127.0.0.1:$port" --github-api "$gh" --stable-only
expect 已是最新     0 "已是最新" python3 "$here/release_check.py" --current v1.0.0 --github-api "$gh"
stop_gw

# 上游模型：上游多了两个（一个调不通），少了 model-b。
start_gw --fault upstream_new=vendor/model-c --fault upstream_new=vendor/broken-d --fault upstream_gone=vendor/model-b
up="http://127.0.0.1:$port/upstream/v1"
expect 上游新增模型 1 "vendor/model-c，试调 200" env UPSTREAM_KEY=sk-fake-upstream python3 "$here/upstream_models.py" \
  --base-url "http://127.0.0.1:$port" --upstream-url "$up" --upstream-key-env UPSTREAM_KEY --mapping "$here/fixtures/mapping.json" --probe
expect 新增调不通   1 "vendor/broken-d，试调 404" env UPSTREAM_KEY=sk-fake-upstream python3 "$here/upstream_models.py" \
  --base-url "http://127.0.0.1:$port" --upstream-url "$up" --upstream-key-env UPSTREAM_KEY --mapping "$here/fixtures/mapping.json" --probe
expect 上游没了     1 "model-b（上游名 vendor/model-b）" env UPSTREAM_KEY=sk-fake-upstream python3 "$here/upstream_models.py" \
  --base-url "http://127.0.0.1:$port" --upstream-url "$up" --upstream-key-env UPSTREAM_KEY --mapping "$here/fixtures/mapping.json"
printf '["vendor/model-c", "vendor/broken-d"]' >"$work/ignore.json"
out=$(env UPSTREAM_KEY=sk-fake-upstream python3 "$here/upstream_models.py" --base-url "http://127.0.0.1:$port" \
  --upstream-url "$up" --upstream-key-env UPSTREAM_KEY --mapping "$here/fixtures/mapping.json" --ignore "$work/ignore.json" 2>&1)
if printf '%s' "$out" | grep -q 上游新增; then
  echo "FAIL 忽略已看过的：忽略清单里的模型还被报成新增"; printf '%s\n' "$out" | sed 's/^/     /'; fail=1
else
  echo "ok   忽略已看过的（忽略清单里的模型不再报）"
fi
expect 上游密钥不对 2 "密钥不对" env UPSTREAM_KEY=wrong python3 "$here/upstream_models.py" \
  --base-url "http://127.0.0.1:$port" --upstream-url "$up" --upstream-key-env UPSTREAM_KEY
stop_gw

# 日报：假 psql 在 SQLite 上跑同样的 SQL。
day_start=$(python3 -c 'import datetime as d; tz=d.timezone(d.timedelta(hours=8)); print(int(d.datetime.combine(d.date(2026,11,22), d.time(), tz).timestamp()))')
sed "s/:DAY_START/$day_start/g" "$here/fixtures/logs.sql" | python3 -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.executescript(sys.stdin.read()); c.commit()' "$work/logs.db"
report() { FAKE_DB="$work/logs.db" python3 "$here/usage_report.py" --sql-cmd "$here/fake-bin/psql" --date 2026-11-22 --json "$work/report.json" "$@"; }
expect 日报总数     0 "调用 3 次，报错 3 次（报错率 50.0%），活跃用户 2 个" report
expect 日报扣费     0 "扣费约 \$1.20（600,000 额度）" report
expect 日报报错分组 0 "| model-a | A timeout occurred 524 | 2 |" report
expect 日报报错带竖线 0 "upstream authentication failed (401)/渠道 1" report
expect 日报没有调用 0 "当天没有调用" report --date 2026-11-23
expect 日报查不了   3 "查不了日志" python3 "$here/usage_report.py" --sql-cmd "false"
exit $fail
