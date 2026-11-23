#!/usr/bin/env bash
# 团队上下文评测（TCMS 第 7 步）：起一个本机的 tcms serve，装上 context/ 里造的上下文仓库，
# 用 evals/team-context.yaml 的 10 道题比较三种配置：
#   off    不接团队上下文（没有 TCMS 服务器，也没有 team_context）
#   on     接上，预算 4000 token：相关条目全文放在对话开头
#   small  接上，预算只比固定前缀多一点：相关条目只列标题，员工要自己用 context_read / context_search 读
#
#   evals/team-context/run.sh [--echo] [--model openai/gpt-6-luna] [--repeat 3] [--only off,on,small] [--out 目录]
#
# 在 rust/ 下运行。要先设：
#   TCMS_DATABASE_URL   团队上下文系统用的 PostgreSQL（用 schema tcms_eval_team，每次先删掉重建）
#   IP_DATABASE_URL     I.P. 用的 PostgreSQL（用 schema ip_eval_team，每次先删掉重建）
#   OPENAI_API_KEY      用真实模型时要；--echo 不用
# 可选：TCMS_BIN（默认 PATH 里的 tcms）、IP_BIN（默认 ./target/release/ip）、TCMS_PORT（默认 7361）。
# --echo 用 echo_model.py 这个回声假模型：不花钱，只看团队上下文进没进提示词、检查能不能判出来。
# 每遍的完整结果（JSON）写进 --out 目录（默认一个临时目录），最后打印汇总。要 psql、python3、curl、git。
set -euo pipefail

: "${TCMS_DATABASE_URL:?要先设 TCMS_DATABASE_URL}"
: "${IP_DATABASE_URL:?要先设 IP_DATABASE_URL}"
HERE=$(cd "$(dirname "$0")" && pwd)
TCMS=${TCMS_BIN:-tcms}
IP=${IP_BIN:-./target/release/ip}
PORT=${TCMS_PORT:-7361}
MODEL=openai/gpt-6-luna
REPEAT=1
ONLY=off,on,small
OUT=
ECHO=
while [ $# -gt 0 ]; do
  case $1 in
    --echo) ECHO=1; MODEL=echo/echo ;;
    --model) MODEL=$2; shift ;;
    --repeat) REPEAT=$2; shift ;;
    --only) ONLY=$2; shift ;;
    --out) OUT=$2; shift ;;
    *) echo "不认识的参数 $1"; exit 2 ;;
  esac
  shift
done
command -v "$TCMS" >/dev/null || { echo "找不到 tcms（设 TCMS_BIN）"; exit 1; }
[ -x "$IP" ] || { echo "找不到 $IP（先 cargo build --release -p ip-cli，或设 IP_BIN）"; exit 1; }
WORK=$(mktemp -d)
OUT=${OUT:-$WORK/results}
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
PIDS=()
trap 'kill "${PIDS[@]}" 2>/dev/null || true' EXIT INT TERM

# 上下文仓库：把 context/ 拷出来提交一次。
cp -r "$HERE/context" "$WORK/repo"
git -C "$WORK/repo" init -q
git -C "$WORK/repo" add -A
git -C "$WORK/repo" -c user.name=eval -c user.email=eval@example.com commit -qm fixture
export TCMS_SCHEMA=tcms_eval_team TCMS_REPO=$WORK/repo
psql "$TCMS_DATABASE_URL" -qc "DROP SCHEMA IF EXISTS tcms_eval_team CASCADE" 2>/dev/null
psql "$IP_DATABASE_URL" -qc "DROP SCHEMA IF EXISTS ip_eval_team CASCADE" 2>/dev/null
"$TCMS" reindex >/dev/null
TCMS_TOKEN_MACRO=$("$TCMS" token add --name "I.P. 评测" --role macro --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["secret"])')
export TCMS_TOKEN_MACRO
# 小预算：固定前缀再加 120 token，相关条目放不下全文，只剩标题目录。
PREFIX=$("$TCMS" get --role macro --task 评测 --budget 200000 --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["prefix_tokens"])')
SMALL=$((PREFIX + 120))

"$TCMS" serve --listen "127.0.0.1:$PORT" > "$WORK/tcms.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 50); do
  curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null && break
  sleep 0.2
done
if [ -n "$ECHO" ]; then
  python3 "$HERE/echo_model.py" --port 18091 &
  PIDS+=($!)
  export ECHO_API_KEY=echo
  sleep 0.5
