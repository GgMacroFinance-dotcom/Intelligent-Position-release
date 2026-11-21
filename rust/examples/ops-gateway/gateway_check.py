#!/usr/bin/env python3
"""New API 类模型中转站的巡检。只读：不登录后台、不改配置；探测模型时每个模型发一次极短的请求（会扣一点额度）。

用法：
  python3 gateway_check.py --base-url https://api.example.com \\
      [--expect-version v1.0.0] [--baseline baseline.json | --write-baseline baseline.json] \\
      [--probe-token-env GATEWAY_PROBE_TOKEN --probe-model gpt-x --probe-model ...] \\
      [--remote-cmd "ssh gateway sudo -n sh -s" --remote-env PROBE_BACKUP_DIR=/srv/backups ...] \\
      [--json report.json]

退出码：0 全部正常，1 有警告，2 有故障，3 巡检本身没跑起来（参数不对）。
结果先打印一张中文表，再（如果给了 --json）写一份机器可读的报告。令牌只从环境变量读，不打印。
"""
import argparse
import datetime
import json
import os
import pathlib
import shlex
import socket
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

OK, WARN, FAIL = "ok", "warn", "fail"
HERE = pathlib.Path(__file__).resolve().parent
# 不走系统代理：巡检要看的是中转站本身。
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class Report:
    def __init__(self):
        self.items = []

    def add(self, check, level, detail, **data):
        self.items.append({"check": check, "level": level, "detail": detail, **data})

    def worst(self):
        levels = [i["level"] for i in self.items]
        return FAIL if FAIL in levels else WARN if WARN in levels else OK


def http(method, url, token=None, body=None, timeout=30):
    """回 (状态码, 解析后的 JSON 或 None, 耗时秒)。连不上时状态码为 0，JSON 里放原因。"""
    headers = {"User-Agent": "ip-ops-gateway-check", "Accept": "application/json"}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = "Bearer " + token
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    started = time.monotonic()
    try:
        with OPENER.open(req, timeout=timeout) as r:
            raw, status = r.read(), r.status
    except urllib.error.HTTPError as e:
        raw, status = e.read(), e.code
    except (urllib.error.URLError, socket.timeout, ConnectionError, TimeoutError) as e:
        reason = getattr(e, "reason", e)
        return 0, {"error": f"连不上：{reason}"}, time.monotonic() - started
    elapsed = time.monotonic() - started
    try:
        return status, json.loads(raw or b"null"), elapsed
    except ValueError:
        return status, {"raw": raw[:200].decode("utf-8", "replace")}, elapsed


def upstream_error(payload):
    """从 OpenAI 风格的错误里取一句话，不带令牌。"""
    if isinstance(payload, dict):
        err = payload.get("error")
        if isinstance(err, dict):
            return str(err.get("message") or err.get("code") or err)[:200]
        if err:
            return str(err)[:200]
        if payload.get("raw"):
            return payload["raw"]
    return ""


def check_status(rep, base, expect_version):
    status, body, secs = http("GET", base + "/api/status")
    if status != 200 or not isinstance(body, dict) or not body.get("success"):
        rep.add("status", FAIL, f"/api/status 回 {status} {upstream_error(body)}".strip(), http=status)
        return None
    version = (body.get("data") or {}).get("version")
    if expect_version and version != expect_version:
        rep.add("status", FAIL, f"版本是 {version}，应为 {expect_version}", version=version, secs=round(secs, 2))
    else:
        rep.add("status", OK, f"版本 {version}，{secs:.2f} 秒", version=version, secs=round(secs, 2))
    return version


def check_auth(rep, base):
    status, body, _ = http("GET", base + "/v1/models")
    if status == 401:
        rep.add("auth", OK, "不带令牌访问 /v1/models 回 401")
    elif status == 0:
        rep.add("auth", FAIL, upstream_error(body))
    else:
        rep.add("auth", FAIL, f"不带令牌访问 /v1/models 回 {status}，应为 401", http=status)


def pricing_view(body):
    """模型名 → 计价字段。只比对影响扣费和可见性的字段。"""
    keys = ["quota_type", "model_ratio", "model_price", "completion_ratio", "cache_ratio", "create_cache_ratio"]
    out = {}
    for m in (body or {}).get("data") or []:
        if isinstance(m, dict) and m.get("model_name"):
            out[m["model_name"]] = {k: m.get(k) for k in keys}
    return out


