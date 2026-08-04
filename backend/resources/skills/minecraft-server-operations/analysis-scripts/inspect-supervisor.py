from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from _common import JsonArgumentParser, emit, is_link_or_reparse, main_guard, parse_cli_root, rel


CONFIDENCE = {"declared": 1, "correlated": 2, "owned": 3}


def bounded_text(path: Path, limit: int = 256 * 1024) -> str:
    if path.stat().st_size > limit:
        raise ValueError(f"file exceeds inspection limit: {path}")
    return path.read_text(encoding="utf-8", errors="replace")


def command_probe(argv: list[str], timeout: float) -> dict[str, Any]:
    started = time.monotonic()
    try:
        result = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout, check=False)
        return {"argv": argv, "available": True, "returncode": result.returncode, "duration_ms": round((time.monotonic() - started) * 1000, 2), "stdout": result.stdout[:1024 * 1024], "stderr": result.stderr[:4096]}
    except FileNotFoundError:
        return {"argv": argv, "available": False, "reason": "command-not-found"}
    except subprocess.TimeoutExpired:
        return {"argv": argv, "available": True, "timed_out": True, "duration_ms": round((time.monotonic() - started) * 1000, 2)}


def script_candidates(root: Path) -> list[dict]:
    result = []
    patterns = ("*.bat", "*.cmd", "*.ps1", "*.sh", "scripts/*.bat", "scripts/*.cmd", "scripts/*.ps1", "scripts/*.sh")
    for pattern in patterns:
        for path in root.glob(pattern):
            if not path.is_file() or is_link_or_reparse(path):
                continue
            text = bounded_text(path)
            jars = re.findall(r"(?i)(?:-jar\s+|java\s+[^\r\n]*?)([^\s\"']+\.jar)", text)
            evidence = [{"type": "file", "source": rel(root, path), "value": "launch script"}]
            if jars:
                evidence.append({"type": "jar-reference", "source": rel(root, path), "value": jars[:20]})
            result.append({"kind": "script", "id": rel(root, path), "confidence": "declared", "state": "unknown", "control_available": True, "evidence": evidence, "execution_handoff": {"recommended_only": True, "start": [str(path)]}, "warnings": ["script content was inspected but not executed"]})
    return result


SYSTEMD_UNIT_RE = re.compile(r"^[A-Za-z0-9_.@-]+\.service$")


def parse_systemd_cgroup(text: str) -> set[str]:
    units = set()
    for line in text.splitlines():
        parts = line.split(":", 2)
        if len(parts) != 3:
            continue
        components = parts[2].split("/")
        if any(component in {".", ".."} or "\\" in component for component in components):
            continue
        for component in components:
            if SYSTEMD_UNIT_RE.fullmatch(component) and "/" not in component and "\\\\" not in component:
                units.add(component)
    return units


def assess_systemd_ownership(root: Path, unit: str, values: dict[str, str], processes: list[dict]) -> dict:
    try:
        main_pid = int(values.get("MainPID", "0") or 0)
    except (TypeError, ValueError):
        main_pid = 0
    unit_processes = [item for item in processes if unit in item.get("units", set())]
    main = [item for item in unit_processes if item.get("pid") == main_pid]
    active = values.get("LoadState") == "loaded" and values.get("ActiveState") == "active" and main_pid > 0
    cwd_exact = len(main) == 1 and Path(str(main[0].get("cwd", ""))).resolve() == root.resolve()
    java_command = len(main) == 1 and bool(main[0].get("minecraft_java"))
    working = values.get("WorkingDirectory", "")
    working_path = Path(working) if working else None
    working_exact = bool(working_path and working_path.is_absolute() and working_path.resolve() == root.resolve())
    identity_exact = values.get("Id") == unit
    unique_instance = len(unit_processes) == 1
    checks = {
        "unit_identity": identity_exact,
        "active_main_pid": active,
        "main_pid_in_cgroup": len(main) == 1,
        "cwd_exact": cwd_exact,
        "minecraft_java": java_command,
        "working_directory_exact": working_exact,
        "unique_instance": unique_instance,
    }
    proven = all(checks.values())
    return {
        "proven": proven,
        "conflict": len(unit_processes) > 1,
        "state": values.get("ActiveState", "unknown"),
        "runtime_ids": [main_pid] if proven else [],
        "checks": checks,
        "attestation_candidate": proven,
    }


