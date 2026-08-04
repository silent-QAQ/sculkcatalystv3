from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, main_guard, parse_cli_root, rel


REGISTRY = {
    "vault": {"domain": "economy", "capabilities": ["economy.service-bridge"]},
    "vaultunlock": {"domain": "economy", "capabilities": ["economy.multi-currency"]},
    "playerpoints": {"domain": "economy", "capabilities": ["economy.points"]},
    "luckperms": {"domain": "permissions", "capabilities": ["permission.resolve", "permission.context", "permission.temporary"]},
    "chemdah": {"domain": "tasks", "capabilities": ["quest.definition", "quest.progress", "conversation.graph"]},
    "mythicmobs": {"domain": "mobs", "capabilities": ["mob.definition", "skill.timeline", "drop.expectation"]},
    "adyeshach": {"domain": "npc", "capabilities": ["npc.identity", "npc.dialogue"]},
    "citizens": {"domain": "npc", "capabilities": ["npc.identity"]},
    "placeholderapi": {"domain": "bridge", "capabilities": ["placeholder.resolve"]},
    "dragoncore": {"domain": "presentation", "capabilities": ["model.render", "animation.play", "ui.open", "input.receive"], "client_required": True},
    "germplugin": {"domain": "presentation", "capabilities": ["model.render", "animation.play", "ui.open", "input.receive"], "client_required": True},
    "germengine": {"domain": "presentation", "capabilities": ["model.render", "animation.play", "ui.open", "input.receive"], "client_required": True},
    "modelengine": {"domain": "presentation", "capabilities": ["model.render", "animation.play"]},
    "bettermodel": {"domain": "presentation", "capabilities": ["model.render", "animation.play"]},
    "paiui": {"domain": "presentation", "capabilities": ["ui.open", "input.receive"]},
    "arcartx": {"domain": "presentation", "capabilities": ["model.render", "animation.play", "ui.open", "input.receive"]},
}


def normalized(value: Any) -> str:
    return re.sub(r"[^a-z0-9]", "", str(value or "").casefold())


def run_child(root: Path, timeout: float) -> dict[str, Any]:
    result = subprocess.run(
        [sys.executable, str(Path(__file__).with_name("inspect-jars.py")), str(root)],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout, check=False,
    )
    if result.returncode not in {0, 1}:
        raise ValueError("inspect-jars.py failed to collect plugin metadata")
    payload = json.loads(result.stdout)
    if not isinstance(payload, dict):
        raise ValueError("inspect-jars.py output is not an object")
    return payload


def config_evidence(root: Path, identity: str) -> list[str]:
    target = normalized(identity)
    matches = []
    plugins = root / "plugins"
    if not plugins.is_dir():
        return matches
    for path in plugins.iterdir():
        if normalized(path.name) == target and path.is_dir():
            matches.append(rel(root, path))
    return sorted(matches)


def runtime_evidence(root: Path, names: list[str]) -> list[dict[str, Any]]:
    latest = root / "logs" / "latest.log"
    if not latest.is_file():
        return []
    text = latest.read_text(encoding="utf-8", errors="replace")[-2_000_000:]
    evidence = []
    for name in names:
        pattern = re.compile(rf"(?im)^.*(?:enabl|load|hook|depend).{{0,80}}{re.escape(name)}.*$")
        lines = pattern.findall(text)
        if lines:
            evidence.append({"plugin": name, "matches": [line[-500:] for line in lines[-5:]]})
    return evidence


def run() -> int:
    parser = JsonArgumentParser(description="Inspect the installed plugin ecosystem without loading or modifying plugins")
    parser.add_argument("server_root")
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    if args.timeout <= 0:
        raise ValueError("timeout must be positive")
    root = parse_cli_root(args.server_root)
    jars = run_child(root, args.timeout)
    plugins = []
    capability_index: dict[str, list[str]] = {}
    for component in jars.get("components", []):
        if component.get("ecosystem") not in {"bukkit", "bungeecord", "velocity"}:
            continue
        identity = str(component.get("id") or component.get("display_name") or "")
        registered = REGISTRY.get(normalized(identity))
        row = {
            "id": identity,
            "version": component.get("version"),
            "artifact_path": component.get("artifact_path"),
            "sha256": next((item.get("sha256") for item in jars.get("artifacts", []) if item.get("path") == component.get("artifact_path")), None),
            "dependencies": component.get("dependencies", []),
            "config_paths": config_evidence(root, identity),
            "registry_match": registered is not None,
            "domain": registered.get("domain") if registered else "unknown",
            "client_required": registered.get("client_required", False) if registered else None,
            "capabilities": [],
        }
        for capability in registered.get("capabilities", []) if registered else []:
            state = "unknown"
            basis = "plugin identity detected; runtime and version-specific capability still require validation"
            row["capabilities"].append({"id": capability, "state": state, "basis": basis})
            capability_index.setdefault(capability, []).append(identity)
        plugins.append(row)
    runtime = runtime_evidence(root, [item["id"] for item in plugins if item["id"]])
    emit({
        "schema": "minecraft-plugin-ecosystem/v1", "ok": True, "server_root": str(root),
        "plugins": plugins, "capability_candidates": capability_index, "runtime_evidence": runtime,
        "dependency_findings": jars.get("findings", []),
        "limitations": ["plugin identity does not prove a registered runtime provider", "capabilities remain unknown until version, configuration, runtime, data access, and client requirements are verified"],
        "analysis_only": True, "execution_performed": False, "writes_performed": False,
    }, args.pretty)
    return 1 if jars.get("findings") else 0


if __name__ == "__main__":
    main_guard(run)
