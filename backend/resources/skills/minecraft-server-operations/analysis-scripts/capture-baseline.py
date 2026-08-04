from __future__ import annotations

from pathlib import Path

from _common import JsonArgumentParser, emit, log_identity, main_guard, parse_cli_root, utc_now


def run() -> int:
    parser = JsonArgumentParser(description="Capture a pre-start latest.log baseline")
    parser.add_argument("server_root")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    log = root / "logs" / "latest.log"
    if log.exists() and not log.is_file():
        raise ValueError(f"latest.log is not a regular file: {log}")
    identity = log_identity(log) if log.is_file() else {"path": "latest.log", "device": None, "inode": None, "size": 0, "mtime_ns": None}
    payload = {"schema": 1, "captured_at": utc_now(), "server_root": str(root), "log": identity}
    emit({"ok": True, "baseline": payload, "analysis_only": True, "writes_performed": False}, args.pretty)
    return 0


if __name__ == "__main__":
    main_guard(run)