def collect_root_systemd_processes(root: Path, proc_root: Path = Path("/proc")) -> list[dict]:
    result = []
    try:
        entries = list(proc_root.iterdir())
    except OSError:
        return result
    for path in entries:
        if not path.name.isdigit():
            continue
        try:
            start_before = (path / "stat").read_text(encoding="utf-8", errors="replace").rsplit(") ", 1)[1].split()[19]
            cwd = (path / "cwd").resolve(strict=True)
            if cwd != root.resolve():
                continue
            cmdline = (path / "cmdline").read_bytes()[:256 * 1024].replace(b"\x00", b" ").decode("utf-8", errors="replace").strip()
            cgroup = (path / "cgroup").read_text(encoding="utf-8", errors="replace")
            start_after = (path / "stat").read_text(encoding="utf-8", errors="replace").rsplit(") ", 1)[1].split()[19]
        except (OSError, IndexError):
            continue
        if start_before != start_after:
            continue
        executable = Path(cmdline.split(" ", 1)[0]).name.casefold() if cmdline else ""
        minecraft_java = executable in {"java", "javaw", "java.exe", "javaw.exe"} and (".jar" in cmdline.casefold() or "net.minecraft" in cmdline.casefold())
        units = parse_systemd_cgroup(cgroup)
        if minecraft_java and len(units) == 1:
            result.append({"pid": int(path.name), "starttime_ticks": start_before, "cwd": str(cwd), "units": units, "minecraft_java": True})
    return result


def systemd_candidates(root: Path, timeout: float, probes: list[dict]) -> list[dict]:
    result = []
    if os.name == "nt":
        return result
    local_units = {}
    for path in root.glob("*.service"):
        if not path.is_file() or is_link_or_reparse(path):
            continue
        text = bounded_text(path)
        working = re.search(r"(?m)^WorkingDirectory=(.+)$", text)
        exec_start = re.search(r"(?m)^ExecStart=(.+)$", text)
        local_units[path.name] = {"path": path, "working": working, "exec_start": exec_start}
    processes = collect_root_systemd_processes(root)
    runtime_units = {unit for item in processes for unit in item["units"]}
    for unit in sorted(set(local_units) | runtime_units):
        local = local_units.get(unit)
        working = local.get("working") if local else None
        exec_start = local.get("exec_start") if local else None
        declared_working = Path(working.group(1).strip()) if working else None
        correlated = bool(declared_working and declared_working.is_absolute() and declared_working.resolve() == root.resolve())
        state = "unknown"
        confidence = "correlated" if correlated else "declared"
        ownership = {"status": "absent", "method": "proc-cgroup-systemd-reverse", "runtime_ids": [], "checks": {}, "attestation_candidate": False}
        runtime_conflict = False
        if shutil.which("systemctl"):
            probe = command_probe(["systemctl", "show", unit, "--no-pager", "--property=Id,LoadState,ActiveState,SubState,MainPID,WorkingDirectory,ExecStart,FragmentPath,ControlGroup,InvocationID"], timeout)
            probes.append(probe)
            if probe.get("returncode") == 0:
                values = dict(line.split("=", 1) for line in probe.get("stdout", "").splitlines() if "=" in line)
                assessment = assess_systemd_ownership(root, unit, values, processes)
                state = assessment["state"]
                ownership = {"status": "runtime-proven" if assessment["proven"] else "partial" if unit in runtime_units else "absent", "method": "proc-cgroup-systemd-reverse", "runtime_ids": assessment["runtime_ids"], "checks": assessment["checks"], "attestation_candidate": assessment["attestation_candidate"], "invocation_id": values.get("InvocationID"), "control_group": values.get("ControlGroup"), "fragment_path": values.get("FragmentPath")}
                runtime_conflict = assessment["conflict"]
                confidence = "owned" if assessment["proven"] else "correlated" if correlated or unit in runtime_units else "declared"
        source = rel(root, local["path"]) if local else "runtime:/proc"
        result.append({"kind": "systemd", "id": unit, "confidence": confidence, "state": state, "control_available": shutil.which("systemctl") is not None, "ownership": ownership, "runtime_conflict": runtime_conflict, "evidence": [{"type": "unit-file" if local else "runtime-cgroup", "source": source, "value": {"WorkingDirectory": working.group(1).strip() if working else None, "ExecStart": exec_start.group(1).strip() if exec_start else None}}], "execution_handoff": {"recommended_only": True, "status": ["systemctl", "status", "--", unit], "start": ["systemctl", "start", "--", unit], "stop": ["systemctl", "stop", "--", unit]}, "warnings": [] if confidence == "owned" else ["systemd ownership requires an exact MainPID, cgroup, cwd, Java command, and WorkingDirectory match"]})
    return result


