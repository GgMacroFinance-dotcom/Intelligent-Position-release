#!/usr/bin/env python3
"""回声假模型：把收到的系统提示词和对话原样当回复回出来，不连任何上游，不需要密钥。

只给 evals/team-context/run.sh 的 --echo 用：不花钱地确认团队上下文进了提示词、评测的检查能判出来。
接上团队上下文时，回复里会有上下文包里的条目，必含项大多能对上；不接时对不上。
它不懂题目，也不会调工具，分数不说明任何模型的好坏。

  python3 echo_model.py --port 18091

只实现 OpenAI 兼容的 POST /v1/chat/completions（流式）。
"""
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(part.get("text", "") for part in content if isinstance(part, dict))
    return ""


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        if not self.path.endswith("/chat/completions"):
            self.send_error(404)
            return
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        reply = "\n\n".join(text_of(m.get("content")) for m in body.get("messages", []))
        chunks = [
            {"choices": [{"index": 0, "delta": {"role": "assistant", "content": reply}}]},
            {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
            {"choices": [], "usage": {"prompt_tokens": len(reply), "completion_tokens": len(reply), "total_tokens": 2 * len(reply)}},
        ]
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for chunk in chunks:
            self.wfile.write(f"data: {json.dumps(chunk, ensure_ascii=False)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=18091)
    args = parser.parse_args()
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
