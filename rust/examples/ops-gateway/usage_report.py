#!/usr/bin/env python3
"""模型中转站（New API）一天的用量和报错统计。只读：只对 logs 表做 SELECT，不读请求和回复内容。

  python3 usage_report.py --sql-cmd "docker exec -i new-api-postgres-1 psql -X -q -U newapi -d newapi -At" \\
      [--date 2026-11-22] [--utc-offset +08:00] [--out reports/2026-11-22-日报.md] [--json reports/2026-11-22-日报.json]

--sql-cmd 是一个从标准输入读 SQL、每行输出一条结果、列之间用 | 分隔的命令（psql -At 的默认格式）。
SQL 只用 PostgreSQL 和 SQLite 都支持的写法（另外用到 chr()，SQLite 里由演练的假 psql 提供）。

统计口径（New API 的日志）：type=2 是一次扣费的调用，type=5 是一次报错。额度换算成美元按 500000 额度 = 1 美元
（New API 的 QuotaPerUnit 默认值，改过的用 --quota-per-usd 改）。报错信息只取前 100 个字，不含令牌。
退出码：0 生成了报告，3 没跑起来。
"""
import argparse
import datetime
import json
import pathlib
import shlex
import subprocess
import sys

SEP = "|"


def query(cmd, sql):
    r = subprocess.run(shlex.split(cmd), input=sql.encode(), capture_output=True, timeout=120)
    if r.returncode != 0:
        raise RuntimeError(f"SQL 命令退出码 {r.returncode}：{r.stderr.decode('utf-8', 'replace').strip()[:300]}")
    return [line.split(SEP) for line in r.stdout.decode("utf-8", "replace").splitlines() if line.strip()]


def num(v):
    try:
        return float(v) if "." in v else int(v)
    except (TypeError, ValueError):
        return 0