def check_pricing(rep, base, baseline_path, write_path):
    status, body, _ = http("GET", base + "/api/pricing")
    if status != 200 or not isinstance(body, dict) or not body.get("success"):
        rep.add("pricing", FAIL, f"/api/pricing 回 {status} {upstream_error(body)}".strip(), http=status)
        return []
    now = pricing_view(body)
    if write_path:
        pathlib.Path(write_path).write_text(json.dumps(now, ensure_ascii=False, indent=2, sort_keys=True) + "\n", "utf-8")
        rep.add("pricing", OK, f"{len(now)} 个模型，基线写进 {write_path}", models=len(now))
        return sorted(now)
    if not baseline_path:
        rep.add("pricing", OK, f"{len(now)} 个模型（没给基线，不比对）", models=len(now))
        return sorted(now)
    base_view = json.loads(pathlib.Path(baseline_path).read_text("utf-8"))
    added = sorted(set(now) - set(base_view))
    removed = sorted(set(base_view) - set(now))
    changed = sorted(m for m in set(now) & set(base_view) if now[m] != base_view[m])
    if added or removed or changed:
        parts = [f"新增 {', '.join(added)}" if added else "", f"下架 {', '.join(removed)}" if removed else "",
                 f"计价变了 {', '.join(changed)}" if changed else ""]
        rep.add("pricing", WARN, "；".join(p for p in parts if p), added=added, removed=removed, changed=changed)
    else:
        rep.add("pricing", OK, f"{len(now)} 个模型，和基线一致", models=len(now))
    return sorted(now)


def check_probes(rep, base, token, models, timeout, slow_secs):
    status, body, _ = http("GET", base + "/v1/models", token=token)
    listed = set()
    if status == 200 and isinstance(body, dict):
        listed = {m.get("id") for m in body.get("data") or [] if isinstance(m, dict)}
    else:
        rep.add("probe-token", FAIL, f"探测令牌访问 /v1/models 回 {status} {upstream_error(body)}".strip(), http=status)
        return
    for model in models:
        if model not in listed:
            rep.add("probe", FAIL, f"{model}：探测令牌的模型列表里没有它", model=model)
            continue
        req = {"model": model, "messages": [{"role": "user", "content": "ping"}], "max_tokens": 1, "stream": False}
        status, body, secs = http("POST", base + "/v1/chat/completions", token=token, body=req, timeout=timeout)
        info = {"model": model, "http": status, "secs": round(secs, 2)}
        if status == 200:
            level = WARN if secs > slow_secs else OK
            rep.add("probe", level, f"{model}：{secs:.1f} 秒" + ("，偏慢" if level == WARN else ""), **info)
        elif status == 0:
            rep.add("probe", FAIL, f"{model}：{upstream_error(body)}（{secs:.0f} 秒）", **info)
        elif status == 524:
            rep.add("probe", FAIL, f"{model}：524，源站在 {secs:.0f} 秒内没回（代理超时）", **info)
        else:
            rep.add("probe", FAIL, f"{model}：{status} {upstream_error(body)}".strip(), **info)


def check_tls(rep, base, warn_days):
    u = urllib.parse.urlsplit(base)
    if u.scheme != "https":
        return
    host, port = u.hostname, u.port or 443
    try:
        ctx = ssl.create_default_context()
        with socket.create_connection((host, port), timeout=10) as sock, ctx.wrap_socket(sock, server_hostname=host) as s:
            not_after = s.getpeercert()["notAfter"]
    except (OSError, ssl.SSLError) as e:
        rep.add("tls", FAIL, f"TLS 握手失败：{e}")
        return
    expires = datetime.datetime.fromtimestamp(ssl.cert_time_to_seconds(not_after), datetime.timezone.utc)
    days = (expires - datetime.datetime.now(datetime.timezone.utc)).days
    level = FAIL if days < 3 else WARN if days < warn_days else OK
    rep.add("tls", level, f"证书还有 {days} 天到期（{expires.date()}）", days=days)


