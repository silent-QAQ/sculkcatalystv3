from __future__ import annotations

import argparse
import re
from pathlib import Path

from _common import JsonArgumentParser, emit, limited_files, load_cycle_baseline, main_guard, parse_cli_root, read_stable_segment, rel


RULES = [
    ("critical", "out-of-memory", re.compile(r"OutOfMemoryError|GC overhead limit exceeded", re.I), "JVM or native memory exhausted; inspect heap, GC, workload, and container limits."),
    ("critical", "watchdog", re.compile(r"A single server tick took|Server thread dump|Watchdog", re.I), "The main/server thread stalled; inspect the first thread dump and profiler evidence."),
    ("error", "port-in-use", re.compile(r"Address already in use|Perhaps a server is already running", re.I), "The configured listen port is occupied or another server instance owns it."),
    ("error", "java-version", re.compile(r"UnsupportedClassVersionError|class file version \d+\.0", re.I), "The runtime Java version is incompatible with a loaded class."),
    ("error", "missing-dependency", re.compile(r"UnknownDependencyException|Missing mandatory dependencies|depends on .* which is not installed", re.I), "A plugin or mod dependency is missing or failed earlier."),
    ("error", "mixin-failure", re.compile(r"Mixin apply failed|MixinTransformerError|InvalidMixinException", re.I), "A mod mixin is incompatible with the current game, loader, or another mod."),
    ("error", "plugin-load", re.compile(r"Could not load .*\.jar|Could not load plugin|Error occurred while enabling", re.I), "A plugin failed to load or enable; inspect its earliest caused-by chain."),
    ("error", "world-corruption", re.compile(r"Exception reading chunk|Failed to load level data|Chunk file .* is in the wrong location|(?:region|chunk|level).{0,40}corrupt", re.I), "World data may be damaged; confirm on a copy before repair."),
    ("warning", "disk-space", re.compile(r"No space left on device|There is not enough space on the disk", re.I), "The filesystem lacks free space; stop writes and recover space safely."),
    ("warning", "authentication", re.compile(r"Failed to verify username|Authentication servers are down", re.I), "Authentication connectivity or online-mode handling failed."),
]

START_MARKER = re.compile(
    r"Starting minecraft server version|Starting Minecraft server on|Loading libraries, please wait|Booting up server|Starting Velocity|Starting BungeeCord|Loading Minecraft .* with Fabric Loader|ModLauncher running",
    re.I,
)


def select_window(text: str, scope: str) -> tuple[list[str], int, bool]:
    lines = text.splitlines()
    if scope == "all":
        return lines, 1, False
    indexes = [index for index, line in enumerate(lines) if START_MARKER.search(line)]
    if not indexes:
        return lines, 1, False
    start = indexes[-1]
    return lines[start:], start + 1, True


