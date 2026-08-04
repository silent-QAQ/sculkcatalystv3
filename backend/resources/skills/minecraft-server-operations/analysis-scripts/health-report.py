from __future__ import annotations

import json
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, main_guard, parse_cli_root, sha256_file


RANK = {"info": 0, "warning": 1, "error": 2, "critical": 3, "unknown": 4}


def child(name: str, arguments: list[str], timeout: float) -> dict[str, Any]:
    script = Path(__file__).with_name(name)
    started = time.monotonic()
    try:
        result = subprocess.run([sys.executable, str(script), *arguments], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout, check=False)
        value = json.loads(result.stdout)
        if not isinstance(value, dict):
            raise ValueError("child output is not an object")
        return {"available": True, "returncode": result.returncode, "duration_ms": round((time.monotonic() - started) * 1000, 2), "payload": value}
    except subprocess.TimeoutExpired:
        return {"available": False, "reason": "timeout", "duration_ms": round((time.monotonic() - started) * 1000, 2)}
    except (json.JSONDecodeError, ValueError) as exc:
        return {"available": False, "reason": f"invalid-child-output: {exc}", "duration_ms": round((time.monotonic() - started) * 1000, 2)}


def check_disk(root: Path, args: Any) -> dict[str, Any]:
    usage = shutil.disk_usage(root)
    free_percent = usage.free / usage.total * 100 if usage.total else 0.0
    critical = free_percent <= args.disk_critical_percent or usage.free <= args.disk_critical_bytes
    warning = free_percent <= args.disk_warning_percent or usage.free <= args.disk_warning_bytes
    severity = "critical" if critical else "warning" if warning else "info"
    return {"id": "disk", "status": "fail" if critical else "pass" if not warning else "fail", "severity": severity, "summary": f"{usage.free} bytes free ({free_percent:.2f}%)", "evidence": {"total": usage.total, "used": usage.used, "free": usage.free, "free_percent": round(free_percent, 2)}}


def check_backup(root: Path, args: Any) -> dict[str, Any]:
    backup_root = root / ".agent-backups"
    candidates = sorted((path for path in backup_root.glob("*/manifest.json") if not path.parent.name.startswith(".incomplete-")), key=lambda path: path.stat().st_mtime, reverse=True)[:100] if backup_root.is_dir() else []
    valid = []
    rejected = []
    for manifest_path in candidates:
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            if manifest.get("schema") != 2 or not isinstance(manifest.get("entries"), list):
                raise ValueError("unsupported manifest schema")
            if Path(str(manifest.get("server_root", ""))).resolve() != root.resolve():
                raise ValueError("manifest belongs to another server root")
            for entry in manifest["entries"]:
                relative = entry.get("path")
                if not isinstance(relative, str) or Path(relative).is_absolute() or ".." in Path(relative).parts:
                    raise ValueError("unsafe manifest path")
                backup_file = manifest_path.parent / "files" / relative
                if entry.get("existed", True):
                    if not backup_file.is_file() or backup_file.stat().st_size != int(entry.get("size", -1)):
                        raise ValueError(f"missing or wrong-sized backup file: {relative}")
                    if args.deep and entry.get("sha256") and sha256_file(backup_file) != entry["sha256"]:
                        raise ValueError(f"backup hash mismatch: {relative}")
            valid.append((manifest_path, manifest))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            rejected.append({"manifest": str(manifest_path), "reason": str(exc)})
    if not valid:
        severity = "error" if candidates and rejected else "warning"
        return {"id": "backup", "status": "fail", "severity": severity, "summary": "no valid completed operation backup found", "evidence": {"candidates": len(candidates), "rejected": rejected, "deep_verified": args.deep}}
    latest_path, latest = valid[0]
    age_hours = (time.time() - latest_path.stat().st_mtime) / 3600
    severity = "critical" if age_hours >= args.backup_critical_hours else "warning" if age_hours >= args.backup_warning_hours else "info"
    return {"id": "backup", "status": "pass" if severity == "info" else "fail", "severity": severity, "summary": f"latest valid backup is {age_hours:.2f} hours old", "evidence": {"manifest": str(latest_path), "operation_id": latest.get("operation_id"), "age_hours": round(age_hours, 2), "files": len(latest["entries"]), "deep_verified": args.deep, "rejected": rejected}}


