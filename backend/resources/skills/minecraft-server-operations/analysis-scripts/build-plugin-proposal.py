from __future__ import annotations

import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, main_guard, parse_cli_root, sha256_file, utc_now


DOMAINS = ("ecosystem", "economy", "permissions", "tasks", "mobs", "npc", "presentation")
RISK = ("L0", "L1", "L2", "L3", "L4")
SUPPORTED_POLICY_VERSION = "minecraft-approval-policy/v1"
RISK_RANK = {value: index for index, value in enumerate(RISK)}
ACTION_MINIMUM_RISK = {
    "reward-adjustment": "L3", "permission-group": "L3", "boss-timeline": "L3", "core-quest": "L3",
    "asset-confiscation": "L4", "permission-wildcard": "L4", "paid-entitlement": "L4",
    "rollback": "L4", "boss-ai-control": "L4", "bulk-balance": "L4",
}


def load_json(path: Path, limit: int = 16 * 1024 * 1024) -> dict[str, Any]:
    if not path.is_file() or path.stat().st_size > limit:
        raise ValueError(f"invalid or oversized JSON input: {path}")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"JSON input must be an object: {path}")
    return value


def future_timestamp(value: Any) -> bool:
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
        return parsed.tzinfo is not None and parsed > datetime.now(timezone.utc)
    except ValueError:
        return False


def approval_status(root: Path, domain: str, risk: str, plugins: list[str], plan_hash: str, analysis: dict[str, Any], scope: dict[str, Any] | None) -> tuple[str, str | None, list[str]]:
    failures = []
    if scope is None:
        return "manual_review_required", None, ["no trust scope supplied"]
    if not scope.get("scope_id"):
        failures.append("trust scope has no stable scope_id")
    if scope.get("policy_version") != SUPPORTED_POLICY_VERSION:
        failures.append("trust scope policy_version is unsupported")
    if not scope.get("server_trust_enabled"):
        failures.append("server-level full trust is not enabled")
    if risk == "L4" and not scope.get("l4_second_confirmation"):
        failures.append("L4 second full-trust confirmation is missing")
    declared_root = scope.get("server_root")
    if not declared_root or Path(str(declared_root)).resolve() != root.resolve():
        failures.append("trust scope belongs to another server root")
    if domain not in scope.get("domains", []):
        failures.append("domain is outside trust scope")
    allowed_plugins = {str(item).casefold() for item in scope.get("plugins", [])}
    if any(plugin.casefold() not in allowed_plugins for plugin in plugins):
        failures.append("one or more plugins are outside trust scope")
    expected_hash = scope.get("approved_plan_hash")
    if not expected_hash:
        failures.append("trust scope is not bound to an approved plan hash")
    elif expected_hash != plan_hash:
        failures.append("approved plan hash does not match")
    if scope.get("blocking_unknowns"):
        failures.append("trust scope contains blocking unknowns")
    if not future_timestamp(scope.get("valid_until")):
        failures.append("trust scope is expired or has no valid timezone-aware expiry")
    if not future_timestamp(analysis.get("evidence_valid_until")):
        failures.append("analysis evidence is stale or has no valid expiry")
    if analysis.get("blocking_unknowns"):
        failures.append("analysis contains blocking unknowns")
    ceiling = scope.get("risk_ceiling")
    if ceiling not in RISK:
        failures.append("trust scope risk ceiling is missing or invalid")
    elif RISK_RANK[risk] > RISK_RANK[ceiling]:
        failures.append("declared risk exceeds trust scope ceiling")
    plugin_versions = analysis.get("plugin_versions")
    target_hashes = analysis.get("target_hashes")
    if not isinstance(plugin_versions, dict) or not plugin_versions or plugin_versions != scope.get("plugin_versions"):
        failures.append("plugin versions are missing or do not match trust scope")
    if not isinstance(target_hashes, dict) or not target_hashes or target_hashes != scope.get("target_hashes"):
        failures.append("target hashes are missing or do not match trust scope")
    controls = analysis.get("controls") if isinstance(analysis.get("controls"), dict) else {}
    for field in ("rollback_requirements", "verification_window", "stop_conditions", "audit_id"):
        if not controls.get(field):
            failures.append(f"analysis controls are missing required field: {field}")
    impact = analysis.get("impact") if isinstance(analysis.get("impact"), dict) else {}
    for field in ("affected_players", "worlds", "actions", "risk_budget"):
        if field not in impact:
            failures.append(f"analysis impact is missing required field: {field}")
    try:
        affected = int(impact.get("affected_players"))
        maximum = int(scope.get("max_affected_players"))
        if affected < 0 or maximum < 0 or affected > maximum:
            failures.append("affected player count exceeds trust scope")
    except (TypeError, ValueError):
        failures.append("affected player limit is missing or invalid")
    worlds = impact.get("worlds")
    actions_value = impact.get("actions")
    budgets_value = impact.get("risk_budget")
    if not isinstance(worlds, list) or not worlds or not all(isinstance(item, str) and item for item in worlds):
        failures.append("impact worlds must be a non-empty string array")
        worlds = []
    if not isinstance(actions_value, list) or not actions_value or not all(isinstance(item, str) and item for item in actions_value):
        failures.append("impact actions must be a non-empty string array")
        actions_value = []
    if not isinstance(budgets_value, dict) or not budgets_value:
        failures.append("impact risk_budget must be a non-empty object")
        budgets_value = {}
    allowed_worlds = {str(item) for item in scope.get("worlds", [])}
    if any(world not in allowed_worlds for world in worlds):
        failures.append("one or more worlds are outside trust scope")
    excluded = {str(item) for item in scope.get("excluded_actions", [])}
    actions = list(actions_value)
    if any(action in excluded for action in actions):
        failures.append("proposal contains an excluded action")
    unknown_actions = sorted(set(actions) - set(ACTION_MINIMUM_RISK))
    if unknown_actions:
        failures.append(f"proposal contains unclassified actions: {', '.join(unknown_actions)}")
    minimum = max((ACTION_MINIMUM_RISK.get(action, "L0") for action in actions), key=lambda value: RISK_RANK[value], default="L0")
    if RISK_RANK[risk] < RISK_RANK[minimum]:
        failures.append(f"declared risk is below the action-derived minimum: {minimum}")
    budgets = budgets_value
    limits = scope.get("risk_budget") if isinstance(scope.get("risk_budget"), dict) else {}
    for key, value in budgets.items():
        try:
            if key not in limits or float(value) > float(limits[key]):
                failures.append(f"risk budget exceeds scope: {key}")
        except (TypeError, ValueError):
            failures.append(f"risk budget is invalid: {key}")
    status = "manual_review_required" if failures else "eligible_for_external_auto_approval"
    return status, scope.get("scope_id"), failures


