
#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
FrameAnalysis IB -> GameType coverage checker.

Two layers are provided:

1. A fast layout-signature check (used by default). It derives the real
   per-buffer vertex layout from each IB's representative draw and compares it
   against the local GameType registry. Covered IBs are reported immediately and
   never go through the expensive full matching.

2. The original Unity VS matching algorithm (``get_possible`` / ``match_type``)
   is kept as a deep-verify fallback for cases the signature check cannot decide.

Usage:
  python gametype_ib_check.py [frame_analysis_root] [--settings-file <path>] [--gametype-root <dir>] [--gametype-dir <dir>] [--verbose] [--json <out>]
"""
import argparse
import collections
import json
import os
import re
import sys

import gametype_common as common


_VB_SLOT_RE = re.compile(r"^\d{6}-vb(\d+)=")


def bw_from_format(fmt: str) -> int:
    return common.bw_from_format(fmt)


def build_index(files):
    return sorted(set(files))


def first_file(files, content, suffix):
    for f in files:
        if content in f and f.endswith(suffix):
            return f
    return ""


def build_deduped_map(lines):
    out = {}
    for line in lines:
        if "->" not in line:
            continue
        start = None
        if "Dumping Texture2D" in line:
            start = line.find("Dumping Texture2D") + len("Dumping Texture2D")
        elif "Dumping Buffer" in line:
            start = line.find("Dumping Buffer") + len("Dumping Buffer")
        else:
            continue
        splits = line[start:].split("->")
        if len(splits) < 2:
            continue
        orig = os.path.basename(splits[-2].strip())
        ded = os.path.basename(splits[-1].strip())
        if orig.endswith(".lnk"):
            orig = orig[:-4]
        if ded.endswith(".lnk"):
            ded = ded[:-4]
        out[orig] = ded
    return out


def deduped_path(root, deduped_map, name):
    d = deduped_map.get(name, "")
    return os.path.join(root, "deduped", d) if d else ""


def load_types(gametype_dir):
    types = []
    for jp in sorted(os.listdir(gametype_dir)):
        if not jp.endswith(".json"):
            continue
        name = jp[:-5]
        data = json.load(open(os.path.join(gametype_dir, jp), encoding="utf-8"))
        categories = []
        cat_slot = {}
        cat_topo = {}
        cat_stride = collections.Counter()
        elem_dict = {}
        counts = collections.Counter()
        for e in data["D3D11ElementList"]:
            sem = e["SemanticName"]
            idx = counts[sem]
            counts[sem] += 1
            el_name = sem if idx == 0 else f"{sem}{idx}"
            cat = e["Category"]
            if cat not in categories:
                categories.append(cat)
            cat_slot[cat] = e["ExtractSlot"]
            cat_topo[cat] = e["ExtractTechnique"]
            cat_stride[cat] += int(e["ByteWidth"])
            elem_dict[el_name] = {"category": cat, "byte_width": int(e["ByteWidth"])}
        gpu = any(e["SemanticName"] == "BLENDINDICES" for e in data["D3D11ElementList"]) or any(
            e["ExtractTechnique"] == "pointlist" for e in data["D3D11ElementList"]
        )
        types.append(
            {
                "name": name,
                "gpu": gpu,
                "categories": categories,
                "cat_slot": cat_slot,
                "cat_topo": cat_topo,
                "cat_stride": cat_stride,
                "elem_dict": elem_dict,
                "self_stride": sum(cat_stride[c] for c in categories),
            }
        )
    types.sort(key=lambda t: (not t["gpu"], t["name"]))
    return types


def get_trianglelist_index_list(files, draw_ib):
    idx_set = set()
    needle = f"-ib={draw_ib}"
    for f in files:
        if needle in f and f.endswith(".txt"):
            idx_set.add(f[:6])
    if not idx_set:
        for f in files:
            if needle in f and f.endswith(".buf"):
                idx_set.add(f[:6])
    return sorted(idx_set)


def get_all_drawib_list(files):
    out = set()
    for f in files:
        if "-ib=" in f and f.endswith(".txt"):
            h = f[10:18]
            if len(h) == 8:
                out.add(h)
    if not out:
        for f in files:
            m = re.search(r"-ib=([0-9a-fA-F]{8})", f)
            if m and f.endswith((".txt", ".buf")):
                out.add(m.group(1).lower())
    return sorted(out)


def get_drawcall_index_list_by_hash(lines, draw_ib):
    index_list = []
    current = ""
    needle_hash = f"hash={draw_ib}"
    needle_ib = f"-ib={draw_ib}"
    for line in lines:
        if line.startswith("00") and len(line) >= 6:
            current = line[:6]
        if needle_hash in line or needle_ib in line:
            if current and current not in index_list:
                index_list.append(current)
    return index_list


def get_line_list_by_index(lines, index):
    try:
        index_number = int(index)
    except ValueError:
        return []
    out = []
    find = False
    for line in lines:
        if line.startswith("00") and not find:
            sub = line[:6]
            if sub.isdigit() and int(sub) == index_number:
                find = True
                out.append(line)
                continue
        if find:
            if line.startswith("00"):
                sub = line[:6]
                if sub.isdigit():
                    if int(sub) > index_number:
                        break
                    out.append(line)
                else:
                    out.append(line)
            else:
                out.append(line)
    return out


def get_last_pointlist_index_by_hash(lines, draw_ib):
    dl = get_drawcall_index_list_by_hash(lines, draw_ib)
    if not dl:
        return None
    first_tl = dl[0]
    line_list = get_line_list_by_index(lines, first_tl)
    vb0_hash = ""
    find_ia = False
    for call_line in line_list:
        if "IASetVertexBuffers" in call_line and not find_ia:
            find_ia = True
            continue
        if find_ia:
            if not call_line.startswith("00"):
                parts = call_line.split(":", 1)
                if len(parts) == 2 and parts[0].strip() == "0":
                    for kv in parts[1].split():
                        if kv.startswith("hash="):
                            vb0_hash = kv[5:]
                            break
            else:
                break
    if not vb0_hash:
        return None
    current = ""
    possible = []
    try:
        tl_num = int(first_tl)
    except ValueError:
        return None
    for log_line in lines:
        if log_line.startswith("00"):
            current = log_line[:6]
        if f"hash={vb0_hash}" in log_line and "dst" not in log_line.lower():
            try:
                pl = int(current)
            except ValueError:
                continue
            if pl < tl_num and current not in possible:
                possible.append(current)
    return possible[-1] if possible else None


def vb_txt_elements(path):
    elements = []
    cur = None
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.rstrip("\r\n")
            if line.startswith("element["):
                if cur and "SemanticName" in cur:
                    elements.append(cur)
                cur = {}
            elif cur is not None:
                t = line.strip()
                tl = t.lower()
                if tl.startswith("semanticname:"):
                    cur["SemanticName"] = t.split(":", 1)[1].strip()
                elif tl.startswith("semanticindex:"):
                    cur["SemanticIndex"] = int(t.split(":", 1)[1].strip())
                elif tl.startswith("format:"):
                    cur["Format"] = t.split(":", 1)[1].strip()
                elif not tl.startswith(("semantic", "format", "inputslot", "aligned", "instancedata", "inputslo")):
                    if "SemanticName" in cur:
                        elements.append(cur)
                    cur = None
    if cur and "SemanticName" in cur:
        elements.append(cur)
    elem_dict = {}
    for e in elements:
        idx = e.get("SemanticIndex", 0)
        en = e["SemanticName"] if idx == 0 else f"{e['SemanticName']}{idx}"
        elem_dict[en] = bw_from_format(e["Format"])
    show = []
    cnt = 0
    meet = False
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            if cnt > 10:
                break
            if not meet:
                if line.strip().startswith("vertex-data:"):
                    meet = True
                continue
            t = line.strip()
            if not t.lower().startswith("vb"):
                continue
            cnt += 1
            left = t.split(":", 1)[0]
            toks = left.split()
            if len(toks) >= 2:
                show.append(toks[1].strip())
    return elem_dict, show


def match_type(gt, files, root, deduped_map, lines, draw_ib, pointlist_index, tl_list):
    tl_index = None
    for idx in tl_list:
        ok = True
        for cat, topo in gt["cat_topo"].items():
            if topo != "trianglelist":
                continue
            if not first_file(files, f"{idx}-{gt['cat_slot'][cat]}", ".buf"):
                ok = False
                break
        if ok:
            tl_index = idx
            break
    if tl_index is None:
        return False
    cat_sizes = {}
    for cat in gt["categories"]:
        topo = gt["cat_topo"][cat]
        ex = pointlist_index if (topo == "pointlist" and pointlist_index) else tl_index
        slot = gt["cat_slot"][cat]
        buf_name = first_file(files, f"{ex}-{slot}", ".buf")
        txt_name = first_file(files, f"{ex}-{slot}", ".txt")
        buf_path = deduped_path(root, deduped_map, buf_name)
        txt_path = deduped_path(root, deduped_map, txt_name)
        if not buf_path or not os.path.exists(buf_path):
            return False
        if txt_name and not txt_path:
            return False
        if not txt_name or not txt_path:
            fs = os.path.getsize(buf_path)
        else:
            if gt["gpu"]:
                fs = os.path.getsize(buf_path)
            else:
                md = {}
                for line in open(txt_path, "r", encoding="utf-8", errors="replace"):
                    if line.startswith("stride:"):
                        md["stride"] = int(line[7:].strip())
                    elif line.startswith("vertex count:"):
                        md["vc"] = int(line[13:].strip())
                fs = md.get("stride", 0) * md.get("vc", 0)
        cat_sizes[cat] = fs
    vn = 0
    for cat in gt["categories"]:
        cs = gt["cat_stride"][cat]
        fs = cat_sizes[cat]
        tmp = fs // cs if cs > 0 else 0
        if tmp == 0:
            return False
        if not gt["gpu"] and fs % cs != 0:
            return False
        if vn == 0:
            vn = tmp
        elif vn != tmp:
            return False
        else:
            if gt["gpu"] and cat == "Texcoord" and pointlist_index:
                slot = gt["cat_slot"][cat]
                txt_name = first_file(files, f"{pointlist_index}-{slot}", ".txt")
                if not txt_name:
                    return False
                txt_path = deduped_path(root, deduped_map, txt_name)
                if not txt_path or not os.path.exists(txt_path):
                    return False
                elem_dict, show = vb_txt_elements(txt_path)
                all_bw = True
                for en, el in gt["elem_dict"].items():
                    if el["category"] != "Texcoord":
                        continue
                    if en in elem_dict:
                        if el["byte_width"] != elem_dict[en]:
                            all_bw = False
                            break
                if not all_bw:
                    txt_len = sum(elem_dict.get(en, 0) for en in show)
                    txt_num = len(show)
                    gt_len = sum(el["byte_width"] for el in gt["elem_dict"].values() if el["category"] == "Texcoord")
                    gt_num = sum(1 for el in gt["elem_dict"].values() if el["category"] == "Texcoord")
                    if txt_len != gt_len or txt_num != gt_num:
                        return False
    if not gt["gpu"] and len(gt["cat_slot"]) == 1:
        slot = gt["cat_slot"][gt["categories"][0]]
        txt_name = first_file(files, f"{tl_index}-{slot}", ".txt")
        if not txt_name:
            return False
        txt_path = deduped_path(root, deduped_map, txt_name)
        if not txt_path or not os.path.exists(txt_path):
            return False
        ss = ""
        for line in open(txt_path, "r", encoding="utf-8", errors="replace"):
            t = line.strip()
            if ":" in t and t.split(":", 1)[0].strip().lower() == "stride":
                ss = t.split(":", 1)[1].strip()
                break
        if ss.strip() and int(ss) != gt["self_stride"]:
            return False
    return True


def get_possible(types, files, root, deduped_map, lines, draw_ib):
    pl = get_last_pointlist_index_by_hash(lines, draw_ib)
    tl = get_trianglelist_index_list(files, draw_ib)
    possible = []
    found = False
    for gt in types:
        if found and not gt["gpu"]:
            continue
        if match_type(gt, files, root, deduped_map, lines, draw_ib, pl, tl):
            possible.append(gt["name"])
            if gt["gpu"]:
                found = True
    all_cpu = not any(t["gpu"] for t in types if t["name"] in possible)
    if all_cpu and possible:
        mx = max(len(next(t for t in types if t["name"] == n)["cat_slot"]) for n in possible)
        possible = [n for n in possible if len(next(t for t in types if t["name"] == n)["cat_slot"]) == mx]
    return pl, tl, possible


# ---------------------------------------------------------------------------
# Fast layout-signature coverage check
# ---------------------------------------------------------------------------

def collect_vb_slots(files, index):
    slots = set()
    for f in files:
        m = _VB_SLOT_RE.match(f)
        if m and f.startswith(index + "-"):
            slots.add(int(m.group(1)))
    return sorted(slots)


def vb_txt_path(root, deduped_map, files, index, slot):
    name = first_file(files, f"{index}-vb{slot}", ".txt")
    if not name:
        return ""
    path = deduped_path(root, deduped_map, name)
    if path and os.path.isfile(path):
        return path
    fallback = os.path.join(root, name)
    return fallback if os.path.isfile(fallback) else path


def derive_draw_layout(root, deduped_map, files, index):
    slots = collect_vb_slots(files, index)
    if not slots:
        return None, None
    txt_path = ""
    for slot in slots:
        p = vb_txt_path(root, deduped_map, files, index, slot)
        if p and os.path.isfile(p):
            txt_path = p
            break
    if not txt_path:
        return None, None

    by_slot_full = common.group_vb_elements_by_slot(txt_path)
    by_slot = {}
    gpu = False
    for slot in slots:
        elements = by_slot_full.get(slot) or []
        if not elements:
            continue
        by_slot[slot] = elements
        for e in elements:
            if common.is_blend_element(e):
                gpu = True
    return gpu, by_slot


# Slot -> Category. Mirrors the GameType convention: vb0 = Position,
# vb1 = Texcoord, vb2 = Blend. Single-buffer CPU types keep TEXCOORD in vb0 and
# therefore in Position, matching the library JSON Category field.
_SLOT_CATEGORY_MAP = {0: "Position", 1: "Texcoord", 2: "Blend", 3: "Texcoord", 4: "Texcoord"}


def layout_signature(gpu, by_slot):
    sig = {}
    for slot, elements in by_slot.items():
        cat = _SLOT_CATEGORY_MAP.get(slot, "Texcoord")
        for e in elements:
            en = common._element_key(e["SemanticName"], int(e.get("SemanticIndex") or 0))
            sig.setdefault(cat, []).append((en, int(e["ByteWidth"])))
    for cat in sig:
        sig[cat].sort()
    return (gpu, tuple(sorted((c, tuple(v)) for c, v in sig.items())))


def type_signature(gt):
    sig = {}
    for en, e in gt["elem_dict"].items():
        sig.setdefault(e["category"], []).append((en, e["byte_width"]))
    for cat in sig:
        sig[cat].sort()
    return (gt["gpu"], tuple(sorted((c, tuple(v)) for c, v in sig.items())))


def build_file_index(files):
    """Build O(1) lookup tables from a normalized FrameAnalysis file list."""
    vb_by_idx = {}
    ib_to_idx = {}
    for f in files:
        if not (f.endswith(".txt") or f.endswith(".buf")):
            continue
        idx = f[:6]
        m = _VB_SLOT_RE.match(f)
        if m:
            vb_by_idx.setdefault(idx, set()).add(int(m.group(1)))
        mm = re.search(r"-ib=([0-9a-fA-F]{8})", f)
        if mm:
            ib_to_idx.setdefault(mm.group(1).lower(), set()).add(idx)
    return vb_by_idx, ib_to_idx


def fast_check_coverage(types, files, root, deduped_map, file_index=None, ib_filter=None):
    sig_to_names = {}
    for gt in types:
        sig_to_names.setdefault(type_signature(gt), []).append(gt["name"])

    if file_index is None:
        file_index = build_file_index(files)
    vb_by_idx, ib_to_idx = file_index

    if ib_filter is not None:
        ib_iter = ib_filter
    else:
        ib_iter = sorted(ib_to_idx)

    coverage = {}
    details = {}
    for ib in ib_iter:
        idxs = sorted(ib_to_idx.get(ib, ()))
        best = None
        best_n = -1
        for ix in idxs:
            n = len(vb_by_idx.get(ix, ()))
            if n > best_n:
                best_n = n
                best = ix
        if best is None:
            coverage[ib] = []
            details[ib] = {"index": None, "gpu": None, "by_slot": {}}
            continue
        gpu, by_slot = derive_draw_layout(root, deduped_map, files, best)
        if not by_slot:
            coverage[ib] = []
            details[ib] = {"index": best, "gpu": gpu, "by_slot": {}}
            continue
        sig = layout_signature(gpu, by_slot)
        matched = sig_to_names.get(sig, [])
        coverage[ib] = matched
        details[ib] = {"index": best, "gpu": gpu, "by_slot": by_slot}
    return coverage, details


def check_coverage(types, files, root, deduped_map, lines, file_index=None, ib_filter=None):
    """Correct coverage check using the extractor's Unity VS matching algorithm.

    For each IB this resolves the pre-skinning pointlist draw (via the FrameAnalysis
    log) and matches it against the GameType registry. GPU-skinned meshes have their
    BLEND data in the pointlist draw, so deriving from the trianglelist draw alone
    produces a wrong (post-skinning) signature; the pointlist path avoids that.

    ``coverage[ib]`` is the list of matching type names (empty when missing) and
    ``details[ib]`` carries enough info to derive a new type for missing layouts.
    """
    if file_index is None:
        file_index = build_file_index(files)
    _, ib_to_idx = file_index

    if ib_filter is not None:
        ib_iter = ib_filter
    else:
        ib_iter = sorted(ib_to_idx)

    coverage = {}
    details = {}
    for ib in ib_iter:
        pl, tl, possible = get_possible(types, files, root, deduped_map, lines, ib)
        coverage[ib] = possible
        if possible:
            gpu = next((t["gpu"] for t in types if t["name"] in possible), False)
            details[ib] = {
                "index": pl or (tl[0] if tl else None),
                "gpu": gpu,
                "by_slot": {},
                "possible": possible,
            }
        else:
            idx = pl or (tl[0] if tl else None)
            gpu = None
            by_slot = {}
            if idx:
                gpu, by_slot = derive_draw_layout(root, deduped_map, files, idx)
            details[ib] = {
                "index": idx,
                "gpu": bool(gpu) if gpu is not None else False,
                "by_slot": by_slot or {},
                "possible": [],
            }
    return coverage, details


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=None)
    parser.add_argument("--gametype-dir", default=None, help="GameType folder override (auto-discovered from the current game when omitted)")
    parser.add_argument("--settings-file", default=None, help="Absolute path to SSMT4 settings.json (prefer MCP GlobalConfig.AppSettingsFilePath)")
    parser.add_argument("--gametype-root", default=None, help="GameType library directory (prefer MCP GlobalConfig.SSMTResourcesFolder()/GameType)")
    parser.add_argument("--drawib-file", default=None, help="Explicit DrawIB list JSON (workspace Config.json format, or a plain array of hashes)")
    parser.add_argument("--scan-all", action="store_true", help="Audit every IB in the dump instead of the workspace DrawIB list")
    parser.add_argument("--verbose", action="store_true", help="print every IB (default prints only a summary and missing signatures)")
    parser.add_argument("--json", default=None)
    args = parser.parse_args()

    root = common.discover_frame_analysis_root(args.root, settings_file=args.settings_file)
    game_type = common.discover_game_type(settings_file=args.settings_file, gametype_root=args.gametype_root)
    gametype_dir = common.resolve_gametype_dir(args.gametype_dir or game_type, gametype_root=args.gametype_root)
    if not gametype_dir:
        print(
            f"[check] GameType folder not found for game type '{game_type or args.gametype_dir}'. "
            "Pass --gametype-dir explicitly.",
            file=sys.stderr,
        )
        sys.exit(2)

    files = common.normalize_filenames(root)
    log_path = os.path.join(root, "log.txt")
    lines = []
    if os.path.isfile(log_path):
        lines = open(log_path, "r", encoding="utf-8", errors="replace").read().splitlines()
    deduped_map = build_deduped_map(lines)
    types = load_types(gametype_dir)
    print(f"[check] Game type: {game_type}; GameType folder: {gametype_dir}")

    drawib_list = []
    if args.drawib_file:
        drawib_list = common.read_drawib_list_file(args.drawib_file)
    elif not args.scan_all:
        settings = common.read_settings(args.settings_file)
        drawib_list = common.read_drawib_list(common.resolve_workspace_dir(settings))

    if drawib_list:
        print(f"[check] IB list source: workspace DrawIB list ({len(drawib_list)} IBs)")
    else:
        print("[check] IB list source: full dump scan (no workspace DrawIB list found)")

    file_index = build_file_index(files)
    coverage, details = check_coverage(
        types, files, root, deduped_map, lines, file_index, ib_filter=drawib_list or None
    )
    results = []
    covered = 0
    missing = 0
    missing_by_sig = collections.OrderedDict()
    for ib in sorted(coverage):
        matched = coverage[ib]
        detail = details[ib]
        gpu_label = "GPU" if detail["gpu"] else "CPU"
        name = common.encode_layout_name(bool(detail["gpu"]), detail["by_slot"]) if detail["by_slot"] else ""
        if matched:
            covered += 1
        else:
            missing += 1
            key = name or "(no layout)"
            missing_by_sig[key] = missing_by_sig.get(key, 0) + 1
        if args.verbose:
            print(
                f"IB {ib}: index={detail['index']} gpu={gpu_label} "
                f"matched={matched if matched else 'NONE'}"
                + (f" signature={name}" if name else "")
            )
        results.append(
            {
                "ib": ib,
                "index": detail["index"],
                "gpu": bool(detail["gpu"]),
                "signature": name,
                "matched": matched,
            }
        )

    print(f"[check] total={len(coverage)} covered={covered} missing={missing}")
    if missing_by_sig:
        print("[check] 缺失的 IB 布局签名：")
        for sig, n in sorted(missing_by_sig.items(), key=lambda kv: -kv[1]):
            print(f"[check]   {n:3d} IB  {sig}")
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(results, f, ensure_ascii=False, indent=2)


if __name__ == "__main__":
    main()
