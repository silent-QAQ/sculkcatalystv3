from __future__ import annotations

import argparse
import socket
import re
from pathlib import Path

from _common import JsonArgumentParser, emit, load_cycle_baseline, main_guard, minecraft_status_ping, parse_cli_root, read_stable_segment


def properties(path: Path) -> dict[str, str]:
    result = {}
    if path.is_file():
        for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
            if raw.strip() and not raw.lstrip().startswith(("#", "!")) and "=" in raw:
                key, value = raw.split("=", 1)
                result[key.strip()] = value.strip()
    return result


def run() -> int:
    parser = JsonArgumentParser(description="Perform static and optional TCP health checks")
    parser.add_argument("server_root")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, help="Override server.properties port, useful for proxies")
    parser.add_argument("--tcp", action="store_true", help="Require a TCP connection to the configured server port")
    parser.add_argument("--minecraft-status", action="store_true", help="Require a valid Java Server List Ping response")
    parser.add_argument("--handshake-host", help="Virtual host sent in the Minecraft handshake; defaults to --host")
    parser.add_argument("--protocol-version", type=int, default=47)
    parser.add_argument("--require-ready", action="store_true", help="Require a readiness marker after the selected log boundary")
    parser.add_argument("--log-offset", type=int, help="Byte offset marking the start of this launch; default selects the latest launch marker")
    parser.add_argument("--cycle-baseline", help="Baseline JSON created by capture-baseline.py")
    parser.add_argument("--timeout", type=float, default=3.0)
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    checks = []
    props_path = root / "server.properties"
    props = properties(props_path)
    if props_path.exists():
        checks.append({"name": "server.properties", "ok": props_path.is_file(), "detail": str(props_path)})
    jars = list(root.glob("*.jar"))
    checks.append({"name": "server-jar", "ok": bool(jars), "detail": [path.name for path in jars]})
    port = args.port if args.port is not None else int(props.get("server-port", "25565"))
    if args.log_offset is not None and args.cycle_baseline:
        raise ValueError("use either --log-offset or --cycle-baseline, not both")
    if args.tcp:
        try:
            with socket.create_connection((args.host, port), timeout=args.timeout):
                tcp_ok = True
                detail = f"connected to {args.host}:{port}"
        except OSError as exc:
            tcp_ok = False
            detail = str(exc)
        checks.append({"name": "tcp", "ok": tcp_ok, "detail": detail})
    if args.minecraft_status:
        probe = minecraft_status_ping(args.host, port, args.timeout, handshake_host=args.handshake_host, protocol_version=args.protocol_version)
        checks.append({"name": "minecraft-status", "ok": probe["minecraft_status"] == "confirmed", "detail": probe})
    if args.require_ready:
        latest = root / "logs" / "latest.log"
        if not latest.is_file():
            checks.append({"name": "readiness", "ok": False, "detail": "logs/latest.log is missing"})
        else:
            if args.cycle_baseline:
                baseline_state = load_cycle_baseline(root, Path(args.cycle_baseline).expanduser().resolve(strict=True), latest)
                if baseline_state["valid"]:
                    chunks = []
                    budget = 2 * 1024 * 1024
                    segment_evidence = []
                    try:
                        for segment in baseline_state["segments"]:
                            chunk, evidence = read_stable_segment(segment, budget)
                            budget -= len(chunk)
                            chunks.append(chunk.decode("utf-8", errors="replace"))
                            segment_evidence.append(evidence)
                        selected = "\n".join(chunks)
                        boundary = {"type": "cycle-baseline", **baseline_state, "segments_read": segment_evidence, "evidence_complete": True}
                    except ValueError as exc:
                        selected = ""
                        boundary = {"type": "invalid-cycle-baseline", **baseline_state, "evidence_complete": False, "read_error": str(exc)}
                else:
                    selected = ""
                    boundary = {"type": "invalid-cycle-baseline", **baseline_state, "evidence_complete": False}
            elif args.log_offset is not None:
                raw = latest.read_bytes()
                if args.log_offset < 0 or args.log_offset > len(raw):
                    raise ValueError(f"log offset outside file: {args.log_offset}")
                selected = raw[args.log_offset:].decode("utf-8", errors="replace")
                boundary = {"type": "byte-offset", "value": args.log_offset}
            else:
                with latest.open("rb") as handle:
                    size = latest.stat().st_size
                    handle.seek(max(0, size - 2 * 1024 * 1024))
                    text = handle.read(2 * 1024 * 1024).decode("utf-8", errors="replace")
                marker = re.compile(r"Starting minecraft server version|Starting Velocity|Starting BungeeCord|Loading Minecraft .* with Fabric Loader|ModLauncher running", re.I)
                starts = [match.start() for match in marker.finditer(text)]
                selected = text[starts[-1] :] if starts else ""
                boundary = {"type": "latest-launch-marker" if starts else "unknown", "value": starts[-1] if starts else None}
            ready = bool(selected) and bool(re.search(r"Done \([^\r\n]+\)!|Done!|Listening on /?[^\s:]+:\d+", selected, re.I))
            fatal_tokens = ("Failed to bind to port", "OutOfMemoryError", "Failed to start the minecraft server")
            fatal = [token for token in fatal_tokens if token.lower() in selected.lower()]
            checks.append({"name": "readiness", "ok": ready and not fatal, "ready_signal": ready if selected else None, "fatal_signals": fatal if selected else None, "boundary": boundary})
    ok = all(check["ok"] for check in checks)
    emit({"ok": ok, "server_root": str(root), "checks": checks}, args.pretty)
    return 0 if ok else 1


if __name__ == "__main__":
    main_guard(run)