def parse_json_records(text: str) -> list[dict]:
    text = text.strip()
    if not text:
        return []
    try:
        value = json.loads(text)
        return value if isinstance(value, list) else [value] if isinstance(value, dict) else []
    except json.JSONDecodeError:
        result = []
        for line in text.splitlines():
            try:
                value = json.loads(line)
                if isinstance(value, dict):
                    result.append(value)
            except json.JSONDecodeError:
                continue
        return result


def assess_compose_ownership(root: Path, compose_file: Path, service: str, ps_rows: list[dict], inspect_rows: list[dict]) -> dict:
    ids = {str(row.get("ID") or row.get("Id") or row.get("ContainerID") or "") for row in ps_rows if str(row.get("Service") or row.get("service") or "") == service}
    containers = [row for row in inspect_rows if str(row.get("Id") or row.get("ID") or "") in ids]
    running = [row for row in containers if bool((row.get("State") or {}).get("Running"))]
    checks = []
    owned_ids = []
    for container in running:
        labels = (container.get("Config") or {}).get("Labels") or {}
        mounts = container.get("Mounts") or []
        label_service = labels.get("com.docker.compose.service") == service
        working = labels.get("com.docker.compose.project.working_dir")
        working_ok = bool(working) and Path(working).resolve() == root.resolve()
        config_files = [item.strip() for item in str(labels.get("com.docker.compose.project.config_files") or "").split(",") if item.strip()]
        config_ok = any(Path(item).resolve() == compose_file.resolve() for item in config_files)
        mount_ok = any(item.get("Type") == "bind" and Path(str(item.get("Source", ""))).resolve() == root.resolve() for item in mounts)
        passed = label_service and working_ok and config_ok and mount_ok
        checks.append({"container_id": container.get("Id"), "service_label": label_service, "working_dir": working_ok, "config_file": config_ok, "root_bind_mount": mount_ok, "passed": passed})
        if passed:
            owned_ids.append(str(container.get("Id")))
    proven = len(owned_ids) == 1 and len(running) == 1
    conflict = len(running) > 1
    return {"proven": proven, "conflict": conflict, "state": "running" if running else "stopped", "runtime_ids": sorted(owned_ids), "checks": checks}


