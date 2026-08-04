from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, is_link_or_reparse, main_guard, parse_cli_root, rel

try:
    import tomllib
except ImportError:  # pragma: no cover - Python 3.11+ is the supported runtime
    tomllib = None

try:
    import yaml
except ImportError:  # Optional: JSON and TOML analysis still works without PyYAML.
    yaml = None


FORMATS = {".yml": "yaml", ".yaml": "yaml", ".json": "json", ".toml": "toml"}
ID_KEYS = {"id", "quest-id", "quest_id", "conversation-id", "conversation_id"}
ENTRY_KEYS = {"entries", "entry", "objectives", "objective", "goals", "goal", "stages", "stage"}
REFERENCE_KEYS = {
    "quest", "quest-id", "quest_id", "required-quest", "required_quest", "prerequisite",
    "next", "goto", "jump", "conversation", "conversation-id", "conversation_id", "dialogue",
    "npc", "npc-id", "npc_id", "level", "level-id", "level_id",
}
SCRIPT_KEYS = {"script", "scripts", "kether", "then", "then-async", "then_async", "action", "actions"}
COMMAND_KEYS = {"command", "commands", "console", "player-command", "player_command"}
REWARD_KEYS = {"reward", "rewards", "prize", "prizes"}
HIGH_RISK = [
    ("arbitrary-command", re.compile(r"(?i)(?:^|\s)(?:command|console|sudo|execute|dispatch)\b")),
    ("permission-change", re.compile(r"(?i)(?:luckperms|\blp\b|permission|pex|group(?:add|set))")),
    ("economy-change", re.compile(r"(?i)(?:\beco\b|money|balance|deposit|withdraw|pay|currency|playerpoints)")),
    ("data-or-file-operation", re.compile(r"(?i)(?:delete|remove|database|sql|file|write|save|load)")),
    ("dynamic-input", re.compile(r"(?:\{[^{}]+\}|%[^%]+%|\$\{[^{}]+\})")),
]


def conceal_identifiers(value: Any, key: str = "") -> Any:
    if key in {"id", "source", "target", "value"} and isinstance(value, str):
        return {"length": len(value), "sha256": hashlib.sha256(value.encode("utf-8")).hexdigest()}
    if isinstance(value, dict):
        return {item_key: conceal_identifiers(item, str(item_key)) for item_key, item in value.items()}
    if isinstance(value, list):
        return [conceal_identifiers(item, key) for item in value]
    return value


def scalar_strings(value: Any) -> list[str]:
    if isinstance(value, (str, int, float)) and not isinstance(value, bool):
        return [str(value)]
    if isinstance(value, list):
        output: list[str] = []
        for item in value:
            output.extend(scalar_strings(item))
        return output
    return []


def normalize_key(value: Any) -> str:
    return str(value).strip().lower().replace(" ", "-")


def yaml_documents(text: str) -> tuple[list[Any], list[dict[str, Any]]]:
    if yaml is None:
        raise RuntimeError("PyYAML is unavailable")
    duplicates: list[dict[str, Any]] = []

    class DuplicateLoader(yaml.SafeLoader):
        def __init__(self, stream: Any) -> None:
            super().__init__(stream)
            self._budget_nodes = 0
            self._budget_aliases = 0
            self._budget_depth = 0

        def compose_node(self, parent: Any, index: Any) -> Any:
            self._budget_nodes += 1
            if self._budget_nodes > 200_000:
                raise yaml.YAMLError("YAML node budget exceeded")
            if self.check_event(yaml.events.AliasEvent):
                self._budget_aliases += 1
                if self._budget_aliases > 10_000:
                    raise yaml.YAMLError("YAML alias budget exceeded")
            self._budget_depth += 1
            if self._budget_depth > 128:
                raise yaml.YAMLError("YAML depth budget exceeded")
            try:
                return super().compose_node(parent, index)
            finally:
                self._budget_depth -= 1

    def construct_mapping(loader: Any, node: Any, deep: bool = False) -> dict[Any, Any]:
        mapping: dict[Any, Any] = {}
        seen: set[str] = set()
        for key_node, value_node in node.value:
            key = loader.construct_object(key_node, deep=deep)
            marker = repr(key)
            if marker in seen:
                duplicates.append({"key": str(key), "line": key_node.start_mark.line + 1})
            seen.add(marker)
            mapping[key] = loader.construct_object(value_node, deep=deep)
        return mapping

    DuplicateLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, construct_mapping)
    try:
        return list(yaml.load_all(text, Loader=DuplicateLoader)), duplicates
    except (yaml.YAMLError, RecursionError, TypeError) as exc:
        raise ValueError(f"unsafe or invalid YAML: {exc}") from exc


