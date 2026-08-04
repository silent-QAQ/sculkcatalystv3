from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import socket
import struct
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


SECRET_KEY = re.compile(r"(?i)(password|passwd|secret|token|api[-_]?key|rcon\.password)")
SECRET_TEXT = [
    re.compile(r"(?i)(authorization\s*:\s*bearer\s+)[^\s]+"),
    re.compile(r"(?i)((?:password|passwd|secret|token|api[-_]?key|rcon\.password)\s*[=:]\s*)[^\s,;]+"),
    re.compile(r"https://discord(?:app)?\.com/api/webhooks/[^\s]+", re.I),
]


class JsonArgumentParser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError(f"argument error: {message}")


def sha256_file(path: Path, chunk_size: int = 1024 * 1024) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(chunk_size):
            digest.update(chunk)
    return digest.hexdigest()


def operation_id() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def encode_varint(value: int) -> bytes:
    value &= 0xFFFFFFFF
    output = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        output.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(output)


def read_exact(stream: socket.socket, length: int, deadline: float | None = None) -> bytes:
    output = bytearray()
    while len(output) < length:
        if deadline is not None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise socket.timeout("protocol deadline exceeded")
            stream.settimeout(remaining)
        chunk = stream.recv(length - len(output))
        if not chunk:
            raise ConnectionError("connection closed before response completed")
        output.extend(chunk)
    return bytes(output)


def read_varint(stream: socket.socket, deadline: float | None = None) -> int:
    value = 0
    for index in range(5):
        byte = read_exact(stream, 1, deadline)[0]
        value |= (byte & 0x7F) << (7 * index)
        if not byte & 0x80:
            return value
    raise ValueError("VarInt exceeds 5 bytes")


