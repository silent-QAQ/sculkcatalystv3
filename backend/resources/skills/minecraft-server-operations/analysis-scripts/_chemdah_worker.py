from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


def run() -> int:
    if len(sys.argv) != 6:
        return 2
    script = Path(__file__).with_name("analyze-chemdah.py")
    spec = importlib.util.spec_from_file_location("chemdah_analyzer", script)
    if spec is None or spec.loader is None:
        return 2
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    root, plugin_root, kind, path, max_bytes = sys.argv[1:]
    payload = module.analyze_one_file(Path(root), Path(plugin_root), kind, Path(path), int(max_bytes))
    print(json.dumps(payload, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(run())