def compose_candidates(root: Path, timeout: float, probes: list[dict]) -> list[dict]:
    result = []
    for name in ("compose.yml", "compose.yaml", "docker-compose.yml", "docker-compose.yaml"):
        path = root / name
        if not path.is_file() or is_link_or_reparse(path):
            continue
        text = bounded_text(path)
        services = []
        try:
            import yaml  # type: ignore
            data = yaml.safe_load(text)
            if isinstance(data, dict) and isinstance(data.get("services"), dict):
                services = list(data["services"].keys())
        except ImportError:
            services = []
        except yaml.YAMLError:
            services = []
        service = services[0] if len(services) == 1 else None
        compose_base = ["docker", "compose", "--project-directory", str(root), "-f", str(path)]
        commands = {"status": [*compose_base, "ps", service]} if service else {}
        if service:
            commands.update({"start": [*compose_base, "up", "-d", service], "stop": [*compose_base, "stop", service]})
        available = shutil.which("docker") is not None
        ownership = {"status": "absent", "method": "compose-label-mount", "runtime_ids": [], "checks": []}
        confidence = "declared"
        state = "unknown"
        runtime_conflict = False
        if available and service:
            ps_probe = command_probe(["docker", "compose", "--project-directory", str(root), "-f", str(path), "ps", "-a", "--format", "json"], timeout)
            probes.append({key: value for key, value in ps_probe.items() if key != "stdout"})
            ps_rows = parse_json_records(ps_probe.get("stdout", "")) if ps_probe.get("returncode") == 0 else []
            ids = [str(row.get("ID") or row.get("Id") or row.get("ContainerID") or "") for row in ps_rows if row.get("ID") or row.get("Id") or row.get("ContainerID")]
            inspect_rows = []
            if ids:
                inspect_probe = command_probe(["docker", "inspect", *ids], timeout)
                probes.append({key: value for key, value in inspect_probe.items() if key != "stdout"})
                inspect_rows = parse_json_records(inspect_probe.get("stdout", "")) if inspect_probe.get("returncode") == 0 else []
            assessment = assess_compose_ownership(root, path, service, ps_rows, inspect_rows)
            ownership = {"status": "proven" if assessment["proven"] else "partial" if ps_rows else "absent", "method": "compose-label-mount", "runtime_ids": assessment["runtime_ids"], "checks": assessment["checks"]}
            confidence = "owned" if assessment["proven"] else "correlated" if ps_rows else "declared"
            state = assessment["state"]
            runtime_conflict = assessment["conflict"]
        result.append({"kind": "docker-compose", "id": name, "service": service, "confidence": confidence, "state": state, "control_available": available, "ownership": ownership, "runtime_conflict": runtime_conflict, "evidence": [{"type": "compose-file", "source": name, "value": {"services": services}}], "execution_handoff": {"recommended_only": True, **commands}, "warnings": ["static compose evidence does not prove a running container owns this root"] + (["multiple services require an explicit ownership mapping"] if len(services) != 1 else [])})
    return result


def assess_windows_service_ownership(root: Path, service: dict, processes: list[dict]) -> dict:
    process_by_id = {}
    duplicate_pids = set()
    for item in processes:
        pid = int(item.get("ProcessId", 0) or 0)
        if pid in process_by_id:
            duplicate_pids.add(pid)
        process_by_id[pid] = item
    children: dict[int, list[int]] = {}
    for item in processes:
        children.setdefault(int(item.get("ParentProcessId", 0) or 0), []).append(int(item.get("ProcessId", 0) or 0))
    service_pid = int(service.get("ProcessId", 0) or 0)
    pending = [service_pid] if service_pid else []
    visited = set()
    descendants = []
    while pending and len(visited) < 256:
        pid = pending.pop()
        if pid in visited:
            continue
        visited.add(pid)
        if pid in process_by_id:
            descendants.append(process_by_id[pid])
        pending.extend(children.get(pid, [])[:64])
    identities = [process_identity(item) for item in descendants]
    identity_complete = all(item is not None for item in identities)
    chronological = identity_complete and all(parent_child_chronological(process_by_id, item) for item in descendants if int(item.get("ParentProcessId", 0) or 0) in visited)
    java = [item for item in descendants if str(item.get("Name", "")).casefold() in {"java.exe", "javaw.exe", "java", "javaw"} and command_jar_in_root(root, str(item.get("CommandLine") or "")) and process_identity(item) is not None]
    wrapper_ok = command_executable_in_root(root, str(service.get("PathName") or ""))
    service_executable_ok = service_pid in process_by_id and path_in_root(root, str(process_by_id[service_pid].get("ExecutablePath") or ""))
    running = str(service.get("State") or "").casefold() == "running" and service_pid > 0
    proven = running and wrapper_ok and service_executable_ok and len(java) == 1 and identity_complete and chronological and not duplicate_pids and service_pid in process_by_id
    bound_identities = [item for item in identities if item is not None] if proven else []
    return {"proven": proven, "conflict": bool(duplicate_pids), "state": "running" if running else "stopped", "runtime_ids": [service_pid, *[int(item.get("ProcessId")) for item in java]] if proven else [], "process_identities": bound_identities, "checks": {"service_running": running, "wrapper_in_root": wrapper_ok, "service_executable_in_root": service_executable_ok, "unique_java_descendant": len(java) == 1, "identity_complete": identity_complete, "chronological_tree": chronological, "unique_pids": not duplicate_pids}}


