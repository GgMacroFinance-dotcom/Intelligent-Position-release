#!/usr/bin/env python3
"""对比上游（比如 OpenCodex）的模型目录和中转站对外的模型，找出上游新增和上游已经没有的。

  python3 upstream_models.py --base-url https://api.example.com --upstream-url http://127.0.0.1:8317/v1 \\
      --upstream-key-env UPSTREAM_KEY [--mapping mapping.json] [--ignore ignore.json] [--probe] [--json report.json]

- 中转站的模型从公开的 /api/pricing 取（不用令牌）。
- --mapping：中转站短名 → 上游模型名的 JSON（就是渠道的 model_mapping）。没写进映射的按同名算。
- --ignore：已经看过、决定不上架的上游模型名（JSON 数组），不再报为新增。
- --probe：对每个新增模型直接调一次上游（输入几个 token、输出 1 个），看它是不是真能用。会花上游一点额度。
只读：不改渠道、不加模型、不改价格。上架要人确认价格后另做。
退出码：0 没变化，1 有新增或消失，2 查不了。
"""
import argparse
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.request

OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def call(method, url, token=None, body=None, timeout=60):
    headers = {"User-Agent": "ip-ops-upstream-models", "Accept": "application/json"}
    data = None
    if body is not None:
        data, headers["Content-Type"] = json.dumps(body).encode(), "application/json"
    if token:
        headers["Authorization"] = "Bearer " + token
    try:
        with OPENER.open(urllib.request.Request(url, data=data, headers=headers, method=method), timeout=timeout) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"null")
        except ValueError:
            return e.code, None


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base-url", required=True)
    p.add_argument("--upstream-url", required=True, help="上游的 OpenAI 兼容地址，带 /v1")
    p.add_argument("--upstream-key-env", required=True, help="上游密钥所在的环境变量")
    p.add_argument("--mapping")
    p.add_argument("--ignore")
    p.add_argument("--probe", action="store_true")
    p.add_argument("--json")
    a = p.parse_args()
    key = os.environ.get(a.upstream_key_env, "").strip()
    if not key:
        print(f"环境变量 {a.upstream_key_env} 没设", file=sys.stderr)
        return 2
    mapping = json.loads(pathlib.Path(a.mapping).read_text("utf-8")) if a.mapping else {}
    ignore = set(json.loads(pathlib.Path(a.ignore).read_text("utf-8"))) if a.ignore else set()
    base = a.base_url.rstrip("/").removesuffix("/v1")
    up = a.upstream_url.rstrip("/")
    try:
        s1, pricing = call("GET", base + "/api/pricing")
        s2, models = call("GET", up + "/models", key)
    except (urllib.error.URLError, OSError) as e:
        print(f"查不了：{e}", file=sys.stderr)
        return 2
    if s1 != 200 or not (pricing or {}).get("success"):
        print(f"中转站 /api/pricing 回 {s1}", file=sys.stderr)
        return 2
    if s2 != 200:
        print(f"上游 /models 回 {s2}（密钥不对或上游挂了）", file=sys.stderr)
        return 2

    published = {m["model_name"] for m in pricing.get("data") or [] if m.get("model_name")}
    served = {mapping.get(m, m) for m in published}  # 对外的模型实际用到的上游模型
    upstream = {m["id"] for m in (models or {}).get("data") or [] if m.get("id")}
    new = sorted(upstream - served - ignore)
    gone = sorted(m for m in published if mapping.get(m, m) not in upstream)

    probes = {}
    if a.probe:
        for m in new:
            started = time.monotonic()
            body = {"model": m, "messages": [{"role": "user", "content": "ping"}], "max_tokens": 1}
            try:
                status, reply = call("POST", up + "/chat/completions", key, body)
            except (urllib.error.URLError, OSError) as e:
                status, reply = 0, {"error": {"message": str(e)}}
            err = (reply or {}).get("error") if isinstance(reply, dict) else None
            msg = (err.get("message") if isinstance(err, dict) else err) or ""
            probes[m] = {"http": status, "secs": round(time.monotonic() - started, 2), "error": str(msg)[:160]}

    print(f"中转站对外 {len(published)} 个模型，上游目录 {len(upstream)} 个。")
    if new:
        print(f"上游新增 {len(new)} 个（还没上架）：")
        for m in new:
            pr = probes.get(m)
            extra = "" if pr is None else (f"，试调 200，{pr['secs']} 秒" if pr["http"] == 200 else f"，试调 {pr['http']} {pr['error']}")
            print(f"- {m}{extra}")
    if gone:
        print(f"上游已经没有、中转站还在卖的 {len(gone)} 个：")
        for m in gone:
            print(f"- {m}（上游名 {mapping.get(m, m)}）")
    if not new and not gone:
        print("没有变化。")
    if a.json:
        out = {"published": sorted(published), "upstream": sorted(upstream), "new": new, "gone": gone, "probes": probes}
        pathlib.Path(a.json).write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", "utf-8")
    return 1 if new or gone else 0


if __name__ == "__main__":
    sys.exit(main())
