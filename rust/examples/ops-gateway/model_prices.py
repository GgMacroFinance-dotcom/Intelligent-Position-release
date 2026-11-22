#!/usr/bin/env python3
"""从 OpenRouter 的公开模型目录读价格，和 harness 配置（ip.yaml）里端点的 prices 对比，人确认后写回。

  python3 model_prices.py --config ip.yaml [--map prices-map.json] [--json report.json]   # 只对比，不改文件
  python3 model_prices.py --config ip.yaml [--map prices-map.json] --write                # 人确认后写回

- 价格来源：OpenRouter 的 GET /api/v1/models（公开，不用密钥），pricing 里是美元 / token，
  这里换算成 ip.yaml 的美元 / 百万 token：prompt → input，input_cache_read → cached_input，
  input_cache_write → cache_write，completion → output。
- 要定价的模型：每个端点 prices 里已有的，加上 roles 里用到的 `<端点>/<模型>`。
- 模型名怎么对到 OpenRouter 的 id：先看 --map（JSON，键写 `<端点>/<模型>` 或 `<模型>`，值是 OpenRouter 的 id，
  比如 "gpt-6-astra": "openai/gpt-6-astra"）；没写的，找 OpenRouter 里 `<厂商>/<模型>` 只有一个的那个（不算 :batch 这类变体）。
  找不到或有好几个的，报出来、不改，请人在 --map 里写明。
- OpenRouter 有分档的（比如输入超过 27.2 万 token 后更贵），harness 只记基础档，输出里会注明。
- --write：先把原文件备份成 <文件>.bak-<时间>，只改 endpoints 下各端点的 prices 段，别的行原样保留。
  harness 不会热加载配置，写完要重启才生效。
退出码：0 没有变化（或已写回），1 有变化或有对不上的模型，2 查不了，3 配置读不懂。
"""
import argparse
import datetime
import json
import os
import pathlib
import re
import shutil
import sys
import urllib.error
import urllib.request

FIELDS = (("input", "prompt"), ("cached_input", "input_cache_read"), ("cache_write", "input_cache_write"),
          ("output", "completion"))