def clean(expr):
    """去掉 | 和换行，免得打乱输出的列。"""
    return f"replace(replace(replace({expr}, '|', '/'), chr(10), ' '), chr(13), ' ')"


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--sql-cmd", required=True)
    p.add_argument("--date", help="哪一天（YYYY-MM-DD，按 --utc-offset 的当地日期），默认今天")
    p.add_argument("--utc-offset", default="+08:00")
    p.add_argument("--quota-per-usd", type=float, default=500000)
    p.add_argument("--top", type=int, default=10)
    p.add_argument("--out", help="Markdown 报告写到哪里；不给就打印")
    p.add_argument("--json")
    a = p.parse_args()

    sign = -1 if a.utc_offset.startswith("-") else 1
    hh, mm = a.utc_offset.lstrip("+-").split(":")
    tz = datetime.timezone(sign * datetime.timedelta(hours=int(hh), minutes=int(mm)))
    day = datetime.date.fromisoformat(a.date) if a.date else datetime.datetime.now(tz).date()
    start = int(datetime.datetime.combine(day, datetime.time(), tz).timestamp())
    end = start + 86400
    window = f"created_at >= {start} AND created_at < {end}"
    usd = a.quota_per_usd

    try:
        totals = query(a.sql_cmd, f"""SELECT
  SUM(CASE WHEN type = 2 THEN 1 ELSE 0 END), SUM(CASE WHEN type = 5 THEN 1 ELSE 0 END),
  COALESCE(SUM(CASE WHEN type = 2 THEN prompt_tokens ELSE 0 END), 0),
  COALESCE(SUM(CASE WHEN type = 2 THEN completion_tokens ELSE 0 END), 0),
  COALESCE(SUM(CASE WHEN type = 2 THEN quota ELSE 0 END), 0),
  COUNT(DISTINCT CASE WHEN type = 2 THEN user_id END)
FROM logs WHERE {window};""")
        models = query(a.sql_cmd, f"""SELECT {clean('model_name')},
  SUM(CASE WHEN type = 2 THEN 1 ELSE 0 END), SUM(CASE WHEN type = 5 THEN 1 ELSE 0 END),
  COALESCE(SUM(CASE WHEN type = 2 THEN prompt_tokens ELSE 0 END), 0),
  COALESCE(SUM(CASE WHEN type = 2 THEN completion_tokens ELSE 0 END), 0),
  COALESCE(SUM(CASE WHEN type = 2 THEN quota ELSE 0 END), 0),
  COALESCE(ROUND(AVG(CASE WHEN type = 2 THEN use_time END), 1), 0),
  COALESCE(MAX(CASE WHEN type = 2 THEN use_time END), 0)
FROM logs WHERE {window} AND type IN (2, 5) GROUP BY model_name ORDER BY 2 DESC, 3 DESC;""")
        users = query(a.sql_cmd, f"""SELECT {clean('username')}, COUNT(*), COALESCE(SUM(quota), 0)
FROM logs WHERE {window} AND type = 2 GROUP BY username ORDER BY 3 DESC LIMIT {a.top};""")
        errors = query(a.sql_cmd, f"""SELECT {clean('model_name')}, {clean('substr(content, 1, 100)')}, COUNT(*)
FROM logs WHERE {window} AND type = 5 GROUP BY model_name, substr(content, 1, 100) ORDER BY 3 DESC LIMIT {a.top * 2};""")
        hours = query(a.sql_cmd, f"""SELECT (created_at - {start}) / 3600, SUM(CASE WHEN type = 2 THEN 1 ELSE 0 END),
  SUM(CASE WHEN type = 5 THEN 1 ELSE 0 END)
FROM logs WHERE {window} AND type IN (2, 5) GROUP BY (created_at - {start}) / 3600 ORDER BY 1;""")
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as e:
        print(f"查不了日志：{e}", file=sys.stderr)
        return 3

    t = [num(v) for v in (totals[0] if totals else [])] or [0] * 6
    calls, errs, p_tok, c_tok, quota, active = (t + [0] * 6)[:6]
    rate = errs / (calls + errs) * 100 if calls + errs else 0
    lines = [
        f"# 中转站日报 {day}（{a.utc_offset}）",
        "",
        f"- 调用 {calls} 次，报错 {errs} 次（报错率 {rate:.1f}%），活跃用户 {active} 个。",
        f"- 输入 {p_tok:,} token，输出 {c_tok:,} token，扣费约 ${quota / usd:.2f}（{quota:,} 额度）。",
        "",
        "## 各模型",
        "",
        "| 模型 | 调用 | 报错 | 输入 token | 输出 token | 扣费（$） | 平均耗时（秒） | 最长耗时（秒） |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    model_rows = []
    for r in models:
        name, ok, bad, pt, ct, q, avg, mx = r[0], *map(num, r[1:8])
        model_rows.append({"model": name, "calls": ok, "errors": bad, "prompt_tokens": pt, "completion_tokens": ct,
                           "quota": q, "avg_secs": avg, "max_secs": mx})
        lines.append(f"| {name or '（空）'} | {ok} | {bad} | {pt:,} | {ct:,} | {q / usd:.2f} | {avg} | {mx} |")
    if not models:
        lines.append("| （当天没有调用） | | | | | | | |")
    lines += ["", f"## 用量前 {a.top} 的用户", "", "| 用户 | 调用 | 扣费（$） |", "| --- | ---: | ---: |"]
    user_rows = []
    for r in users:
        user_rows.append({"user": r[0], "calls": num(r[1]), "quota": num(r[2])})
        lines.append(f"| {r[0]} | {num(r[1])} | {num(r[2]) / usd:.2f} |")
    lines += ["", "## 报错", ""]
    error_rows = []
    if errors:
        lines += ["| 模型 | 报错信息（前 100 字） | 次数 |", "| --- | --- | ---: |"]
        for r in errors:
            error_rows.append({"model": r[0], "message": r[1], "count": num(r[2])})
            lines.append(f"| {r[0] or '（空）'} | {r[1]} | {num(r[2])} |")
    else:
        lines.append("当天没有报错。")
    lines += ["", "## 按小时", "", "| 时段 | 调用 | 报错 |", "| --- | ---: | ---: |"]
    hour_rows = []
    for r in hours:
        h = num(r[0])
        hour_rows.append({"hour": h, "calls": num(r[1]), "errors": num(r[2])})
        lines.append(f"| {h:02d}:00 | {num(r[1])} | {num(r[2])} |")
    text = "\n".join(lines) + "\n"

    if a.out:
        pathlib.Path(a.out).write_text(text, "utf-8")
        print(f"日报写进 {a.out}")
    else:
        print(text, end="")
    if a.json:
        out = {"date": str(day), "utc_offset": a.utc_offset, "calls": calls, "errors": errs, "error_rate_pct": round(rate, 2),
               "active_users": active, "prompt_tokens": p_tok, "completion_tokens": c_tok, "quota": quota,
               "usd": round(quota / usd, 4), "models": model_rows, "users": user_rows, "error_groups": error_rows,
               "hours": hour_rows}
        pathlib.Path(a.json).write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", "utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
