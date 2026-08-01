# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
"""Minimal OpenAI-compatible relay used only by the isolated CI smoke test."""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class RelayHandler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            request = json.loads(self.rfile.read(length))
        except (ValueError, json.JSONDecodeError):
            self.send_error(400)
            return
        response = {
            "id": "chatcmpl-sculk-ci",
            "object": "chat.completion",
            "created": 0,
            "model": request.get("model", "mock-gpt-mini"),
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": "relay mock ok"},
                    "finish_reason": "stop",
                }
            ],
            "usage": {"prompt_tokens": 11, "completion_tokens": 20, "total_tokens": 31},
        }
        encoded = json.dumps(response).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=9944)
    args = parser.parse_args()
    ThreadingHTTPServer((args.host, args.port), RelayHandler).serve_forever()


if __name__ == "__main__":
    main()