def run() -> int:
    parser = JsonArgumentParser(description="Diagnose common Minecraft server failures from logs")
    parser.add_argument("server_root")
    parser.add_argument("--max-bytes", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--scope", choices=("latest-launch", "all"), default="latest-launch")
    parser.add_argument("--log-offset", type=int, help="Read latest.log from this byte offset; overrides latest-launch selection for that file")
    parser.add_argument("--cycle-baseline", help="Baseline JSON created by capture-baseline.py")
    parser.add_argument("--context", type=int, default=2)
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    if args.log_offset is not None and args.cycle_baseline:
        raise ValueError("use either --log-offset or --cycle-baseline, not both")
    files = limited_files(root, ("logs/latest.log", "logs/debug.log", "crash-reports/*.txt", "hs_err_pid*.log", "*.log"), 100)
    findings = []
    historical_findings = []
    scopes = []
    evidence_complete = True
    baseline_state_global = None
    segment_overrides = {}
    baseline_budget = args.max_bytes
    if args.cycle_baseline:
        latest = root / "logs" / "latest.log"
        if not latest.is_file():
            raise ValueError("cycle baseline requires logs/latest.log")
        baseline_state_global = load_cycle_baseline(root, Path(args.cycle_baseline).expanduser().resolve(strict=True), latest)
        if baseline_state_global["valid"]:
            for segment in baseline_state_global["segments"]:
                segment_path = Path(segment["path"]).resolve(strict=True)
                segment_overrides[segment_path] = segment
                if segment_path not in files:
                    files.append(segment_path)
            files = sorted(set(files))
        else:
            evidence_complete = False
    for path in files:
        size = path.stat().st_size
        start_offset = max(0, size - args.max_bytes)
        explicit_boundary = False
        baseline_state = baseline_state_global if path.resolve() in segment_overrides or (args.cycle_baseline and rel(root, path) == "logs/latest.log") else None
        segment = segment_overrides.get(path.resolve())
        if segment:
            start_offset = int(segment["start"])
            explicit_boundary = True
        elif args.log_offset is not None and rel(root, path) == "logs/latest.log":
            if args.log_offset < 0 or args.log_offset > size:
                raise ValueError(f"log offset outside file: {args.log_offset}")
            start_offset = args.log_offset
            explicit_boundary = True
        previous = b"\n"
        if segment:
            segment_length = int(segment["end"]) - int(segment["start"])
            if segment_length > baseline_budget:
                evidence_complete = False
                raw = b""
            else:
                try:
                    raw, _ = read_stable_segment(segment, baseline_budget)
                    baseline_budget -= len(raw)
                except ValueError:
                    evidence_complete = False
                    raw = b""
            if start_offset:
                with path.open("rb") as handle:
                    handle.seek(start_offset - 1)
                    previous = handle.read(1)
        else:
            with path.open("rb") as handle:
                if start_offset:
                    handle.seek(start_offset - 1)
                    previous = handle.read(1)
                handle.seek(start_offset)
                raw = handle.read(args.max_bytes)
        dropped_partial = False
        if start_offset and previous not in {b"\n", b"\r"} and b"\n" in raw:
            raw = raw.split(b"\n", 1)[1]
            dropped_partial = True
        text = raw.decode("utf-8", errors="replace")
        prefix_lines = 0
        if start_offset:
            with path.open("rb") as prefix:
                remaining = start_offset
                while remaining:
                    chunk = prefix.read(min(1024 * 1024, remaining))
                    if not chunk:
                        break
                    prefix_lines += chunk.count(b"\n")
                    remaining -= len(chunk)
        use_launch_scope = args.scope if rel(root, path) in {"logs/latest.log", "logs/debug.log"} and not explicit_boundary else "all"
        lines, selected_line, marker_found = select_window(text, use_launch_scope)
        line_base = prefix_lines + selected_line + (1 if dropped_partial else 0)
        scopes.append({"file": rel(root, path), "byte_offset": start_offset, "first_line": line_base, "latest_launch_marker_found": marker_found, "cycle_baseline": baseline_state, "lines": len(lines)})
        historical = path.resolve() not in segment_overrides and ("crash-reports" in path.parts or path.name.startswith("hs_err_pid"))
        for severity, code, pattern, explanation in RULES:
            matches = [(index, line.strip()[:500]) for index, line in enumerate(lines) if pattern.search(line)]
            if matches:
                samples = []
                for index, line in matches[:3]:
                    low = max(0, index - args.context)
                    high = min(len(lines), index + args.context + 1)
                    samples.append({"line": line_base + index, "text": line, "context": [{"line": line_base + pos, "text": lines[pos].strip()[:500]} for pos in range(low, high)]})
                finding = {"severity": severity, "code": code, "file": rel(root, path), "occurrences": len(matches), "samples": samples, "interpretation": explanation}
                (historical_findings if historical else findings).append(finding)
    severity_order = {"critical": 0, "error": 1, "warning": 2}
    findings.sort(key=lambda item: (severity_order[item["severity"]], item["file"], item["code"]))
    historical_findings.sort(key=lambda item: (severity_order[item["severity"]], item["file"], item["code"]))
    failed = bool(not evidence_complete or findings or (args.scope == "all" and historical_findings))
    emit({"ok": not failed, "scope": args.scope, "evidence_complete": evidence_complete, "files_scanned": scopes, "findings": findings, "historical_findings": historical_findings, "note": "An invalid cycle baseline creates an evidence gap and fails closed."}, args.pretty)
    return 1 if failed else 0


if __name__ == "__main__":
    main_guard(run)