def creation_timestamp(raw: Any) -> float | None:
    value = str(raw or "")
    cim = re.fullmatch(r"(\d{14}\.\d{6})([+-])(\d{3})", value)
    if cim:
        try:
            naive = datetime.strptime(cim.group(1), "%Y%m%d%H%M%S.%f")
            minutes = int(cim.group(3)) * (1 if cim.group(2) == "+" else -1)
            return naive.replace(tzinfo=timezone(timedelta(minutes=minutes))).timestamp()
        except ValueError:
            return None
    formats = ("%Y-%m-%dT%H:%M:%S.%f%z", "%Y-%m-%dT%H:%M:%S%z")
    normalized = value[:-1] + "+0000" if value.endswith("Z") else value
    for candidate in (normalized, value):
        for pattern in formats:
            try:
                return datetime.strptime(candidate, pattern).timestamp()
            except ValueError:
                continue
    return None


def process_identity(process: dict) -> dict[str, Any] | None:
    try:
        pid = int(process.get("ProcessId", 0) or 0)
        parent = int(process.get("ParentProcessId", 0) or 0)
    except (TypeError, ValueError):
        return None
    created = creation_timestamp(process.get("CreationDate"))
    executable = str(process.get("ExecutablePath") or "")
    if pid <= 0 or created is None or not executable or not Path(executable).is_absolute():
        return None
    return {"pid": pid, "parent_pid": parent, "creation_date": str(process.get("CreationDate")), "creation_timestamp": created, "executable_path": str(Path(executable).resolve())}


def parent_child_chronological(processes: dict[int, dict], child: dict) -> bool:
    parent = processes.get(int(child.get("ParentProcessId", 0) or 0))
    if parent is None:
        return True
    parent_identity = process_identity(parent)
    child_identity = process_identity(child)
    return bool(parent_identity and child_identity and parent_identity["creation_timestamp"] <= child_identity["creation_timestamp"])


def path_in_root(root: Path, raw: str) -> bool:
    if not raw or not Path(raw).is_absolute():
        return False
    try:
        Path(raw).resolve().relative_to(root.resolve())
        return True
    except (OSError, ValueError):
        return False


def command_executable_in_root(root: Path, command: str) -> bool:
    stripped = command.strip()
    if not stripped:
        return False
    if stripped.startswith('"'):
        end = stripped.find('"', 1)
        executable = stripped[1:end] if end > 1 else ""
    else:
        executable = stripped.split(None, 1)[0]
    return path_in_root(root, executable)


def command_jar_in_root(root: Path, command: str) -> bool:
    match = re.search(r'(?i)(?:^|\s)-jar\s+(?:"([^"]+)"|(\S+))', command)
    return bool(match and path_in_root(root, match.group(1) or match.group(2)))


