from __future__ import annotations

import json
import re
import tomllib
import zipfile
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, limited_files, main_guard, parse_cli_root, rel, sha256_file


DESCRIPTORS = (
    "META-INF/MANIFEST.MF",
    "plugin.yml",
    "paper-plugin.yml",
    "bungee.yml",
    "velocity-plugin.json",
    "fabric.mod.json",
    "META-INF/mods.toml",
    "META-INF/neoforge.mods.toml",
)
PLATFORM_CAPABILITIES = {
    "minecraft", "java", "fabricloader", "fabric-api", "forge", "neoforge", "javafml",
    "bukkit", "spigot", "paper", "purpur", "folia", "velocity", "bungeecord", "waterfall",
}


def normalized(value: Any) -> str:
    return str(value or "").strip().casefold()


def as_list(value: Any) -> list[Any]:
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def manifest(text: str) -> dict[str, str]:
    unfolded = re.sub(r"\r?\n ", "", text)
    return {key.strip(): value.strip() for line in unfolded.splitlines() if ":" in line for key, value in [line.split(":", 1)]}


def edge(target: Any, required: bool, relation: str = "depends", constraint: Any = None, scope: str = "runtime", **extra: Any) -> dict[str, Any]:
    return {"target": str(target), "required": required, "relation": relation, "constraint": constraint, "scope": scope, **extra}


def component(path: str, ecosystem: str, identity: Any, name: Any, version: Any, dependencies: list[dict], provides: list[Any] | None = None, metadata: dict | None = None) -> dict:
    identity = str(identity or name or "").strip()
    return {
        "component_key": f"{ecosystem}:{normalized(identity)}@{path}",
        "artifact_path": path,
        "ecosystem": ecosystem,
        "id": identity,
        "display_name": str(name or identity),
        "version": str(version) if version is not None else None,
        "provides": [str(item) for item in (provides or [])],
        "dependencies": dependencies,
        "metadata": metadata or {},
    }


def parse_yaml_descriptor(name: str, text: str, path: str) -> tuple[dict, list[dict], list[str]]:
    try:
        import yaml  # type: ignore
    except ImportError:
        return {}, [], [f"{name}: PyYAML unavailable; dependency graph omitted"]
    try:
        data = yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise ValueError(f"{name}: {exc}") from exc
    if not isinstance(data, dict):
        raise ValueError(f"{name}: descriptor root must be a mapping")
    dependencies: list[dict] = []
    ecosystem = "bukkit"
    if name == "bungee.yml":
        ecosystem = "bungeecord"
        for target in as_list(data.get("depends")):
            dependencies.append(edge(target, True, scope="proxy", source_field="depends"))
        for target in as_list(data.get("softDepends")):
            dependencies.append(edge(target, False, "suggests", scope="proxy", source_field="softDepends"))
    elif name == "paper-plugin.yml":
        declared = data.get("dependencies")
        if isinstance(declared, dict):
            for scope, group in declared.items():
                if not isinstance(group, dict):
                    continue
                for target, settings in group.items():
                    settings = settings if isinstance(settings, dict) else {}
                    required = settings.get("required")
                    load = str(settings.get("load", "")).upper()
                    relation = "load_before" if load == "BEFORE" else "load_after" if load == "AFTER" else "depends"
                    dependencies.append(edge(target, bool(required) if required is not None else False, relation, scope=str(scope), source_field=f"dependencies.{scope}"))
    else:
        for target in as_list(data.get("depend")):
            dependencies.append(edge(target, True, source_field="depend"))
        for target in as_list(data.get("softdepend")):
            dependencies.append(edge(target, False, "suggests", source_field="softdepend"))
        for target in as_list(data.get("loadbefore")):
            dependencies.append(edge(target, False, "load_before", source_field="loadbefore"))
    metadata = {key: data.get(key) for key in ("main", "api-version", "folia-supported") if key in data}
    item = component(path, ecosystem, data.get("name"), data.get("name"), data.get("version"), dependencies, metadata=metadata)
    return data, [item] if item["id"] else [], []


