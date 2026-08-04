from __future__ import annotations

import argparse
import os
import subprocess
import re
from pathlib import Path

from _common import JsonArgumentParser, emit, main_guard, minecraft_status_ping, parse_cli_root


def read_properties(path: Path) -> dict[str, str]:
    values = {}
    if path.is_file():
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.strip() and not line.lstrip().startswith(("#", "!")) and "=" in line:
                key, value = line.split("=", 1)
                values[key.strip()] = value.strip()
    return values


def process_rows(timeout: float = 10.0) -> list[dict]:
    if os.name == "nt":
        script = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); Get-CimInstance Win32_Process | Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress"
        try:
            completed = subprocess.run(
                ["powershell", "-NoProfile", "-Command", script],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=timeout,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return []
        if completed.returncode != 0 or not completed.stdout.strip():
            return []
        import json
        raw = json.loads(completed.stdout)
        items = raw if isinstance(raw, list) else [raw]
        return [{"pid": item.get("ProcessId"), "name": item.get("Name"), "command": item.get("CommandLine") or ""} for item in items]
    rows = []
    proc = Path("/proc")
    if proc.is_dir():
        for item in proc.iterdir():
            if not item.name.isdigit():
                continue
            try:
                command = (item / "cmdline").read_bytes().replace(b"\x00", b" ").decode(errors="replace").strip()
                name = (item / "comm").read_text(errors="replace").strip()
                rows.append({"pid": int(item.name), "name": name, "command": command})
            except (OSError, ValueError):
                continue
    return rows


def run() -> int:
    parser = JsonArgumentParser(description="Inspect Minecraft server process, port, and log readiness")
    parser.add_argument("server_root")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--handshake-host", help="Virtual host sent in the Minecraft handshake; defaults to --host")
    parser.add_argument("--protocol-version", type=int, default=47)
    parser.add_argument("--minecraft-status", action="store_true", help="Explicitly perform a Minecraft status protocol probe")
    parser.add_argument("--timeout", type=float, default=1.5)
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    props = read_properties(root / "server.properties")
    port = int(props.get("server-port", "25565"))
    jar_names = {path.name.lower() for path in root.glob("*.jar")}
    root_text = str(root).lower()
    candidates = []
    for row in process_rows(min(max(args.timeout * 2, 1.0), 10.0)):
        command = row["command"].lower()
        if "java" not in (row["name"] or "").lower() and "java" not in command:
            continue
        evidence = []
        if root_text in command:
            evidence.append("server-root-in-command")
        matched = sorted(name for name in jar_names if name in command)
        if matched:
            evidence.append("server-jar-in-command")
        if evidence:
            candidates.append({**row, "evidence": evidence, "matched_jars": matched})
    probe = minecraft_status_ping(args.host, port, args.timeout, handshake_host=args.handshake_host, protocol_version=args.protocol_version) if args.minecraft_status else None
    port_open = probe["tcp_open"] if probe else None
    latest = root / "logs" / "latest.log"
    ready = None
    fatal = None
    cycle = {"confidence": "unknown", "marker": None}
    if latest.is_file():
        tail = latest.read_text(encoding="utf-8", errors="replace")[-500_000:]
        marker = re.compile(r"Starting minecraft server version|Starting Velocity|Starting BungeeCord|Booting up Velocity|Loading Minecraft .* with Fabric Loader|ModLauncher running", re.I)
        starts = list(marker.finditer(tail))
        if starts:
            selected = tail[starts[-1].start() :]
            ready = bool(re.search(r"Done \([^\r\n]+\)!|Done!|Listening on /?[^\s:]+:\d+", selected, re.I))
            fatal = any(value.lower() in selected.lower() for value in ("Failed to bind to port", "Failed to start the minecraft server", "OutOfMemoryError"))
            cycle = {"confidence": "inferred", "marker": starts[-1].group(0)}
    if fatal:
        state = "failed"
    elif probe and probe["minecraft_status"] == "confirmed" and candidates and ready:
        state = "ready"
    elif candidates:
        state = "starting-or-running"
    elif port_open is True:
        state = "unknown-listener"
    else:
        state = "stopped-or-undetected"
    emit({"ok": True, "state": state, "server_root": str(root), "port": {"host": args.host, "number": port, "open": port_open, "probe": probe, "network_probe_performed": bool(args.minecraft_status)}, "process_candidates": candidates, "log": {"present": latest.is_file(), "cycle": cycle, "ready_signal": ready, "fatal_signal": fatal}, "analysis_only": True, "writes_performed": False, "note": "Protocol confirmation proves an endpoint response, not process ownership; lifecycle execution belongs to an external operator."}, args.pretty)
    return 0


if __name__ == "__main__":
    main_guard(run)