def check_remote(rep, cmd, env_pairs, backup_hours, disk_warn, disk_fail):
    # 探针的设置写成脚本开头的赋值，这样不管远端命令是 ssh、sudo 还是本地的 sh 都一样。
    prelude = "".join(f"{k}={shlex.quote(v)}\n" for k, v in (pair.split("=", 1) for pair in env_pairs))
    script = prelude.encode() + (HERE / "remote_probe.sh").read_bytes()
    argv = shlex.split(cmd)
    try:
        r = subprocess.run(argv, input=script, capture_output=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired) as e:
        rep.add("remote", FAIL, f"探针没跑起来：{e}")
        return
    if r.returncode != 0 and not r.stdout:
        rep.add("remote", FAIL, f"探针退出码 {r.returncode}：{r.stderr.decode('utf-8', 'replace').strip()[:200]}")
        return
    lines = []
    for raw in r.stdout.decode("utf-8", "replace").splitlines():
        try:
            lines.append(json.loads(raw))
        except ValueError:
            rep.add("remote", WARN, f"探针输出读不懂：{raw[:120]}")
    host = next((l for l in lines if l.get("kind") == "host"), {})
    now_s = host.get("now_s") or time.time()
    containers = [l for l in lines if l.get("kind") == "container"]
    if not containers:
        rep.add("containers", FAIL, "没找到 compose 项目里的容器")
    for c in containers:
        st = c.get("status", "")
        if "(unhealthy)" in st or not st.startswith("Up"):
            rep.add("containers", FAIL, f"{c['name']}：{st}", name=c["name"])
        elif "(health: starting)" in st:
            rep.add("containers", WARN, f"{c['name']}：{st}", name=c["name"])
        else:
            rep.add("containers", OK, f"{c['name']}：{st}", name=c["name"])
    for s in (l for l in lines if l.get("kind") == "service"):
        level = OK if s.get("state") == "active" else FAIL
        rep.add("services", level, f"{s['name']}：{s.get('state')}", name=s["name"])
    for d in (l for l in lines if l.get("kind") == "disk"):
        pct = d.get("used_pct")
        if pct is None:
            rep.add("disk", WARN, f"{d['path']}：读不到用量")
            continue
        level = FAIL if pct >= disk_fail else WARN if pct >= disk_warn else OK
        rep.add("disk", level, f"{d['path']}：已用 {pct}%", path=d["path"], used_pct=pct)
    for b in (l for l in lines if l.get("kind") == "backup"):
        if not b.get("file"):
            rep.add("backup", FAIL, "备份目录里没有备份文件")
            continue
        age_h = (now_s - b["mtime_s"]) / 3600
        level = FAIL if age_h > backup_hours * 2 else WARN if age_h > backup_hours else OK
        if b.get("bytes", 0) == 0:
            level = FAIL
        rep.add("backup", level, f"最近一次 {b['file']}，{age_h:.1f} 小时前，{b.get('bytes', 0)} 字节",
                age_hours=round(age_h, 1), bytes=b.get("bytes"))
    for e in (l for l in lines if l.get("kind") == "error"):
        rep.add("remote", FAIL, f"{e.get('what')}：{e.get('detail')}")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base-url", required=True, help="中转站的地址，不带 /v1")
    p.add_argument("--expect-version", help="应该是哪个版本（/api/status 的 version）")
    p.add_argument("--baseline", help="比对模型和计价的基线文件")
    p.add_argument("--write-baseline", help="把当前的模型和计价写成基线文件（不比对）")
    p.add_argument("--probe-token-env", help="探测令牌所在的环境变量；不给就不探测模型")
    p.add_argument("--probe-model", action="append", default=[], help="要探测的模型，可以写多次")
    p.add_argument("--probe-timeout", type=float, default=60)
    p.add_argument("--slow-secs", type=float, default=20, help="探测超过多少秒算偏慢")
    p.add_argument("--remote-cmd", help="在主机上跑探针的命令，探针从标准输入读进去，比如 \"ssh gateway sudo -n sh -s\"")
    p.add_argument("--remote-env", action="append", default=[], help="传给探针的 VAR=值，可以写多次")
    p.add_argument("--backup-hours", type=float, default=26, help="备份超过多少小时算过期")
    p.add_argument("--disk-warn", type=int, default=80)
    p.add_argument("--disk-fail", type=int, default=92)
    p.add_argument("--tls-warn-days", type=int, default=14)
    p.add_argument("--json", help="机器可读的报告写到哪里")
    a = p.parse_args()
    base = a.base_url.rstrip("/")
    if base.endswith("/v1"):
        base = base[:-3]
    token = None
    if a.probe_token_env:
        token = os.environ.get(a.probe_token_env, "").strip()
        if not token:
            print(f"环境变量 {a.probe_token_env} 没设", file=sys.stderr)
            return 3
    for pair in a.remote_env:
        if "=" not in pair or not pair.split("=", 1)[0].startswith("PROBE_"):
            print(f"--remote-env 只收 PROBE_ 开头的 VAR=值：{pair}", file=sys.stderr)
            return 3

    rep = Report()
    started = datetime.datetime.now(datetime.timezone.utc)
    check_status(rep, base, a.expect_version)
    check_auth(rep, base)
    check_pricing(rep, base, a.baseline, a.write_baseline)
    if token:
        check_probes(rep, base, token, a.probe_model, a.probe_timeout, a.slow_secs)
    check_tls(rep, base, a.tls_warn_days)
    if a.remote_cmd:
        check_remote(rep, a.remote_cmd, a.remote_env, a.backup_hours, a.disk_warn, a.disk_fail)

    mark = {OK: "正常", WARN: "警告", FAIL: "故障"}
    print(f"巡检 {base}（{started.strftime('%Y-%m-%d %H:%M:%S')} UTC）：{mark[rep.worst()]}")
    print("| 项目 | 结果 | 说明 |\n| --- | --- | --- |")
    for i in rep.items:
        print(f"| {i['check']} | {mark[i['level']]} | {i['detail']} |")
    if a.json:
        out = {"base_url": base, "started_at": started.isoformat(), "result": rep.worst(), "items": rep.items}
        pathlib.Path(a.json).write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", "utf-8")
    return {OK: 0, WARN: 1, FAIL: 2}[rep.worst()]


if __name__ == "__main__":
    sys.exit(main())
