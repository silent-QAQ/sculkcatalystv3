# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

"""Small throttled Range server used by provisioning restart E2E tests."""

from __future__ import annotations

import argparse
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit


def handler_for(root: Path, delay_seconds: float):
    class SlowFileHandler(BaseHTTPRequestHandler):
        server_version = "SculkE2E/1"

        def do_HEAD(self) -> None:  # noqa: N802 - HTTP verb naming
            self._serve(False)

        def do_GET(self) -> None:  # noqa: N802 - HTTP verb naming
            self._serve(True)

        def _serve(self, include_body: bool) -> None:
            name = Path(unquote(urlsplit(self.path).path)).name
            target = (root / name).resolve()
            if not name or target.parent != root or not target.is_file():
                self.send_error(404)
                return

            size = target.stat().st_size
            start = 0
            range_header = self.headers.get("Range", "")
            if range_header.startswith("bytes="):
                raw_start = range_header[6:].split("-", 1)[0]
                try:
                    start = int(raw_start)
                except ValueError:
                    self.send_error(400)
                    return
                if start >= size:
                    self.send_response(416)
                    self.send_header("Content-Range", f"bytes */{size}")
                    self.end_headers()
                    return

            status = 206 if start else 200
            self.send_response(status)
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Type", "application/java-archive")
            self.send_header("Content-Length", str(size - start))
            if start:
                self.send_header("Content-Range", f"bytes {start}-{size - 1}/{size}")
            self.end_headers()
            if not include_body:
                return

            with target.open("rb") as source:
                source.seek(start)
                while chunk := source.read(64 * 1024):
                    try:
                        self.wfile.write(chunk)
                        self.wfile.flush()
                    except (BrokenPipeError, ConnectionResetError):
                        return
                    time.sleep(delay_seconds)

        def log_message(self, _format: str, *_args: object) -> None:
            return

    return SlowFileHandler


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--delay-ms", type=int, default=10)
    args = parser.parse_args()
    root = args.root.resolve()
    server = ThreadingHTTPServer(
        ("127.0.0.1", args.port), handler_for(root, args.delay_ms / 1000)
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