fi

# 三种配置只差团队上下文那两段。员工不带工作目录（tools: false），只能回答和用 TCMS 的工具；记忆关掉，免得别的上下文混进来。
config() { # 名字 预算（空表示不接）
  local file=$WORK/$1.yaml
  cat > "$file" <<YAML
database: { url_env: IP_DATABASE_URL, schema: ip_eval_team }
utc_offset: "+08:00"
org_prompt: |
  你在一家 AI 原生的研究机构工作。结论先行；引用数据注明来源和日期；不确定时直接说明。
sandbox: { mode: none, network: false }
max_steps: 8
memory: { enabled: false }
endpoints:
$ENDPOINT
roles:
  - id: macro-researcher
    name: 宏观研究员
    description: |
      跟踪国内外宏观数据，撰写宏观研究报告和数据点评；也回答同事关于本组工作安排、口径和规矩的问题。
      不知道的就直接说不知道，不要编。
    model:
      default: $MODEL
agents:
  - { id: macro-1, role: macro-researcher, name: 小宏, tools: false }
YAML
  if [ -n "$2" ]; then
    cat >> "$file" <<YAML
mcp_servers:
  tcms-macro:
    url: http://127.0.0.1:$PORT/mcp
    bearer_token_env: TCMS_TOKEN_MACRO
    roles: [macro-researcher]
    timeout_secs: 30
team_context:
  budget_tokens: $2
  roles:
    macro-researcher: { server: tcms-macro, role: macro }
YAML
  fi
  echo "$file"
}
# 只写要用的那个端点：启动时每个端点的 key 都要设。价格是 2026-11-14 按 OpenAI 模型文档核对的，和 ip.example.yaml 一样。
if [ -n "$ECHO" ]; then
  ENDPOINT="  echo: { base_url: http://127.0.0.1:18091/v1, api_key_env: ECHO_API_KEY }"
else
  : "${OPENAI_API_KEY:?要先设 OPENAI_API_KEY（或用 --echo）}"
  ENDPOINT="  openai:
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY
    api: responses
    prices:
      gpt-6-astra: { input: 10, cached_input: 1, cache_write: 12.5, output: 50 }
      gpt-6-luna: { input: 0.10, cached_input: 0.01, cache_write: 0.125, output: 0.50 }"
fi
declare -A BUDGET=([off]="" [on]=4000 [small]=$SMALL)
SAFE_MODEL=${MODEL//\//-}
echo "模型 $MODEL，每种配置 $REPEAT 遍，固定前缀约 $PREFIX token，小预算 $SMALL token，结果写进 $OUT"
JOBS=()
for cond in ${ONLY//,/ }; do
  cfg=$(config "$cond" "${BUDGET[$cond]}")
  for i in $(seq "$REPEAT"); do
    name=$SAFE_MODEL-$cond-$i
    "$IP" eval --config "$cfg" --set evals/team-context.yaml --model "$MODEL" --timeout-secs 300 \
      --out "$OUT/$name.json" > "$OUT/$name.log" 2>&1 &
    JOBS+=($!)
  done
done
for job in "${JOBS[@]}"; do wait "$job" || true; done

python3 - "$OUT" "$SAFE_MODEL" <<'PY'
import json, pathlib, sys
out, model = pathlib.Path(sys.argv[1]), sys.argv[2]
runs = sorted(out.glob(f"{model}-*.json"))
if not runs:
    sys.exit("没有结果，看同目录下的 .log")
total = 0.0
for cond in ["off", "on", "small"]:
    mine = [json.loads(p.read_text()) for p in runs if p.stem.rsplit("-", 2)[1] == cond]
    if not mine:
        continue
    passed = sum(r["passed"] for r in mine)
    cases = sum(r["total"] for r in mine)
    cost = sum(r["cost_usd"] for r in mine)
    total += cost
    reads = sum(1 for r in mine for c in r["cases"] if any(t.endswith("__context_read") or t.endswith("__context_search") for t in c["tools_used"]))
    fails = {}
    for r in mine:
        for c in r["cases"]:
            if not c["pass"]:
                fails[c["id"]] = fails.get(c["id"], 0) + 1
    print(f"{cond:>5}：通过 {passed}/{cases}，{len(mine)} 遍共 {cost:.4f} 美元，用了 context_read 或 context_search 的题 {reads} 次；没过的：{fails or '无'}")
print(f"合计 {total:.4f} 美元")
PY
