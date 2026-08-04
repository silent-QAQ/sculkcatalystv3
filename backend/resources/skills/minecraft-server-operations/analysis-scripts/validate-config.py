from __future__ import annotations

import argparse
import json
import re
import tomllib
from pathlib import Path

from _common import JsonArgumentParser, emit, main_guard, sha256_file


def validate_properties(text: str) -> list[str]:
    errors: list[str] = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith(("#", "!")):
            continue
        if "=" not in line and ":" not in line:
            errors.append(f"line {number}: expected key=value")
    return errors


def validate_yaml_basic(text: str) -> list[str]:
    errors: list[str] = []
    stack = []
    for number, raw in enumerate(text.splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if "\t" in raw[: len(raw) - len(raw.lstrip())]:
            errors.append(f"line {number}: tab indentation is not allowed")
        stripped = re.sub(r"\s+#.*$", "", raw.strip())
        for char in stripped:
            if char in "[{":
                stack.append((char, number))
            elif char in "]}":
                expected = "[" if char == "]" else "{"
                if not stack or stack.pop()[0] != expected:
                    errors.append(f"line {number}: unbalanced {char}")
                    break
    if stack:
        errors.append(f"line {stack[-1][1]}: unclosed {stack[-1][0]}")
    return errors


def run() -> int:
    parser = JsonArgumentParser(description="Validate a Minecraft configuration file")
    parser.add_argument("file")
    parser.add_argument("--pretty", action="store_true")
    args = parser.parse_args()
    path = Path(args.file).expanduser().resolve(strict=True)
    raw = path.read_bytes()
    if b"\x00" in raw:
        emit({"ok": False, "file": str(path), "errors": ["binary file: NUL byte detected"]}, args.pretty)
        return 1
    text = raw.decode("utf-8-sig")
    suffix = path.suffix.lower()
    errors: list[str] = []
    parser_name = "text"
    try:
        if suffix == ".json":
            parser_name = "json"
            json.loads(text)
        elif suffix == ".toml":
            parser_name = "tomllib"
            tomllib.loads(text)
        elif suffix in {".properties", ".conf", ".cfg"}:
            parser_name = "properties"
            errors.extend(validate_properties(text))
        elif suffix in {".yml", ".yaml"}:
            try:
                import yaml  # type: ignore
                parser_name = "PyYAML"
                try:
                    yaml.safe_load(text)
                except yaml.YAMLError as exc:
                    errors.append(str(exc))
            except ImportError:
                parser_name = "unavailable"
                errors.append("reliable YAML validation requires PyYAML; file was not validated")
    except (UnicodeDecodeError, json.JSONDecodeError, tomllib.TOMLDecodeError) as exc:
        errors.append(str(exc))
    emit({"ok": not errors, "file": str(path), "format": parser_name, "size": len(raw), "sha256": sha256_file(path), "errors": errors}, args.pretty)
    return 1 if errors else 0


if __name__ == "__main__":
    main_guard(run)