def fetch(url, timeout=60):
    # OpenRouter 在外网，走系统代理（和只连本机的其他脚本不同）。
    req = urllib.request.Request(url, headers={"User-Agent": "ip-ops-model-prices", "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def per_million(v):
    """OpenRouter 的美元 / token 字符串 → 美元 / 百万 token；没有或是负数（价格不定）时返回 None。"""
    try:
        x = float(v) * 1_000_000
    except (TypeError, ValueError):
        return None
    return None if x < 0 else float(f"{x:.6g}")


def catalog(data):
    out = {}
    for m in data.get("data") or []:
        mid, pr = m.get("id"), m.get("pricing") or {}
        if not mid:
            continue
        price = {ours: per_million(pr.get(theirs)) for ours, theirs in FIELDS}
        if price["input"] is None or price["output"] is None:
            continue
        price = {k: v for k, v in price.items() if v is not None}
        if price.get("cached_input") == price["input"]:
            price.pop("cached_input")
        if price.get("cache_write") == price["input"]:
            price.pop("cache_write")
        out[mid] = {"price": price, "tiered": bool(pr.get("overrides"))}
    return out


# ---- 只认 ip.yaml 里用得到的那一小部分 YAML：顶层 endpoints / roles，端点下的 prices，prices 下的一行一个模型。

def indent_of(line):
    return len(line) - len(line.lstrip(" "))


def meaningful(line):
    s = line.strip()
    return s and not s.startswith("#")


def top_block(lines, key):
    """顶层 `key:` 块的行号范围 [start, end)，不含 key 那一行。"""
    for i, line in enumerate(lines):
        if re.match(rf"^{re.escape(key)}:\s*(#.*)?$", line):
            j = i + 1
            while j < len(lines) and (not meaningful(lines[j]) or indent_of(lines[j]) > 0):
                j += 1
            while j > i + 1 and not meaningful(lines[j - 1]):  # 块尾的空行和注释留给后面
                j -= 1
            return i + 1, j
    return None


def children(lines, start, end):
    """块里最外一层的 `name:` 子项：[(名字, 名字所在行, 子块结束行, 缩进)]。"""
    first = next((i for i in range(start, end) if meaningful(lines[i])), None)
    if first is None:
        return []
    ind = indent_of(lines[first])
    heads = [i for i in range(start, end) if meaningful(lines[i]) and indent_of(lines[i]) == ind]
    out = []
    for n, i in enumerate(heads):
        m = re.match(r"^\s*([A-Za-z0-9_.\-]+):", lines[i])
        if not m:
            continue
        stop = heads[n + 1] if n + 1 < len(heads) else end
        while stop > i + 1 and not meaningful(lines[stop - 1]):
            stop -= 1
        out.append((m.group(1), i, stop, ind))
    return out


def parse_flow(text):
    """`{ input: 10, output: 50 }` → dict。"""
    m = re.match(r"^\{(.*)\}\s*(#.*)?$", text.strip())
    if not m:
        raise ValueError(text)
    out = {}
    for part in filter(None, (p.strip() for p in m.group(1).split(","))):
        k, _, v = part.partition(":")
        out[k.strip()] = float(v)
    return out


def read_config(text):
    lines = text.split("\n")
    eps = top_block(lines, "endpoints")
    if eps is None:
        raise ValueError("没有顶层 endpoints")
    endpoints = {}
    for name, i, stop, _ in children(lines, *eps):
        prices, block = {}, None
        for key, j, pstop, _ in children(lines, i + 1, stop):
            if key != "prices":
                continue
            block = (j, pstop)
            for model, k, _, _ in children(lines, j + 1, pstop):
                prices[model] = parse_flow(lines[k].split(":", 1)[1])
        endpoints[name] = {"head": i, "end": stop, "prices": prices, "block": block}
    used = set()
    roles = top_block(lines, "roles")
    if roles:
        for line in lines[roles[0]:roles[1]]:
            code = line.split("#", 1)[0]
            for ep, model in re.findall(r"(?<![\w.\-/])([A-Za-z0-9_\-]+)/([A-Za-z0-9_.:\-]+)", code):
                if ep in endpoints:
                    used.add((ep, model))
    return lines, endpoints, used


def fmt_price(p):
    order = [k for k, _ in FIELDS if k in p]
    return "{ " + ", ".join(f"{k}: {p[k]:g}" for k in order) + " }"


def write_config(lines, endpoints, new_prices):
    """new_prices: {端点: {模型: 价格}}，是改完以后的完整 prices。从后往前改，行号不乱。"""
    for name in sorted(new_prices, key=lambda n: endpoints[n]["head"], reverse=True):
        ep = endpoints[name]
        ind = None
        for j in range(ep["head"] + 1, ep["end"]):
            if meaningful(lines[j]):
                ind = indent_of(lines[j])
                break
        if ind is None:
            ind = indent_of(lines[ep["head"]]) + 2
        pad = " " * ind
        body = [f"{pad}prices:"] + [f"{pad}  {m}: {fmt_price(p)}" for m, p in sorted(new_prices[name].items())]
        if ep["block"]:
            j, pstop = ep["block"]
            # 保留 prices 段里原有的注释行，放在段首
            notes = [lines[k] for k in range(j + 1, pstop) if lines[k].strip().startswith("#")]
            lines[j:pstop] = body[:1] + notes + body[1:]
        else:
            lines[ep["end"]:ep["end"]] = body
    return "\n".join(lines)


def same(a, b):
    keys = set(a) | set(b)
    return all(k in a and k in b and abs(a[k] - b[k]) <= 1e-9 * max(1, abs(a[k])) for k in keys)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--config", required=True, help="harness 的 ip.yaml")
    p.add_argument("--map", help="模型名 → OpenRouter id 的 JSON")
    p.add_argument("--openrouter-url", default="https://openrouter.ai/api/v1")
    p.add_argument("--write", action="store_true", help="人确认后才加：把变化写回配置")
    p.add_argument("--json")
    a = p.parse_args()
    path = pathlib.Path(a.config)
    try:
        lines, endpoints, used = read_config(path.read_text("utf-8"))
    except (OSError, ValueError) as e:
        print(f"配置读不懂：{e}", file=sys.stderr)
        return 3
    mapping = json.loads(pathlib.Path(a.map).read_text("utf-8")) if a.map else {}
    try:
        table = catalog(fetch(a.openrouter_url.rstrip("/") + "/models"))
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(f"查不了 OpenRouter：{e}", file=sys.stderr)
        return 2
    if not table:
        print("OpenRouter 目录是空的", file=sys.stderr)
        return 2
    by_name = {}
    for mid in table:
        if ":" not in mid:
            by_name.setdefault(mid.split("/", 1)[-1], []).append(mid)

    wanted = sorted({(ep, m) for ep, e in endpoints.items() for m in e["prices"]} | used)
    changes, unmatched, rows = {}, [], []
    for ep, model in wanted:
        old = endpoints[ep]["prices"].get(model)
        mid = mapping.get(f"{ep}/{model}") or mapping.get(model)
        if mid is None:
            hits = by_name.get(model, [])
            if len(hits) != 1:
                unmatched.append({"model": f"{ep}/{model}", "candidates": sorted(hits)})
                continue
            mid = hits[0]
        if mid not in table:
            unmatched.append({"model": f"{ep}/{model}", "openrouter": mid, "candidates": []})
            continue
        new, tiered = table[mid]["price"], table[mid]["tiered"]
        status = "same" if old is not None and same(old, new) else ("new" if old is None else "changed")
        rows.append({"model": f"{ep}/{model}", "openrouter": mid, "old": old, "new": new, "tiered": tiered,
                     "status": status})
        if status != "same":
            changes.setdefault(ep, dict(endpoints[ep]["prices"]))[model] = new

    print(f"OpenRouter 目录 {len(table)} 个模型；配置里要定价的 {len(wanted)} 个。")
    for r in rows:
        tier = "（OpenRouter 有分档，这里只记基础档）" if r["tiered"] else ""
        if r["status"] == "same":
            print(f"- {r['model']}：没变 {fmt_price(r['new'])}{tier}")
        elif r["status"] == "new":
            print(f"- {r['model']}：配置里还没有价格，OpenRouter（{r['openrouter']}）是 {fmt_price(r['new'])}{tier}")
        else:
            print(f"- {r['model']}：价格变了 {fmt_price(r['old'])} → {fmt_price(r['new'])}（{r['openrouter']}）{tier}")
    for u in unmatched:
        if u.get("openrouter"):
            print(f"- {u['model']}：--map 写的 {u['openrouter']} 在 OpenRouter 上没有，保留原样")
        elif u["candidates"]:
            print(f"- {u['model']}：OpenRouter 上有好几个同名的（{'、'.join(u['candidates'])}），请在 --map 里写明，先不改")
        else:
            print(f"- {u['model']}：OpenRouter 上找不到，请在 --map 里写明，先不改")
    if a.json:
        out = {"rows": rows, "unmatched": unmatched, "changed": sorted(f"{e}/{m}" for e in changes for m in changes[e]
                                                                      if not same(endpoints[e]["prices"].get(m) or {}, changes[e][m]))}
        pathlib.Path(a.json).write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", "utf-8")
    if not changes:
        print("价格没有变化。")
        return 1 if unmatched else 0
    if not a.write:
        print("有变化，没写回。人确认后加 --write 再跑一次。")
        return 1
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    backup = path.with_name(f"{path.name}.bak-{stamp}")
    shutil.copy2(path, backup)
    text = write_config(lines, endpoints, changes)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(text, "utf-8")
    # 新文件保留原文件的权限和属主（配置里可能有不该让别人读的内容；root 跑时不会变成 root:root）
    st = path.stat()
    os.chmod(tmp, st.st_mode & 0o7777)
    if hasattr(os, "chown") and (st.st_uid, st.st_gid) != (os.getuid(), os.getgid()):
        try:
            os.chown(tmp, st.st_uid, st.st_gid)
        except PermissionError:
            pass  # 不是 root 时改不了属主，文件本来就是自己的
    os.replace(tmp, path)
    _, again, _ = read_config(path.read_text("utf-8"))  # 写完再读一遍，确认读得回来
    for ep, prices in changes.items():
        if any(not same(again[ep]["prices"].get(m) or {}, pr) for m, pr in prices.items()):
            shutil.copy2(backup, path)
            print("写回以后读出来不对，已经还原", file=sys.stderr)
            return 3
    print(f"已写回 {path}（原文件备份在 {backup}）。harness 重启后生效。")
    return 1 if unmatched else 0


if __name__ == "__main__":
    sys.exit(main())