def run() -> int:
    parser = JsonArgumentParser(description="Aggregate Minecraft server health and protection evidence")
    parser.add_argument("server_root")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--handshake-host")
    parser.add_argument("--protocol-version", type=int, default=47)
    parser.add_argument("--network", action="store_true", help="Explicitly allow a Minecraft status protocol probe")
    parser.add_argument("--cycle-baseline")
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--probe-timeout", type=float, default=1.5)
    parser.add_argument("--backup-warning-hours", type=float, default=24.0)
    parser.add_argument("--backup-critical-hours", type=float, default=72.0)
    parser.add_argument("--disk-warning-percent", type=float, default=10.0)
    parser.add_argument("--disk-critical-percent", type=float, default=3.0)
    parser.add_argument("--disk-warning-bytes", type=int, default=5 * 1024**3)
    parser.add_argument("--disk-critical-bytes", type=int, default=1024**3)
    parser.add_argument("--deep", action="store_true")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    if args.timeout <= 0 or args.probe_timeout <= 0:
        raise ValueError("timeouts must be positive")
    if args.backup_critical_hours < args.backup_warning_hours or args.disk_critical_percent > args.disk_warning_percent or args.disk_critical_bytes > args.disk_warning_bytes:
        raise ValueError("critical thresholds must be stricter than warning thresholds")
    root = parse_cli_root(args.server_root)
    started = time.monotonic()
    checks = [check_disk(root, args), check_backup(root, args)]
    limitations = []

    def remaining(limit: float = 12.0) -> float:
        value = args.timeout - (time.monotonic() - started)
        if value <= 0:
            raise TimeoutError("health report global deadline exceeded")
        return min(value, limit)

    diagnose_args = [str(root)]
    if args.cycle_baseline:
        diagnose_args += ["--cycle-baseline", args.cycle_baseline]
    else:
        limitations.append("current log cycle is inferred from the latest recognized start marker")
    diagnosis = child("diagnose-server.py", diagnose_args, remaining())
    if not diagnosis["available"]:
        checks.append({"id": "current-cycle", "status": "unknown", "severity": "unknown", "summary": diagnosis["reason"], "evidence": diagnosis})
    else:
        payload = diagnosis["payload"]
        findings = payload.get("findings", [])
        if not payload.get("evidence_complete", True):
            severity = "unknown"
            status = "unknown"
        elif findings:
            severity = max((str(item.get("severity", "warning")) for item in findings), key=lambda item: RANK.get(item, 1))
            status = "fail"
        else:
            severity = "info"
            status = "pass"
        checks.append({"id": "current-cycle", "status": status, "severity": severity, "summary": f"{len(findings)} current-cycle findings", "evidence": payload})

    status_args = [str(root), "--host", args.host, "--timeout", str(args.probe_timeout), "--protocol-version", str(args.protocol_version)]
    if args.network:
        status_args.append("--minecraft-status")
    if args.handshake_host:
        status_args += ["--handshake-host", args.handshake_host]
    status_result = child("server-status.py", status_args, remaining())
    if not status_result["available"]:
        checks.append({"id": "runtime", "status": "unknown", "severity": "unknown", "summary": status_result["reason"], "evidence": status_result})
    else:
        payload = status_result["payload"]
        state = payload.get("state")
        severity = "critical" if state == "failed" else "warning" if state == "unknown-listener" else "info"
        checks.append({"id": "runtime", "status": "fail" if severity != "info" else "pass", "severity": severity, "summary": str(state), "evidence": payload})

    supervisor = child("inspect-supervisor.py", [str(root), "--timeout", str(min(args.probe_timeout * 2, 10.0))], remaining())
    if not supervisor["available"]:
        checks.append({"id": "supervisor", "status": "unknown", "severity": "unknown", "summary": supervisor["reason"], "evidence": supervisor})
    else:
        payload = supervisor["payload"]
        severity = "error" if payload.get("conflicts") else "info" if payload.get("selected") else "warning" if payload.get("supervisors") else "info"
        checks.append({"id": "supervisor", "status": "fail" if severity in {"warning", "error"} else "pass", "severity": severity, "summary": "ownership evidence complete" if payload.get("ownership_evidence_complete") else "ownership evidence incomplete", "evidence": payload})

    counts = {key: sum(1 for check in checks if check["severity"] == key) for key in RANK}
    has_unknown = counts["unknown"] > 0
    highest = max((check["severity"] for check in checks), key=lambda item: RANK[item])
    if has_unknown:
        overall, code = "unknown", 2
    elif RANK[highest] >= RANK["error"]:
        overall, code = "unhealthy", 1
    elif counts["warning"]:
        overall, code = "degraded", 1
    else:
        overall, code = "healthy", 0
    emit({"schema": 1, "ok": code == 0, "overall": overall, "severity": highest, "server_root": str(root), "mode": "deep" if args.deep else "standard", "duration_ms": round((time.monotonic() - started) * 1000, 2), "summary": counts, "checks": checks, "limitations": limitations}, args.pretty)
    return code


if __name__ == "__main__":
    main_guard(run)