def minecraft_status_ping(host: str, port: int, timeout: float = 3.0, max_frame: int = 1024 * 1024, handshake_host: str | None = None, protocol_version: int = 47) -> dict[str, Any]:
    if not 0 < port < 65536:
        raise ValueError(f"invalid port: {port}")
    handshake_host = handshake_host or host
    encoded_host = handshake_host.encode("utf-8")
    if len(handshake_host) > 255 or len(encoded_host) > 1020:
        raise ValueError("handshake host exceeds protocol limit")
    handshake = encode_varint(0) + encode_varint(protocol_version) + encode_varint(len(encoded_host)) + encoded_host + struct.pack(">H", port) + encode_varint(1)
    request = encode_varint(len(handshake)) + handshake + b"\x01\x00"
    started = time.monotonic()
    deadline = started + timeout
    try:
        with socket.create_connection((host, port), timeout=timeout) as stream:
            stream.settimeout(max(deadline - time.monotonic(), 0.001))
            stream.sendall(request)
            try:
                frame_length = read_varint(stream, deadline)
                if frame_length <= 0 or frame_length > max_frame:
                    raise ValueError(f"invalid status frame length: {frame_length}")
                frame = read_exact(stream, frame_length, deadline)
                packet_id, used = decode_varint_bytes(frame)
                if packet_id != 0:
                    raise ValueError(f"unexpected status packet id: {packet_id}")
                json_length, length_used = decode_varint_bytes(frame[used:])
                payload = frame[used + length_used :]
                if json_length != len(payload) or json_length > max_frame:
                    raise ValueError("status JSON length does not match frame")
                status = json.loads(payload.decode("utf-8"))
                if not isinstance(status, dict):
                    raise ValueError("status response must be a JSON object")
                return {"tcp_open": True, "minecraft_status": "confirmed", "detail": "valid Java status response", "error_code": None, "latency_ms": round((time.monotonic() - started) * 1000, 2), "protocol_version": protocol_version, "handshake_host": handshake_host, "response": status}
            except socket.timeout:
                return {"tcp_open": True, "minecraft_status": "unknown", "detail": "TCP opened but status response timed out", "error_code": "timeout", "response": None}
            except (ConnectionError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as exc:
                return {"tcp_open": True, "minecraft_status": "rejected", "detail": str(exc), "error_code": "malformed-status-response", "response": None}
    except (ConnectionRefusedError, TimeoutError, socket.timeout, OSError) as exc:
        return {"tcp_open": False, "minecraft_status": "unknown", "detail": str(exc), "error_code": "connect-failed", "response": None}


def decode_varint_bytes(data: bytes) -> tuple[int, int]:
    value = 0
    for index, byte in enumerate(data[:5]):
        value |= (byte & 0x7F) << (7 * index)
        if not byte & 0x80:
            return value, index + 1
    raise ValueError("incomplete or oversized VarInt")


def log_identity(path: Path) -> dict[str, Any]:
    stat = path.stat()
    return {"path": path.name, "device": stat.st_dev, "inode": stat.st_ino, "size": stat.st_size, "mtime_ns": stat.st_mtime_ns}


def load_cycle_baseline(root: Path, baseline_path: Path, log_path: Path) -> dict[str, Any]:
    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    if baseline.get("schema") != 1 or not isinstance(baseline.get("log"), dict):
        raise ValueError("invalid cycle baseline schema")
    declared_root = Path(str(baseline.get("server_root", ""))).resolve()
    if declared_root != root.resolve():
        raise ValueError("cycle baseline belongs to a different server root")
    current = log_identity(log_path)
    recorded = baseline["log"]
    if recorded.get("device") is None and recorded.get("inode") is None and int(recorded.get("size", 0)) == 0:
        return {"valid": True, "reason": "log-created-after-baseline", "segments": [{"path": str(log_path.resolve()), "start": 0, "end": current["size"], "device": current["device"], "inode": current["inode"]}], "recorded": recorded, "current": current}
    same_identity = current["device"] == recorded.get("device") and current["inode"] == recorded.get("inode")
    valid = same_identity and current["size"] >= int(recorded.get("size", 0))
    if valid:
        return {"valid": True, "reason": "valid", "segments": [{"path": str(log_path.resolve()), "start": int(recorded["size"]), "end": current["size"], "device": current["device"], "inode": current["inode"]}], "recorded": recorded, "current": current}
    if same_identity:
        return {"valid": False, "reason": "log-truncated", "segments": [], "recorded": recorded, "current": current}
    matches = []
    for candidate in log_path.parent.iterdir():
        if candidate == log_path or not candidate.is_file() or is_link_or_reparse(candidate):
            continue
        try:
            identity = log_identity(candidate)
        except OSError:
            continue
        if identity["device"] == recorded.get("device") and identity["inode"] == recorded.get("inode"):
            matches.append((candidate.resolve(), identity))
    if len(matches) != 1:
        reason = "rotated-log-missing" if not matches else "rotated-log-ambiguous"
        return {"valid": False, "reason": reason, "segments": [], "recorded": recorded, "current": current}
    rotated, rotated_identity = matches[0]
    if rotated_identity["size"] < int(recorded.get("size", 0)):
        return {"valid": False, "reason": "rotated-log-truncated", "segments": [], "recorded": recorded, "current": current}
    segments = [
        {"path": str(rotated), "start": int(recorded["size"]), "end": rotated_identity["size"], "device": rotated_identity["device"], "inode": rotated_identity["inode"]},
        {"path": str(log_path.resolve()), "start": 0, "end": current["size"], "device": current["device"], "inode": current["inode"]},
    ]
    return {"valid": True, "reason": "single-rotation-recovered", "segments": segments, "recorded": recorded, "current": current}


def read_stable_segment(segment: dict[str, Any], max_bytes: int) -> tuple[bytes, dict[str, Any]]:
    path = Path(segment["path"])
    if is_link_or_reparse(path):
        raise ValueError(f"refusing linked log segment: {path}")
    start, end = int(segment["start"]), int(segment["end"])
    if start < 0 or end < start:
        raise ValueError(f"invalid log segment bounds: {path}")
    length = end - start
    if length > max_bytes:
        raise ValueError(f"log segment exceeds byte budget: {path}")
    with path.open("rb") as handle:
        before = os.fstat(handle.fileno())
        if before.st_dev != segment["device"] or before.st_ino != segment["inode"] or before.st_size < end:
            raise ValueError(f"log segment identity or size drifted: {path}")
        handle.seek(start)
        raw = handle.read(length)
        after = os.fstat(handle.fileno())
    if len(raw) != length or after.st_dev != before.st_dev or after.st_ino != before.st_ino or after.st_size < end:
        raise ValueError(f"unstable or short log segment read: {path}")
    return raw, {"path": str(path), "start": start, "end": end, "bytes": length}


def is_link_or_reparse(path: Path) -> bool:
    if path.is_symlink():
        return True
    try:
        attributes = path.lstat().st_file_attributes  # type: ignore[attr-defined]
        return bool(attributes & 0x400)
    except (AttributeError, OSError):
        return False


def redact(value: Any, key: str = "") -> Any:
    if SECRET_KEY.search(key):
        return "<redacted>"
    if isinstance(value, dict):
        return {k: redact(v, str(k)) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(item) for item in value]
    if isinstance(value, str):
        for pattern in SECRET_TEXT:
            value = pattern.sub(lambda match: (match.group(1) if match.lastindex else "") + "<redacted>", value)
    return value


def emit(payload: dict[str, Any], pretty: bool = False) -> None:
    print(json.dumps(redact(payload), ensure_ascii=False, indent=2 if pretty else None))


def rel(root: Path, path: Path) -> str:
    return path.resolve().relative_to(root.resolve()).as_posix()


def limited_files(root: Path, patterns: Iterable[str], limit: int = 500) -> list[Path]:
    result: list[Path] = []
    for pattern in patterns:
        for path in root.glob(pattern):
            if path.is_file():
                result.append(path)
                if len(result) >= limit:
                    return sorted(set(result))
    return sorted(set(result))


def error_payload(message: str, **extra: Any) -> dict[str, Any]:
    return {"ok": False, "error": message, **extra}


def parse_cli_root(raw: str) -> Path:
    root = Path(raw).expanduser().resolve(strict=True)
    if not root.is_dir():
        raise ValueError(f"not a directory: {root}")
    return root


def main_guard(function: Any) -> None:
    try:
        code = function()
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        emit(error_payload(str(exc)))
        code = 2
    except Exception as exc:
        emit(error_payload("unexpected operational failure", error_type=type(exc).__name__))
        code = 2
    sys.exit(code)