def windows_service_candidates(root: Path, timeout: float, probes: list[dict]) -> list[dict]:
    if os.name != "nt":
        return []
    script = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); $s=Get-CimInstance Win32_Service | Select-Object Name,DisplayName,State,ProcessId,PathName,StartName,StartMode; $p=Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine,CreationDate; @{services=$s;processes=$p} | ConvertTo-Json -Compress -Depth 4"
    snapshots = []
    for index in range(2):
        probe = command_probe(["powershell", "-NoProfile", "-Command", script], timeout)
        probes.append({key: value for key, value in probe.items() if key != "stdout"})
        if probe.get("returncode") != 0:
            return []
        try:
            snapshots.append(json.loads(probe.get("stdout", "")))
        except json.JSONDecodeError:
            return []
        if index == 0:
            time.sleep(0.15)
    first_data, data = snapshots
    services = data.get("services", []) if isinstance(data, dict) else []
    processes = data.get("processes", []) if isinstance(data, dict) else []
    first_services = first_data.get("services", []) if isinstance(first_data, dict) else []
    first_processes = first_data.get("processes", []) if isinstance(first_data, dict) else []
    services = services if isinstance(services, list) else [services]
    processes = processes if isinstance(processes, list) else [processes]
    first_services = first_services if isinstance(first_services, list) else [first_services]
    first_processes = first_processes if isinstance(first_processes, list) else [first_processes]
    first_by_name = {str(item.get("Name", "")).casefold(): item for item in first_services}
    result = []
    for service in services:
        command = str(service.get("PathName") or "")
        if not command_executable_in_root(root, command) and not command_jar_in_root(root, command):
            continue
        assessment = assess_windows_service_ownership(root, service, processes)
        first_service = first_by_name.get(str(service.get("Name", "")).casefold())
        first_assessment = assess_windows_service_ownership(root, first_service, first_processes) if first_service else {"proven": False, "runtime_ids": [], "process_identities": []}
        stable = bool(first_assessment["proven"] and assessment["proven"] and first_assessment["runtime_ids"] == assessment["runtime_ids"] and first_assessment["process_identities"] == assessment["process_identities"] and first_service.get("ProcessId") == service.get("ProcessId"))
        assessment["checks"]["double_sample_stable"] = stable
        result.append({"kind": "windows-service", "id": service.get("Name"), "confidence": "owned" if stable else "correlated", "state": assessment["state"], "control_available": shutil.which("sc.exe") is not None, "ownership": {"status": "proven" if stable else "partial", "method": "windows-service-process-tree-double-sample", "runtime_ids": assessment["runtime_ids"] if stable else [], "process_identities": assessment["process_identities"] if stable else [], "checks": assessment["checks"]}, "runtime_conflict": assessment["conflict"], "evidence": [{"type": "service", "source": service.get("Name"), "value": {"PathName": command, "ProcessId": service.get("ProcessId"), "StartName": service.get("StartName")}}], "execution_handoff": {"recommended_only": True, "status": ["sc.exe", "query", service.get("Name")], "start": ["sc.exe", "start", service.get("Name")], "stop": ["sc.exe", "stop", service.get("Name")]}, "warnings": [] if stable else ["service correlation does not prove a stable child Java process identity across two samples"]})
    return result


def run() -> int:
    parser = JsonArgumentParser(description="Identify existing Minecraft server supervisors without controlling them")
    parser.add_argument("server_root")
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    probes = []
    supervisors = script_candidates(root) + compose_candidates(root, args.timeout, probes) + systemd_candidates(root, args.timeout, probes) + windows_service_candidates(root, min(args.timeout, 10.0), probes)
    high = [item for item in supervisors if CONFIDENCE[item["confidence"]] >= CONFIDENCE["correlated"]]
    selected = high[0] if len(high) == 1 else None
    conflicts = [] if len(high) <= 1 else [{"code": "multiple-correlated-supervisors", "ids": [item["id"] for item in high]}]
    conflicts.extend({"code": "multiple-runtime-instances", "id": item["id"]} for item in supervisors if item.get("runtime_conflict"))
    evidence_complete = selected is not None and selected["confidence"] == "owned" and not conflicts
    emit({"ok": True, "server_root": str(root), "supervisors": supervisors, "selected": selected, "ownership_evidence_complete": evidence_complete, "conflicts": conflicts, "probes": probes, "analysis_only": True, "execution_performed": False, "note": "This is evidence for an external executor; discovery never authorizes or executes lifecycle actions."}, args.pretty)
    return 0


if __name__ == "__main__":
    main_guard(run)