def parse_json_descriptor(name: str, text: str, path: str) -> tuple[dict, list[dict]]:
    data = json.loads(text)
    if not isinstance(data, dict):
        raise ValueError(f"{name}: descriptor root must be an object")
    dependencies = []
    if name == "fabric.mod.json":
        for field, required, relation in (("depends", True, "depends"), ("recommends", False, "recommends"), ("suggests", False, "suggests"), ("conflicts", False, "conflicts"), ("breaks", False, "breaks")):
            values = data.get(field, {})
            if isinstance(values, dict):
                for target, constraint in values.items():
                    dependencies.append(edge(target, required, relation, constraint, source_field=field))
        item = component(path, "fabric", data.get("id"), data.get("name"), data.get("version"), dependencies, as_list(data.get("provides")), {"environment": data.get("environment"), "entrypoints": data.get("entrypoints")})
    else:
        for declared in as_list(data.get("dependencies")):
            if isinstance(declared, dict) and declared.get("id"):
                dependencies.append(edge(declared["id"], not bool(declared.get("optional", False)), "depends" if not declared.get("optional") else "suggests", declared.get("version"), scope="proxy", source_field="dependencies"))
        item = component(path, "velocity", data.get("id"), data.get("name"), data.get("version"), dependencies, metadata={"main": data.get("main")})
    return data, [item] if item["id"] else []


def parse_toml_descriptor(name: str, text: str, path: str) -> tuple[dict, list[dict]]:
    data = tomllib.loads(text)
    ecosystem = "neoforge" if "neoforge" in name else "forge"
    components = []
    dependency_groups = data.get("dependencies", {})
    for mod in as_list(data.get("mods")):
        if not isinstance(mod, dict) or not mod.get("modId"):
            continue
        mod_id = str(mod["modId"])
        dependencies = []
        groups = dependency_groups.get(mod_id, []) if isinstance(dependency_groups, dict) else []
        for declared in as_list(groups):
            if not isinstance(declared, dict) or not declared.get("modId"):
                continue
            dep_type = str(declared.get("type", "")).lower()
            mandatory = bool(declared.get("mandatory", False))
            relation = "breaks" if dep_type == "incompatible" else "conflicts" if dep_type == "discouraged" else "suggests" if dep_type == "optional" else "depends"
            required = mandatory or dep_type == "required"
            dependencies.append(edge(declared["modId"], required, relation, declared.get("versionRange"), side=declared.get("side"), ordering=declared.get("ordering"), source_field=f"dependencies.{mod_id}"))
        components.append(component(path, ecosystem, mod_id, mod.get("displayName"), mod.get("version"), dependencies, metadata={"description": mod.get("description")}))
    return data, components


def inspect(path: Path, root: Path) -> tuple[dict, list[dict]]:
    artifact_path = rel(root, path)
    row = {"path": artifact_path, "size": path.stat().st_size, "sha256": sha256_file(path), "descriptors": [], "metadata": {}, "warnings": [], "errors": []}
    components: list[dict] = []
    try:
        with zipfile.ZipFile(path) as archive:
            all_names = archive.namelist()
            for name in DESCRIPTORS:
                count = all_names.count(name)
                if count > 1:
                    row["warnings"].append(f"ambiguous descriptor appears {count} times: {name}")
                if not count:
                    continue
                row["descriptors"].append(name)
                info = archive.getinfo(name)
                ratio = info.file_size / max(info.compress_size, 1)
                if info.file_size > 2 * 1024 * 1024 or ratio > 100:
                    row["warnings"].append(f"descriptor exceeds size or compression budget: {name}")
                    continue
                with archive.open(info) as stream:
                    raw = stream.read(2 * 1024 * 1024 + 1)
                if len(raw) > 2 * 1024 * 1024:
                    row["warnings"].append(f"descriptor exceeds read budget: {name}")
                    continue
                text = raw.decode("utf-8", errors="strict")
                try:
                    if name.endswith("MANIFEST.MF"):
                        values = manifest(text)
                        row["metadata"][name] = {key: values[key] for key in ("Implementation-Title", "Implementation-Version", "Main-Class", "Automatic-Module-Name") if key in values}
                    elif name.endswith(".json"):
                        data, parsed = parse_json_descriptor(name, text, artifact_path)
                        row["metadata"][name] = data
                        components.extend(parsed)
                    elif name.endswith(".toml"):
                        data, parsed = parse_toml_descriptor(name, text, artifact_path)
                        row["metadata"][name] = data
                        components.extend(parsed)
                    else:
                        data, parsed, warnings = parse_yaml_descriptor(name, text, artifact_path)
                        row["metadata"][name] = data
                        row["warnings"].extend(warnings)
                        components.extend(parsed)
                except (ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError, UnicodeDecodeError) as exc:
                    row["errors"].append(str(exc))
    except (OSError, zipfile.BadZipFile, RuntimeError) as exc:
        row["errors"].append(str(exc))
    # Prefer Paper's descriptor over plugin.yml when both describe the same Bukkit component.
    merged: dict[tuple[str, str], dict] = {}
    for item in components:
        key = (item["ecosystem"], normalized(item["id"]))
        if key in merged and item["artifact_path"] == merged[key]["artifact_path"]:
            previous = merged[key]
            previous["dependencies"] = list({json.dumps(dep, sort_keys=True): dep for dep in previous["dependencies"] + item["dependencies"]}.values())
            previous["metadata"].update(item["metadata"])
        else:
            merged[key] = item
    return row, list(merged.values())


