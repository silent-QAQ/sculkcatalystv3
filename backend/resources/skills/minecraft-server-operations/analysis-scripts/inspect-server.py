from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path

from _common import JsonArgumentParser, emit, limited_files, main_guard, parse_cli_root, rel, sha256_file


VERSION_PATTERNS = [
    re.compile(r"Starting minecraft server version ([^\s]+)", re.I),
    re.compile(r"Running Java ([^\r\n]+)", re.I),
    re.compile(r"This server is running ([^\r\n]+)", re.I),
]


def read_properties(path: Path) -> dict[str, str]:
    data: dict[str, str] = {}
    if not path.is_file():
        return data
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.strip()
        if line and not line.startswith(("#", "!")) and "=" in line:
            key, value = line.split("=", 1)
            data[key.strip()] = value.strip()
    return data


def jar_markers(path: Path) -> list[str]:
    markers: list[str] = []
    try:
        with zipfile.ZipFile(path) as archive:
            names = set(archive.namelist())
            checks = {
                "paper": "META-INF/io.papermc.paper/paper-server",
                "bukkit": "org/bukkit/Bukkit.class",
                "fabric": "fabric.mod.json",
                "forge": "META-INF/mods.toml",
                "neoforge": "META-INF/neoforge.mods.toml",
                "velocity": "com/velocitypowered/proxy/Velocity.class",
                "bungeecord": "net/md_5/bungee/BungeeCord.class",
            }
            for name, marker in checks.items():
                if marker in names or any(item.startswith(marker) for item in names):
                    markers.append(name)
    except (OSError, zipfile.BadZipFile):
        markers.append("invalid-jar")
    return markers


def detect_platform(root: Path, jars: list[Path]) -> list[str]:
    found: set[str] = set()
    names = " ".join(path.name.lower() for path in jars)
    for platform in ("paper", "purpur", "folia", "spigot", "fabric", "forge", "neoforge", "velocity", "bungee"):
        if platform in names:
            found.add("bungeecord" if platform == "bungee" else platform)
    if (root / "config" / "paper-global.yml").exists() or (root / "paper.yml").exists():
        found.add("paper")
    if (root / "config" / "fabric-loader.properties").exists() or ((root / "mods").is_dir() and any((root / "mods").glob("*.jar"))):
        found.add("fabric-or-mod-loader")
    for jar in jars[:20]:
        found.update(jar_markers(jar))
    if not found and (root / "server.properties").exists():
        found.add("vanilla-or-bukkit")
    return sorted(found)


def run() -> int:
    parser = JsonArgumentParser(description="Inspect a Minecraft server directory")
    parser.add_argument("server_root")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    root = parse_cli_root(args.server_root)
    jars = limited_files(root, ("*.jar", "plugins/*.jar", "mods/*.jar"), 1000)
    configs = limited_files(root, ("*.properties", "*.yml", "*.yaml", "*.json", "*.toml", "config/**/*.*", "plugins/**/*.yml"), 1000)
    scripts = limited_files(root, ("*.bat", "*.cmd", "*.ps1", "*.sh", "docker-compose*.yml", "docker-compose*.yaml", "compose*.yml", "compose*.yaml", "*.service"), 100)
    worlds = [path for path in root.iterdir() if path.is_dir() and ((path / "level.dat").exists() or (path / "region").is_dir())]
    props = read_properties(root / "server.properties")
    recent_log = root / "logs" / "latest.log"
    evidence: list[str] = []
    if recent_log.is_file():
        tail = recent_log.read_text(encoding="utf-8", errors="replace")[-300_000:]
        for pattern in VERSION_PATTERNS:
            evidence.extend(match.group(0) for match in pattern.finditer(tail))
    artifact_rows = []
    for jar in jars:
        stat = jar.stat()
        artifact_rows.append({"path": rel(root, jar), "size": stat.st_size, "sha256": sha256_file(jar), "markers": jar_markers(jar)})
    payload = {
        "ok": True,
        "server_root": str(root),
        "platform_candidates": detect_platform(root, jars),
        "server_properties": {key: props.get(key) for key in ("server-port", "level-name", "online-mode", "enable-rcon", "query.port") if key in props},
        "worlds": [rel(root, path) for path in worlds],
        "launch_files": [rel(root, path) for path in scripts],
        "configuration_count": len(configs),
        "configurations": [rel(root, path) for path in configs[:200]],
        "artifacts": artifact_rows,
        "log_evidence": evidence[-30:],
    }
    emit(payload, args.pretty)
    return 0


if __name__ == "__main__":
    main_guard(run)
