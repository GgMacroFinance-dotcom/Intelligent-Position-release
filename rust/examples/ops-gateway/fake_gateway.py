#!/usr/bin/env python3
"""假的 New API 类模型中转站，只给演练和测试用。不连任何上游，不需要密钥。

  python3 fake_gateway.py --port 18080 [--fault 名字[=参数] ...]

只实现巡检会碰到的接口：/api/status、/api/pricing、/v1/models、/v1/chat/completions。
另外顺带假扮两个外部服务，给 release_check.py 和 upstream_models.py 演练：
  /github/repos/<owner>/<repo>/releases   GitHub 的发布列表（--release 注入，最新的写在最后）
  /upstream/v1/models、/upstream/v1/chat/completions   上游（比如 OpenCodex）的模型目录和调用
  /openrouter/api/v1/models   OpenRouter 的模型目录（含价格，美元 / token），给 model_prices.py 演练
令牌固定是 sk-fake-probe（不是真密钥）。故障用 --fault 注入，可以写多次：

  version=<版本>        /api/status 报这个版本
  status_down           /api/status 回 502（像 Nginx 后面的容器挂了）
  open_models           不带令牌访问 /v1/models 也回 200（鉴权丢了）
  drop_model=<模型>     模型从价格目录和模型列表里消失
  price=<模型>:<倍率>   改这个模型的 model_ratio
  upstream_auth=<模型>  调这个模型时回 500，错误说上游鉴权失败
  timeout524=<模型>     调这个模型时等 2 秒回 524（像 CDN 等源站超时）
  slow=<模型>:<秒>      调这个模型时先等这么多秒再正常回
  upstream_new=<模型>   上游目录里多出这个模型（模型名带 broken 的，调用回 404）
  upstream_gone=<模型>  上游目录里没有这个模型
  or_price=<id>:<输入价>  OpenRouter 目录里这个模型的输入价改成这么多（美元 / 百万 token）
  or_gone=<id>          OpenRouter 目录里没有这个模型

--release <标签>[:pre] 往假 GitHub 的发布列表里加一个版本，可以写多次，按时间先后写。
"""
import argparse
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = "sk-fake-probe"
VERSION = "v1.0.0-fake"
UPSTREAM_TOKEN = "sk-fake-upstream"
# 中转站对外的短名 → 上游的模型名（真实部署里是渠道的 model_mapping）。
UPSTREAM_NAMES = {"model-a": "vendor/model-a", "model-b": "vendor/model-b", "image-1": "vendor/image-1"}
MODELS = {
    "model-a": {"quota_type": 0, "model_ratio": 1.25, "model_price": 0, "completion_ratio": 8, "owner_by": "fake"},
    "model-b": {"quota_type": 0, "model_ratio": 0.5, "model_price": 0, "completion_ratio": 4, "owner_by": "fake"},
    "image-1": {"quota_type": 1, "model_ratio": 0, "model_price": 0.04, "completion_ratio": 0, "owner_by": "fake"},
}
# 假 OpenRouter 目录：id → 价格（美元 / 百万 token）。和 fixtures/ip-prices.yaml 里写的一致。
OPENROUTER = {
    "vendor/model-a": {"prompt": 10, "completion": 50, "input_cache_read": 1, "input_cache_write": 12.5},
    "vendor/model-b": {"prompt": 0.1, "completion": 0.5, "input_cache_read": 0.01},
    "other/model-b": {"prompt": 9, "completion": 9},
    "vendor/model-a:batch": {"prompt": 5, "completion": 25},
}


def parse_faults(items):
    faults = {}
    for item in items:
        name, _, arg = item.partition("=")
        faults.setdefault(name, []).append(arg)
    return faults