def json_document(text: str) -> tuple[list[Any], list[dict[str, Any]]]:
    duplicates: list[dict[str, Any]] = []

    def pairs_hook(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        seen: set[str] = set()
        for key, value in pairs:
            if key in seen:
                duplicates.append({"key": key, "line": None})
            seen.add(key)
            result[key] = value
        return result

    return [json.loads(text, object_pairs_hook=pairs_hook)], duplicates


def load_document(path: Path, max_bytes: int) -> tuple[list[Any], list[dict[str, Any]], str, str]:
    if is_link_or_reparse(path):
        raise ValueError("linked or reparse-point configuration is not accepted")
    with path.open("rb") as handle:
        before = os.fstat(handle.fileno())
        if before.st_size > max_bytes:
            raise ValueError(f"file exceeds byte limit ({before.st_size} > {max_bytes})")
        raw = handle.read(max_bytes + 1)
        after = os.fstat(handle.fileno())
    if len(raw) > max_bytes:
        raise ValueError("file grew beyond byte budget while reading")
    before_id = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    after_id = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
    if before_id != after_id or len(raw) != before.st_size:
        raise ValueError("configuration identity or size changed while reading")
    text = raw.decode("utf-8-sig")
    fmt = FORMATS[path.suffix.lower()]
    if fmt == "yaml":
        documents, duplicates = yaml_documents(text)
    elif fmt == "json":
        documents, duplicates = json_document(text)
    else:
        if tomllib is None:
            raise RuntimeError("tomllib is unavailable")
        documents, duplicates = [tomllib.loads(text)], []
    return documents, duplicates, fmt, hashlib.sha256(raw).hexdigest()


def locate_chemdah(root: Path) -> tuple[Path | None, list[str]]:
    plugins = root / "plugins"
    if not plugins.is_dir():
        return None, []
    matches = [path for path in plugins.iterdir() if path.is_dir() and path.name.lower() == "chemdah"]
    if not matches:
        return None, []
    return matches[0], [rel(root, path) for path in matches]


def collect_files(base: Path, max_files: int, max_entries: int) -> tuple[list[tuple[str, Path]], bool]:
    rows: list[tuple[str, Path]] = []
    entries = 0
    for kind in ("quest", "conversation"):
        folder = base / "core" / kind
        if not folder.is_dir() or is_link_or_reparse(folder):
            continue
        stack = [folder]
        while stack:
            current = stack.pop()
            for path in current.iterdir():
                entries += 1
                if entries > max_entries:
                    return sorted(rows, key=lambda item: str(item[1])), True
                if is_link_or_reparse(path):
                    continue
                if path.is_dir():
                    stack.append(path)
                elif path.is_file() and path.suffix.lower() in FORMATS:
                    rows.append((kind, path))
                    if len(rows) >= max_files:
                        return sorted(rows, key=lambda item: str(item[1])), True
    return sorted(rows, key=lambda item: str(item[1])), False


def declared_ids(data: Any, fallback: str) -> list[str]:
    found: list[str] = []
    if isinstance(data, dict):
        for key, value in data.items():
            if normalize_key(key) in ID_KEYS:
                found.extend(scalar_strings(value))
    return [value for value in found if value.strip()] or [fallback]


def walk(value: Any, source_id: str, file_path: str, pointer: str, state: dict[str, Any], traversal: dict[str, Any], depth: int = 0) -> None:
    if depth > 128:
        state["traversal_findings"].append({"severity": "error", "code": "content-depth-limit", "file": file_path, "path": pointer})
        return
    traversal["nodes"] += 1
    if traversal["nodes"] > 200_000:
        if not traversal["limit_reported"]:
            state["traversal_findings"].append({"severity": "error", "code": "content-node-limit", "file": file_path, "path": pointer})
            traversal["limit_reported"] = True
        return
    if isinstance(value, (dict, list)):
        marker = id(value)
        if marker in traversal["active"]:
            state["traversal_findings"].append({"severity": "error", "code": "recursive-alias", "file": file_path, "path": pointer})
            return
        traversal["active"].add(marker)
    if isinstance(value, dict):
        for raw_key, child in value.items():
            key = normalize_key(raw_key)
            child_pointer = f"{pointer}/{raw_key}"
            if key in REFERENCE_KEYS:
                for target in scalar_strings(child):
                    state["references"].append({"source": source_id, "target": target, "kind": key, "file": file_path, "path": child_pointer})
            if key in ENTRY_KEYS:
                entry_ids: list[str] = []
                items = child.values() if isinstance(child, dict) else child if isinstance(child, list) else []
                for index, item in enumerate(items):
                    if isinstance(item, dict):
                        ids = declared_ids(item, str(index))
                        entry_ids.extend(ids)
                        objective = next((v for k, v in item.items() if normalize_key(k) in {"objective", "type"}), None)
                        if objective is not None:
                            state["registry_validation"].extend({"type": "objective", "value": item_value, "file": file_path, "path": child_pointer} for item_value in scalar_strings(objective))
                for duplicate, count in Counter(entry_ids).items():
                    if count > 1:
                        state["duplicate_entries"].append({"id": duplicate, "occurrences": count, "file": file_path, "path": child_pointer})
            if key in SCRIPT_KEYS | COMMAND_KEYS | REWARD_KEYS:
                surface = "script" if key in SCRIPT_KEYS else "command" if key in COMMAND_KEYS else "reward"
                texts = scalar_strings(child)
                risks: set[str] = set()
                for text in texts:
                    risks.update(code for code, pattern in HIGH_RISK if pattern.search(text))
                severity = "high" if surface == "command" or risks.intersection({"arbitrary-command", "permission-change", "data-or-file-operation"}) else "medium"
                state["surfaces"].append({
                    "surface": surface,
                    "key": key,
                    "file": file_path,
                    "path": child_pointer,
                    "severity": severity,
                    "risk_codes": sorted(risks or ({"unverified-kether-or-extension"} if surface == "script" else {"reward-impact-review"})),
                    "content_fingerprints": [{"length": len(text), "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest()} for text in texts[:3]],
                    "executed": False,
                })
            walk(child, source_id, file_path, child_pointer, state, traversal, depth + 1)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk(child, source_id, file_path, f"{pointer}/{index}", state, traversal, depth + 1)
    if isinstance(value, (dict, list)):
        traversal["active"].remove(id(value))


def analyze_one_file(root: Path, plugin_root: Path, kind: str, path: Path, max_bytes: int) -> dict[str, Any]:
    relative = rel(root, path)
    fallback = path.relative_to(plugin_root / "core" / kind).with_suffix("").as_posix()
    size = path.stat().st_size
    row: dict[str, Any] = {"file": relative, "kind": kind, "size": size, "sha256": None}
    state: dict[str, Any] = {"references": [], "duplicate_entries": [], "surfaces": [], "registry_validation": [], "traversal_findings": []}
    declarations: list[dict[str, Any]] = []
    findings: list[dict[str, Any]] = []
    try:
        documents, duplicate_keys, fmt, digest = load_document(path, max_bytes)
        row.update({"format": fmt, "parse_status": "parsed", "documents": len(documents), "duplicate_keys": duplicate_keys, "sha256": digest})
        findings.extend({"severity": "error", "code": "duplicate-mapping-key", "file": relative, **item} for item in duplicate_keys)
        for index, document in enumerate(documents):
            for item_id in declared_ids(document, fallback if index == 0 else f"{fallback}#{index + 1}"):
                declarations.append({"id": item_id, "kind": kind, "file": relative, "document": index + 1})
                traversal = {"nodes": 0, "active": set(), "limit_reported": False}
                walk(document, item_id, relative, "", state, traversal)
    except (UnicodeDecodeError, ValueError, RuntimeError, RecursionError, TypeError) as exc:
        row.update({"format": FORMATS[path.suffix.lower()], "parse_status": "unavailable" if isinstance(exc, RuntimeError) else "error", "error": str(exc)})
        findings.append({"severity": "error", "code": "configuration-not-parsed", "file": relative, "detail": str(exc)})
    return {"row": row, "declarations": declarations, "state": state, "findings": findings}


def run() -> int:
    parser = JsonArgumentParser(description="Analyze Chemdah quests and conversations without executing content")
    parser.add_argument("server_root")
    parser.add_argument("--max-files", type=int, default=1000)
    parser.add_argument("--max-entries", type=int, default=20000)
    parser.add_argument("--max-bytes-per-file", type=int, default=4 * 1024 * 1024)
    parser.add_argument("--max-total-bytes", type=int, default=64 * 1024 * 1024)
    parser.add_argument("--max-seconds", type=float, default=30.0)
    parser.add_argument("--include-identifiers", action="store_true", help="Explicitly include task, reference, NPC, and objective identifiers")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.max_files <= 10000:
        raise ValueError("--max-files must be between 1 and 10000")
    if not args.max_files <= args.max_entries <= 200000:
        raise ValueError("--max-entries must be between --max-files and 200000")
    if not 1024 <= args.max_bytes_per_file <= 64 * 1024 * 1024:
        raise ValueError("--max-bytes-per-file must be between 1024 and 67108864")
    if not args.max_bytes_per_file <= args.max_total_bytes <= 2 * 1024 * 1024 * 1024:
        raise ValueError("--max-total-bytes must be between --max-bytes-per-file and 2147483648")
    if not 0.1 <= args.max_seconds <= 300:
        raise ValueError("--max-seconds must be between 0.1 and 300")

    root = parse_cli_root(args.server_root)
    plugin_root, matches = locate_chemdah(root)
    if plugin_root is None:
        emit({
            "ok": True, "analysis_only": True, "writes_performed": False, "execution_performed": False,
            "chemdah_detected": False, "files_scanned": [], "findings": [],
            "unknowns": ["Chemdah plugin data directory was not found under plugins/."],
        }, args.pretty)
        return 0

    files, truncated = collect_files(plugin_root, args.max_files, args.max_entries)
    state: dict[str, Any] = {"references": [], "duplicate_entries": [], "surfaces": [], "registry_validation": [], "traversal_findings": []}
    file_rows: list[dict[str, Any]] = []
    declarations: list[dict[str, Any]] = []
    findings: list[dict[str, Any]] = []
    total_bytes = 0
    started = time.monotonic()
    for kind, path in files:
        if time.monotonic() - started > args.max_seconds:
            findings.append({"severity": "error", "code": "analysis-time-budget", "detail": "Chemdah analysis time budget was reached."})
            truncated = True
            break
        size = path.stat().st_size
        relative = rel(root, path)
        if size > args.max_bytes_per_file:
            file_rows.append({"file": relative, "kind": kind, "size": size, "sha256": None, "format": FORMATS[path.suffix.lower()], "parse_status": "error", "error": "file exceeds byte budget"})
            findings.append({"severity": "error", "code": "configuration-not-parsed", "file": relative, "detail": "file exceeds byte budget"})
            continue
        if total_bytes + size > args.max_total_bytes:
            findings.append({"severity": "error", "code": "total-byte-limit", "file": relative, "detail": "Chemdah total byte budget was reached."})
            truncated = True
            break
        remaining = args.max_seconds - (time.monotonic() - started)
        try:
            result = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("_chemdah_worker.py")), str(root), str(plugin_root), kind, str(path), str(args.max_bytes_per_file)],
                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=max(remaining, 0.01), check=False,
            )
        except subprocess.TimeoutExpired:
            findings.append({"severity": "error", "code": "analysis-time-budget", "file": relative, "detail": "Chemdah parser worker exceeded the remaining deadline."})
            truncated = True
            break
        if result.returncode != 0:
            findings.append({"severity": "error", "code": "parser-worker-failed", "file": relative})
            truncated = True
            break
        parsed = json.loads(result.stdout)
        total_bytes += size
        file_rows.append(parsed["row"])
        declarations.extend(parsed["declarations"])
        findings.extend(parsed["findings"])
        for key in state:
            state[key].extend(parsed["state"][key])

    grouped: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for declaration in declarations:
        grouped.setdefault((declaration["kind"], declaration["id"]), []).append(declaration)
    duplicate_ids = [
        {"kind": kind, "id": item_id, "occurrences": len(rows), "locations": rows}
        for (kind, item_id), rows in sorted(grouped.items()) if len(rows) > 1
    ]
    declared = {item["id"] for item in declarations}
    unknown_references = [edge for edge in state["references"] if edge["target"] not in declared]
    findings.extend({"severity": "error", "code": "duplicate-content-id", **item} for item in duplicate_ids)
    findings.extend({"severity": "error", "code": "duplicate-entry-id", **item} for item in state["duplicate_entries"])
    findings.extend({"severity": "warning", "code": "unresolved-reference", **item} for item in unknown_references)
    findings.extend({"severity": item["severity"], "code": "active-content-surface", "surface": item} for item in state["surfaces"])
    findings.extend(state["traversal_findings"])
    if truncated:
        findings.append({"severity": "error", "code": "file-limit-reached", "detail": "Evidence is incomplete because --max-files was reached."})

    output_declarations = declarations if args.include_identifiers else conceal_identifiers(declarations)
    output_references = state["references"] if args.include_identifiers else conceal_identifiers(state["references"])
    output_unknown = unknown_references if args.include_identifiers else conceal_identifiers(unknown_references)
    output_findings = findings if args.include_identifiers else conceal_identifiers(findings)
    payload = {
        "ok": not any(item["severity"] == "error" for item in findings),
        "schema": "minecraft-chemdah-analysis/v1",
        "analysis_only": True,
        "writes_performed": False,
        "execution_performed": False,
        "kether_executed": False,
        "commands_executed": False,
        "chemdah_detected": True,
        "chemdah_roots": matches,
        "evidence_complete": not truncated and all(item["parse_status"] == "parsed" for item in file_rows),
        "files_scanned": file_rows,
        "content_nodes": output_declarations,
        "reference_graph": {"nodes": output_declarations, "edges": output_references, "unresolved": output_unknown},
        "duplicate_ids": duplicate_ids if args.include_identifiers else conceal_identifiers(duplicate_ids),
        "duplicate_entries": state["duplicate_entries"] if args.include_identifiers else conceal_identifiers(state["duplicate_entries"]),
        "active_content_surfaces": state["surfaces"],
        "registry_validation_required": state["registry_validation"] if args.include_identifiers else conceal_identifiers(state["registry_validation"]),
        "findings": output_findings,
        "review_policy": {
            "configuration_is_not_executed": True,
            "recommendations_are_not_authorization": True,
            "external_executor_must_revalidate_version_hashes_and_approval_scope": True,
        },
    }
    emit(payload, args.pretty)
    return 1 if findings else 0


if __name__ == "__main__":
    main_guard(run)
