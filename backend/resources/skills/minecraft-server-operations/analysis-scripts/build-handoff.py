from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, main_guard, operation_id, parse_cli_root, utc_now


def run_analysis(name: str, root: Path, timeout: float) -> dict[str, Any]:
    result = subprocess.run(
        [sys.executable, str(Path(__file__).with_name(name)), str(root)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        check=False,
    )
    if result.returncode not in {0, 1}:
        return {"collection_status": "failed", "returncode": result.returncode, "error": result.stderr[:4096] or "analysis failed"}
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError:
        return {"collection_status": "failed", "returncode": result.returncode, "error": "invalid JSON output"}
    return {"collection_status": "complete", "returncode": result.returncode, "payload": payload}


def recommendation(intent: str, diagnosis: dict[str, Any], supervisor: dict[str, Any]) -> dict[str, Any]:
    findings = diagnosis.get("payload", {}).get("findings", []) if diagnosis.get("collection_status") == "complete" else []
    codes = {str(item.get("code")) for item in findings}
    if "out-of-memory" in codes:
        summary = "The external executor should verify memory limits, JVM flags, GC behavior, and leak evidence before changing memory settings or isolating components."
        confidence = "high"
    elif {"port-bind-failed", "port-in-use"} & codes:
        summary = "The external executor should verify the listening-port owner and supervisor identity; starting a second instance is not a valid port-conflict remedy."
        confidence = "high"
    elif findings:
        summary = "The external executor should address the earliest current-cycle failure first and avoid changes aimed only at downstream symptoms."
        confidence = "medium"
    else:
        summary = "Current evidence does not establish a specific failure. Resolve blocking unknowns before any state-changing operation."
        confidence = "low"
    selected = supervisor.get("payload", {}).get("selected") if supervisor.get("collection_status") == "complete" else None
    return {
        "id": "action-1",
        "decision": "conditional",
        "intent": intent,
        "summary": summary,
        "confidence": {"level": confidence, "basis": "current-cycle findings and supervisor evidence"},
        "target_supervisor": {"kind": selected.get("kind"), "id": selected.get("id")} if isinstance(selected, dict) else None,
        "execution": {"performed": False, "role": "external-executor", "handoff_is_authorization": False},
        "preconditions": [
            {"check": "server-root-identity-unchanged", "on_failure": "reanalyse"},
            {"check": "supervisor-and-process-identity-unchanged", "on_failure": "abort"},
            {"check": "required-backup-verified-by-executor", "on_failure": "abort"},
        ],
    }


def run() -> int:
    parser = JsonArgumentParser(description="Build an analysis-only Minecraft operations handoff")
    parser.add_argument("server_root")
    parser.add_argument("--intent", choices=("diagnose", "change", "restart", "upgrade", "restore", "performance"), default="diagnose")
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    if args.timeout <= 0:
        raise ValueError("timeout must be positive")
    root = parse_cli_root(args.server_root)
    identity = root.stat()
    inventory = run_analysis("inspect-server.py", root, args.timeout)
    diagnosis = run_analysis("diagnose-server.py", root, args.timeout)
    supervisor = run_analysis("inspect-supervisor.py", root, args.timeout)
    evidence = [
        {"id": "inventory", "type": "server-inventory", **inventory},
        {"id": "diagnosis", "type": "current-cycle-diagnosis", **diagnosis},
        {"id": "supervisor", "type": "supervisor-evidence", **supervisor},
    ]
    payload = {
        "schema": "minecraft-ops-handoff/v1",
        "handoff_id": operation_id(),
        "created_at": utc_now(),
        "producer": {"name": "manage-minecraft-server", "mode": "analysis-only"},
        "intent": {"goal": args.intent, "authorization": "not-conveyed"},
        "target": {"server_root": str(root), "root_identity": {"device": identity.st_dev, "inode": identity.st_ino}},
        "evidence": evidence,
        "recommended_actions": [recommendation(args.intent, diagnosis, supervisor)],
        "risks": [{"id": "state-drift", "severity": "high", "description": "Evidence can become stale before external execution."}],
        "rollback_plan": {"available": False, "reason": "The analysis role does not create or verify a rollback artifact."},
        "verification_plan": {"all_required_steps": True, "steps": ["revalidate target identity", "verify current-cycle logs", "verify process ownership", "verify Minecraft readiness when explicitly authorized"]},
        "prohibitions": ["no lifecycle execution", "no server file writes", "no backup or restore", "no raw shell execution", "no historical PID control", "no old Done fallback"],
        "unknowns": [item["id"] for item in evidence if item["collection_status"] != "complete"],
        "executor_contract": {"may_execute": False, "handoff_is_authorization": False, "must_revalidate_all_preconditions": True, "must_report_actual_actions": True},
        "execution_performed": False,
        "writes_performed": False,
    }
    canonical = json.dumps(payload, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode("utf-8")
    payload["integrity"] = {"canonicalization": "json-sort-keys-v1", "payload_sha256": hashlib.sha256(canonical).hexdigest(), "signature": None}
    emit(payload, args.pretty)
    return 0


if __name__ == "__main__":
    main_guard(run)
