#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Derive and add missing GameType JSON files.

One-command workflow (writes and packages by default):

  1. Auto-locate the current workspace FrameAnalysis dump directory and game type
     from the SSMT4 global config (no manual searching needed).
  2. Run a fast layout-signature coverage check against the local GameType registry.
     Every IB that already matches is reported as covered and skipped.
  3. Only IBs with no match are derived. IBs sharing the same layout are deduplicated
     into a single new type. IBs whose layout cannot be resolved (no VB headers, or a
     single-element layout) are marked as unable and left alone.
  4. Generate one GameType JSON per distinct missing type and write it into the
     current game's GameType folder.
  5. Package all newly written files into a shareable zip archive.

Use --dry-run to preview without writing anything.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import zipfile

import gametype_common as common
import gametype_ib_check as checker


COVERED_TEXT = "已有数据类型"
NEW_TYPE_TEXT = "新增数据类型"
UNABLE_TEXT = "无法识别"


def _unique_name(gametype_dir, base_name):
    candidate = base_name
    idx = 2
    while os.path.exists(os.path.join(gametype_dir, candidate + ".json")):
        candidate = f"{base_name}_{idx}"
        idx += 1
    return candidate


def package_written_types(gametype_dir, written_paths):
    """Zip newly written GameType JSON files, preserving the game-type folder."""
    root = os.path.dirname(gametype_dir)
    game_type = os.path.basename(gametype_dir)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    zip_path = os.path.join(root, f"gametype-additions-{game_type}-{stamp}.zip")
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as zf:
        for path in written_paths:
            rel = os.path.relpath(path, root)
            zf.write(path, rel)
    return zip_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root",
        nargs="?",
        default=None,
        help="FrameAnalysis folder (auto-discovered from SSMT4 settings if omitted)",
    )
    parser.add_argument(
        "--gametype-dir",
        default=None,
        help="GameType folder override (auto-discovered from the current game when omitted)",
    )
    parser.add_argument(
        "--settings-file",
        default=None,
        help="Absolute path to SSMT4 settings.json (prefer MCP GlobalConfig.AppSettingsFilePath)",
    )
    parser.add_argument(
        "--gametype-root",
        default=None,
        help="GameType library directory (prefer MCP GlobalConfig.SSMTResourcesFolder()/GameType)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="preview only; do not write JSON or create a zip",
    )
    parser.add_argument(
        "--drawib-file",
        default=None,
        help="Explicit DrawIB list JSON (workspace Config.json format, or a plain array of hashes)",
    )
    parser.add_argument(
        "--scan-all",
        action="store_true",
        help="Audit every IB in the dump instead of the workspace DrawIB list",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="print every covered/unable IB (default prints only a summary)",
    )
    parser.add_argument("--json", default=None, help="write a JSON report to this path")
    args = parser.parse_args()

    root = common.discover_frame_analysis_root(args.root, settings_file=args.settings_file)
    game_type = common.discover_game_type(settings_file=args.settings_file, gametype_root=args.gametype_root)
    gametype_dir = common.resolve_gametype_dir(args.gametype_dir or game_type, gametype_root=args.gametype_root)
    if not gametype_dir:
        print(
            f"[add] GameType folder not found for game type '{game_type or args.gametype_dir}'. "
            "Pass --gametype-dir explicitly.",
            file=sys.stderr,
        )
        sys.exit(2)
    gametype_dir = os.path.abspath(gametype_dir)
    print(f"[add] Game type: {game_type}; GameType folder: {gametype_dir}")

    files = common.normalize_filenames(root)
    log_path = os.path.join(root, "log.txt")
    lines = []
    if os.path.isfile(log_path):
        with open(log_path, "r", encoding="utf-8", errors="replace") as handle:
            lines = handle.read().splitlines()

    deduped_map = checker.build_deduped_map(lines)
    types = checker.load_types(gametype_dir)

    drawib_list = []
    if args.drawib_file:
        drawib_list = common.read_drawib_list_file(args.drawib_file)
    elif not args.scan_all:
        settings = common.read_settings(args.settings_file)
        drawib_list = common.read_drawib_list(common.resolve_workspace_dir(settings))

    if drawib_list:
        print(f"[add] IB list source: workspace DrawIB list ({len(drawib_list)} IBs)")
    else:
        print("[add] IB list source: full dump scan (no workspace DrawIB list found)")

    coverage, details = checker.check_coverage(
        types, files, root, deduped_map, lines, ib_filter=drawib_list or None
    )

    covered_count = 0
    unable_count = 0
    to_write = {}  # base_name -> {"gpu": bool, "by_slot": dict, "ibs": [str]}

    for ib in sorted(coverage):
        matched = coverage[ib]
        if matched:
            covered_count += 1
            if args.verbose:
                print(f"[add] IB {ib}: {COVERED_TEXT} ({', '.join(matched)})")
            continue

        detail = details.get(ib) or {}
        index = detail.get("index")
        gpu = bool(detail.get("gpu"))
        by_slot = detail.get("by_slot") or {}

        if not index or not by_slot:
            unable_count += 1
            if args.verbose:
                print(f"[add] IB {ib}: {UNABLE_TEXT} (no VB headers)", file=sys.stderr)
            continue

        if sum(len(v) for v in by_slot.values()) < 2:
            unable_count += 1
            if args.verbose:
                print(f"[add] IB {ib}: {UNABLE_TEXT} (incomplete layout)", file=sys.stderr)
            continue

        base_name = common.encode_layout_name(gpu, by_slot)
        rec = to_write.setdefault(base_name, {"gpu": gpu, "by_slot": by_slot, "ibs": []})
        rec["ibs"].append(ib)

    written = []
    report = []
    for base_name in sorted(to_write):
        rec = to_write[base_name]
        json_elements = common.build_game_type_json(rec["by_slot"], rec["gpu"])
        final_name = _unique_name(gametype_dir, base_name)
        filename = final_name + ".json"
        payload = {"D3D11ElementList": json_elements}
        record = {
            "type_name": final_name,
            "gpu": rec["gpu"],
            "ib_count": len(rec["ibs"]),
            "ibs": sorted(rec["ibs"]),
            "status": "written" if not args.dry_run else "derived",
            "filename": filename,
            "elements": json_elements,
        }
        report.append(record)
        if args.dry_run:
            print(f"[add] 推导 {filename} (GPU={rec['gpu']}, {len(rec['ibs'])} 个 IB)")
        else:
            target = os.path.join(gametype_dir, filename)
            with open(target, "w", encoding="utf-8") as handle:
                json.dump(payload, handle, ensure_ascii=False, indent=2)
            written.append(target)
            print(f"[add] {NEW_TYPE_TEXT} {filename} ({len(rec['ibs'])} 个 IB)")

    if args.json:
        with open(args.json, "w", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)

    total = len(coverage)
    print()
    print(f"[add] checked {total} IB hashes")
    print(f"[add] covered: {covered_count} ({COVERED_TEXT})")
    print(f"[add] unable: {unable_count} ({UNABLE_TEXT})")
    print(f"[add] new: {len(to_write)} ({NEW_TYPE_TEXT})")

    if args.dry_run:
        print("[add] dry-run: 预览模式，不会写入文件；去掉 --dry-run 参数后正式运行")
    elif written:
        package_path = package_written_types(gametype_dir, written)
        print()
        print("[add] 新增的数据类型已打包为压缩包，可直接分享给开发人员：")
        print(f"[add]   {package_path}")
        print(f"[add] 共 {len(written)} 个文件")
    else:
        print("[add] 没有需要新增的数据类型")


if __name__ == "__main__":
    main()
