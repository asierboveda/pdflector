#!/usr/bin/env python3
"""CLI for deterministic analysis of an ink benchmark JSONL stream."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parent))

from ink_bench.analyzer import AnalysisError, analyze_file


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Analiza resultados JSONL de tinta")
    parser.add_argument("jsonl", help="fichero JSONL del benchmark")
    args = parser.parse_args(argv)
    try:
        result = analyze_file(args.jsonl)
    except AnalysisError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