def build_findings(artifacts: list[dict], components: list[dict]) -> list[dict]:
    findings = []
    by_hash: dict[str, list[str]] = {}
    for artifact in artifacts:
        by_hash.setdefault(artifact["sha256"], []).append(artifact["path"])
        if artifact["errors"]:
            findings.append({"code": "malformed-artifact", "severity": "error", "artifacts": [artifact["path"]], "evidence": artifact["errors"]})
    for paths in by_hash.values():
        if len(paths) > 1:
            findings.append({"code": "duplicate-content", "severity": "warning", "artifacts": paths})
    identities: dict[tuple[str, str], list[dict]] = {}
    providers: dict[tuple[str, str], list[dict]] = {}
    for item in components:
        key = (item["ecosystem"], normalized(item["id"]))
        identities.setdefault(key, []).append(item)
        for provided in [item["id"], *item["provides"]]:
            providers.setdefault((item["ecosystem"], normalized(provided)), []).append(item)
    for (ecosystem, identity), items in identities.items():
        paths = sorted({item["artifact_path"] for item in items})
        if len(paths) > 1:
            findings.append({"code": "duplicate-identity", "severity": "error", "ecosystem": ecosystem, "target": identity, "artifacts": paths})
    for item in components:
        own = {normalized(item["id"]), *(normalized(value) for value in item["provides"])}
        for dependency in item["dependencies"]:
            target = normalized(dependency["target"])
            if dependency["relation"] not in {"depends", "recommends", "suggests"}:
                continue
            if target in own:
                findings.append({"code": "self-dependency", "severity": "error", "component_key": item["component_key"], "target": dependency["target"]})
            elif dependency["required"] and target not in PLATFORM_CAPABILITIES and (item["ecosystem"], target) not in providers:
                findings.append({"code": "missing-required", "severity": "error", "component_key": item["component_key"], "target": dependency["target"], "constraint": dependency.get("constraint")})
    return findings


def run() -> int:
    parser = JsonArgumentParser(description="Inspect JAR metadata and build a normalized dependency graph")
    parser.add_argument("server_root")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    jars = limited_files(root, ("*.jar", "plugins/*.jar", "mods/*.jar"), 2000)
    artifacts = []
    components = []
    for path in jars:
        artifact, parsed = inspect(path, root)
        artifacts.append(artifact)
        components.extend(parsed)
    findings = build_findings(artifacts, components)
    errors = [item for item in findings if item["severity"] == "error"]
    emit({"ok": not errors, "artifacts": artifacts, "components": components, "findings": findings, "version_constraints_evaluated": False}, args.pretty)
    return 1 if errors else 0


if __name__ == "__main__":
    main_guard(run)