class Handler(BaseHTTPRequestHandler):
    faults = {}
    releases = []

    def log_message(self, *_):
        pass

    def send(self, status, body):
        raw = json.dumps(body, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def models(self):
        out = {k: dict(v) for k, v in MODELS.items() if k not in self.faults.get("drop_model", [])}
        for spec in self.faults.get("price", []):
            model, _, ratio = spec.partition(":")
            if model in out:
                out[model]["model_ratio"] = float(ratio)
        return out

    def authed(self):
        return self.headers.get("Authorization") == "Bearer " + TOKEN

    def upstream_models(self):
        gone = self.faults.get("upstream_gone", [])
        names = [n for n in UPSTREAM_NAMES.values() if n not in gone]
        return names + self.faults.get("upstream_new", [])

    def do_GET(self):
        if self.path.startswith("/github/repos/") and self.path.split("?")[0].endswith("/releases"):
            out = []
            for i, spec in enumerate(self.releases):
                tag, _, pre = spec.partition(":")
                out.append({"tag_name": tag, "name": tag, "prerelease": pre == "pre", "draft": False,
                            "published_at": f"2026-11-{i + 1:02d}T00:00:00Z",
                            "html_url": f"https://github.com/example/new-api/releases/tag/{tag}",
                            "body": f"{tag} 的更新说明：修复若干问题。"})
            return self.send(200, list(reversed(out)))
        if self.path == "/openrouter/api/v1/models":
            table = {k: dict(v) for k, v in OPENROUTER.items() if k not in self.faults.get("or_gone", [])}
            for spec in self.faults.get("or_price", []):
                mid, _, price = spec.partition(":")
                if mid in table:
                    table[mid]["prompt"] = float(price)
            data = [{"id": k, "name": k, "pricing": {f: f"{v / 1e6:.12f}" for f, v in prices.items()}}
                    for k, prices in table.items()]
            return self.send(200, {"data": data})
        if self.path == "/upstream/v1/models":
            if self.headers.get("Authorization") != "Bearer " + UPSTREAM_TOKEN:
                return self.send(401, {"error": {"message": "bad upstream key"}})
            return self.send(200, {"object": "list", "data": [{"id": m, "object": "model"} for m in self.upstream_models()]})
        if self.path == "/api/status":
            if "status_down" in self.faults:
                return self.send(502, {"error": "bad gateway"})
            version = (self.faults.get("version") or [VERSION])[-1]
            return self.send(200, {"success": True, "data": {"version": version, "system_name": "Fake Gateway"}})
        if self.path == "/api/pricing":
            data = [{"model_name": k, **v, "enable_groups": ["default"]} for k, v in self.models().items()]
            return self.send(200, {"success": True, "data": data})
        if self.path == "/v1/models":
            if not self.authed() and "open_models" not in self.faults:
                return self.send(401, {"error": {"message": "未提供令牌", "type": "new_api_error"}})
            return self.send(200, {"object": "list", "data": [{"id": k, "object": "model"} for k in self.models()]})
        return self.send(404, {"error": "not found"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        req = json.loads(self.rfile.read(length) or b"{}")
        if self.path == "/upstream/v1/chat/completions":
            if self.headers.get("Authorization") != "Bearer " + UPSTREAM_TOKEN:
                return self.send(401, {"error": {"message": "bad upstream key"}})
            model = req.get("model", "")
            if model not in self.upstream_models() or "broken" in model:
                return self.send(404, {"error": {"message": f"model {model} not found", "type": "invalid_request_error"}})
            return self.send(200, {"id": "u", "object": "chat.completion", "model": model,
                                   "choices": [{"index": 0, "message": {"role": "assistant", "content": "p"}}],
                                   "usage": {"prompt_tokens": 8, "completion_tokens": 1, "total_tokens": 9}})
        if self.path != "/v1/chat/completions":
            return self.send(404, {"error": "not found"})
        if not self.authed():
            return self.send(401, {"error": {"message": "无效的令牌", "type": "new_api_error"}})
        model = req.get("model")
        if model not in self.models():
            return self.send(503, {"error": {"message": f"当前分组下没有可用渠道：{model}", "type": "new_api_error"}})
        if model in self.faults.get("upstream_auth", []):
            return self.send(500, {"error": {"message": "upstream authentication failed (401)", "type": "upstream_error"}})
        if model in self.faults.get("timeout524", []):
            time.sleep(2)
            return self.send(524, {"error": {"message": "A timeout occurred", "type": "cdn"}})
        for spec in self.faults.get("slow", []):
            m, _, secs = spec.partition(":")
            if m == model:
                time.sleep(float(secs))
        return self.send(200, {
            "id": "chatcmpl-fake", "object": "chat.completion", "model": model,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "p"}, "finish_reason": "length"}],
            "usage": {"prompt_tokens": 8, "completion_tokens": 1, "total_tokens": 9},
        })


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=18080)
    p.add_argument("--fault", action="append", default=[])
    p.add_argument("--release", action="append", default=[])
    a = p.parse_args()
    Handler.faults = parse_faults(a.fault)
    Handler.releases = a.release
    server = ThreadingHTTPServer((a.host, a.port), Handler)
    print(f"假中转站在 http://{a.host}:{server.server_address[1]}，故障：{a.fault or '无'}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