def run() -> int:
    parser = JsonArgumentParser(description="Build an analysis-only plugin governance proposal")
    parser.add_argument("server_root")
    parser.add_argument("--domain", choices=DOMAINS, required=True)
    parser.add_argument("--risk-level", choices=RISK, required=True)
    parser.add_argument("--analysis", required=True, help="Read-only JSON analysis prepared by an analyzer")
    parser.add_argument("--plugin", action="append", default=[])
    parser.add_argument("--trust-scope", help="Optional owner-approved JSON trust scope")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    analysis_path = Path(args.analysis).expanduser().resolve(strict=True)
    analysis = load_json(analysis_path)
    proposal_core = {"domain": args.domain, "risk_level": args.risk_level, "plugins": sorted(set(args.plugin)), "analysis": analysis}
    plan_hash = hashlib.sha256(json.dumps(proposal_core, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()
    scope_path = Path(args.trust_scope).expanduser().resolve(strict=True) if args.trust_scope else None
    scope = load_json(scope_path) if scope_path else None
    status, scope_id, approval_failures = approval_status(root, args.domain, args.risk_level, proposal_core["plugins"], plan_hash, analysis, scope)
    payload = {
        "schema": "minecraft-plugin-governance-handoff/v1", "created_at": utc_now(), "domain": args.domain,
        "evidence": [{"source": str(analysis_path), "sha256": sha256_file(analysis_path), "payload": analysis}],
        "findings": analysis.get("findings", []),
        "proposal": proposal_core,
        "approval": {"risk_level": args.risk_level, "status": status, "approval_authoritative": False, "scope_id": scope_id, "plan_hash": plan_hash, "failures": approval_failures},
        "executor_contract": {"may_execute": False, "handoff_is_authorization": False, "must_revalidate_evidence": True, "must_revalidate_scope": True, "external_approval_role_must_verify_scope_authority": True},
        "execution_performed": False, "writes_performed": False,
    }
    canonical = json.dumps(payload, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode("utf-8")
    payload["integrity"] = {"canonicalization": "json-sort-keys-v1", "payload_sha256": hashlib.sha256(canonical).hexdigest(), "signature": None}
    emit(payload, args.pretty)
    return 0


if __name__ == "__main__":
    main_guard(run)
