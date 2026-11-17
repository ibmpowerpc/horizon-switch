#!/usr/bin/env python3
"""Measure this project's experiment outputs; never delete or fetch anything."""
import argparse
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--reserve-mib", type=int, default=0,
                    help="Minimum free space requested for this operation; no fixed artifact cap")
args = parser.parse_args()
if args.reserve_mib < 0:
    parser.error("reserve must be nonnegative")
root = Path(__file__).resolve().parents[2]
patterns = (
    "platform/switch/*/target", "platform/switch/*/build*",
    "platform/switch/*/*/target", "platform/switch/*/*/build*",
    "tools/switch-abi-audit/build", "tools/switch-abi-audit/rust/build",
    "dist/switch",
    ".local/switch-assets", ".local/switch-state-facts",
    ".local/switch-models", "tools/switch_model_prep/build*", "tools/switch_model_prep/target",
    *(f"platform/switch/probe/*.{ext}" for ext in ("elf", "nro", "nacp", "map", "lst")),
)
paths = {p for pattern in patterns for p in root.glob(pattern)
         if p.is_dir() or p.suffix in (".elf", ".nro", ".nacp", ".map", ".lst")}
# A parent output directory already includes all its children.
paths = {p for p in paths if not any(q in paths for q in p.parents)}
kib = sum(int(subprocess.check_output(["du", "-sk", str(p)], text=True).split()[0])
          for p in paths)
free = shutil.disk_usage(root).free
print(f"Experiment artifacts: {kib / 1024:.1f} MiB; free: {free / 2**30:.1f} GiB")
reserve = args.reserve_mib * 1024 * 1024
if free < reserve:
    raise SystemExit("Insufficient free space for the explicitly requested build headroom.")
