#!/usr/bin/env python3
"""看上游 New API 有没有比中转站当前版本更新的发布。只读，不下载、不升级。

  python3 release_check.py --base-url https://api.example.com [--repo QuantumNous/new-api] [--current v1.0.0-rc.46]
      [--stable-only] [--json report.json]

当前版本默认从中转站的 /api/status 取，去掉 “+” 后面的定制后缀（比如 v1.0.0-rc.46+custom.20261118 → v1.0.0-rc.46）。
发布列表来自 GitHub 的公开接口，按发布时间排；当前版本之后发布的都算更新。当前版本本身是预发布（rc、beta）时
预发布也算，否则只算正式版；--stable-only 时一律只算正式版。设了 GITHUB_TOKEN 就带上，免得被限流。
退出码：0 已是最新，1 有更新，2 查不了。
"""
import argparse
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request

OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
SYSTEM_OPENER = urllib.request.build_opener()  # GitHub 走系统代理（如果有）


def get(url, opener, token=None):
    headers = {"User-Agent": "ip-ops-release-check", "Accept": "application/vnd.github+json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    with opener.open(urllib.request.Request(url, headers=headers), timeout=30) as r:
        return json.load(r)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base-url", help="中转站地址；给了 --current 就不用")
    p.add_argument("--current", help="当前的上游版本标签")
    p.add_argument("--repo", default="QuantumNous/new-api")
    p.add_argument("--github-api", default="https://api.github.com")
    p.add_argument("--stable-only", action="store_true")
    p.add_argument("--json")
    a = p.parse_args()
    try:
        current = a.current
        if not current:
            if not a.base_url:
                print("要给 --base-url 或 --current", file=sys.stderr)
                return 2
            status = get(a.base_url.rstrip("/").removesuffix("/v1") + "/api/status", OPENER)
            current = (status.get("data") or {}).get("version") or ""
        upstream = current.split("+", 1)[0]
        releases = get(f"{a.github_api.rstrip('/')}/repos/{a.repo}/releases?per_page=30", SYSTEM_OPENER,
                       os.environ.get("GITHUB_TOKEN") or None)
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(f"查不了：{e}", file=sys.stderr)
        return 2

    releases = sorted((r for r in releases if not r.get("draft")), key=lambda r: r.get("published_at") or "")
    tags = [r.get("tag_name") for r in releases]
    allow_pre = not a.stable_only and any(m in upstream for m in ("-rc", "-beta", "-alpha"))
    if upstream in tags:
        newer = releases[tags.index(upstream) + 1:]
        note = ""
    else:
        # 当前版本不在最近 30 个发布里：要么太旧，要么不是官方标签。都列出来让人看。
        newer = releases
        note = f"当前版本 {upstream} 不在最近 {len(releases)} 个发布里"
    newer = [r for r in newer if allow_pre or not r.get("prerelease")]

    print(f"中转站当前版本 {current}（上游 {upstream}），{a.repo}")
    if note:
        print(note)
    if not newer:
        print("已是最新。")
    else:
        print(f"有 {len(newer)} 个更新的发布：")
        print("| 版本 | 发布时间 | 预发布 | 说明 | 链接 |\n| --- | --- | --- | --- | --- |")
        for r in newer:
            body = " ".join((r.get("body") or "").split())[:120]
            print(f"| {r.get('tag_name')} | {(r.get('published_at') or '')[:10]} | {'是' if r.get('prerelease') else '否'} | {body} | {r.get('html_url')} |")
    if a.json:
        out = {"current": current, "upstream": upstream, "repo": a.repo, "note": note,
               "newer": [{k: r.get(k) for k in ("tag_name", "published_at", "prerelease", "html_url")} for r in newer]}
        pathlib.Path(a.json).write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", "utf-8")
    return 1 if newer else 0


if __name__ == "__main__":
    sys.exit(main())
