
#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Shared helpers for the GameType analysis scripts.

Contains:
  - SSMT4 workspace discovery: resolve the current workspace's FrameAnalysis
    dump directory from %LOCALAPPDATA%/SSMT4GlobalConfigs/settings.json and the
    workspace's Config/FrameAnalysisPath.json.
  - Migoto VB .txt header parsing (including InputSlot / AlignedByteOffset).
  - GameType name encoding and JSON building.
  - Layout signature extraction for fast coverage checks.
"""

from __future__ import annotations

import json
import os
import re
import sys


def _force_utf8_stdio():
    """Make Chinese console output robust regardless of the Windows code page."""
    for stream in (sys.stdout, sys.stderr):
        try:
            if hasattr(stream, "reconfigure"):
                stream.reconfigure(encoding="utf-8")
        except Exception:
            pass


_force_utf8_stdio()


SLOTS = ["vb0", "vb1", "vb2", "vb3", "vb4"]

# ExtractSlot -> (Category, DrawCategory). Matches the standard Unity VS GameType conventions:
# vb0 = Position (POSITION/NORMAL/TANGENT, sometimes TEXCOORD in single-buffer CPU types),
# vb1 = Texcoord (COLOR/TEXCOORD*),
# vb2 = Blend (BLENDWEIGHTS/BLENDINDICES).
SLOT_CATEGORY = {
    "vb0": ("Position", "Position"),
    "vb1": ("Texcoord", "Texcoord"),
    "vb2": ("Blend", "Position"),
}


def _local_appdata() -> str:
    return os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")


def ssmt_global_configs_folder() -> str:
    return os.path.join(_local_appdata(), "SSMT4GlobalConfigs")


def settings_path() -> str:
    return os.path.join(ssmt_global_configs_folder(), "settings.json")


def read_json(path: str):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def settings_dir(settings_file=None) -> str:
    """Directory that contains settings.json (usually SSMT4GlobalConfigs)."""
    if settings_file:
        return os.path.dirname(os.path.abspath(os.path.expanduser(settings_file)))
    return ssmt_global_configs_folder()


def global_games_folder(settings_file=None) -> str:
    return os.path.join(settings_dir(settings_file), "Games")


def game_config_path(game_name: str, settings_file=None) -> str:
    return os.path.join(global_games_folder(settings_file), game_name, "Config.json")


def read_settings(settings_file=None) -> dict:
    sp = settings_file or settings_path()
    if not os.path.isfile(sp):
        return {}
    try:
        return read_json(sp)
    except Exception:
        return {}


def bundled_gametype_root() -> str:
    """Return the GameType library folder this skill is shipped inside."""
    cur = os.path.abspath(os.path.dirname(__file__))
    while True:
        if os.path.basename(cur).lower() == "gametype":
            return cur
        parent = os.path.dirname(cur)
        if parent == cur:
            break
        cur = parent
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def gametype_roots(gametype_root=None) -> list:
    """Candidate GameType library roots.

    ``gametype_root`` (when provided, typically via MCP) is the authoritative
    GameType directory and is checked first. Otherwise fall back to the dev repo
    and to the GameType folder this skill itself is shipped inside (i.e. the
    installed ``resources/GameType``). The legacy user-config copy is deliberately
    not consulted: the runtime no longer reads it.
    """
    roots = []
    if gametype_root:
        r = os.path.abspath(os.path.expanduser(gametype_root))
        if os.path.isdir(r):
            roots.append(r)
    repo = os.path.join(os.getcwd(), "src-tauri", "resources", "GameType")
    if os.path.isdir(repo):
        roots.append(repo)
    skill = bundled_gametype_root()
    if skill and skill not in roots and os.path.isdir(skill):
        roots.append(skill)
    return roots


def list_gametype_folders(root=None, gametype_root=None) -> list:
    roots = [root] if root else gametype_roots(gametype_root)
    names = set()
    for r in roots:
        if not os.path.isdir(r):
            continue
        for name in os.listdir(r):
            if os.path.isdir(os.path.join(r, name)):
                names.add(name)
    return sorted(names)


def normalize_game_type_name(name, root=None, gametype_root=None) -> str:
    """Normalize a game type / GameType folder name, preserving on-disk casing."""
    n = (name or "").strip()
    if not n:
        return ""
    if n.lower() == "identityv":
        return "IdentityV"
    upper = n.upper()
    for folder in list_gametype_folders(root, gametype_root):
        if folder.upper() == upper:
            return folder
    return upper


def discover_game_type(explicit=None, settings_file=None, gametype_root=None) -> str:
    """Resolve the current game type (GameType folder name)."""
    if explicit:
        return normalize_game_type_name(explicit, gametype_root=gametype_root)
    settings = read_settings(settings_file)
    game_name = (settings.get("CurrentGameName") or "").strip()
    if not game_name:
        return ""
    preset = ""
    cfg = game_config_path(game_name, settings_file)
    if os.path.isfile(cfg):
        try:
            preset = (read_json(cfg).get("gamePreset") or "").strip()
        except Exception:
            preset = ""
    return normalize_game_type_name(preset or game_name, gametype_root=gametype_root)


def resolve_gametype_dir(game_type, root=None, gametype_root=None) -> str:
    """Resolve a GameType folder path for a game type name (or a direct path)."""
    if game_type:
        direct = os.path.abspath(os.path.expanduser(game_type))
        if os.path.isdir(direct):
            return direct
    gt = normalize_game_type_name(game_type, root, gametype_root) if game_type else ""
    if not gt:
        return ""
    roots = [root] if root else gametype_roots(gametype_root)
    for r in roots:
        path = os.path.join(r, gt)
        if os.path.isdir(path):
            return path
    return ""


def _first_existing(paths):
    for path in paths:
        if path and os.path.isfile(path):
            return path
    return None


def resolve_workspace_dir(settings: dict) -> str:
    """Resolve the current workspace directory, or empty string.

    Layout: ``<DBMTWorkFolder>/WorkSpace/<game>/<workspace>``.
    """
    dbmt = (settings.get("DBMTWorkFolder") or "").strip()
    if not dbmt:
        return ""

    game_name = (settings.get("CurrentGameName") or "").strip() or "Default"
    game_key = game_name if game_name != "Default" else "DefaultGame"

    by_game = settings.get("CurrentWorkSpaceByGame") or {}
    ws_name = (
        by_game.get(game_key) if isinstance(by_game, dict) else None
    ) or (settings.get("CurrentWorkSpace") or "").strip() or "Default"

    return os.path.join(dbmt, "WorkSpace", game_key, ws_name)


def resolve_workspace_frame_analysis(settings: dict) -> str:
    """Return the current workspace's FrameAnalysis folder path, or empty string."""
    workspace_dir = resolve_workspace_dir(settings)
    if not workspace_dir:
        return ""

    config_path = _first_existing(
        [
            os.path.join(workspace_dir, "Config", "FrameAnalysisPath.json"),
            os.path.join(workspace_dir, "FrameAnalysisPathConfig.json"),
        ]
    )
    if not config_path:
        return ""

    config = read_json(config_path)
    return (config.get("frameAnalysisFolderPath") or "").strip()


def read_drawib_list(workspace_dir: str) -> list:
    """Read the DrawIB list from the current workspace's LOD folders.

    Each ``LOD<n>/Config.json`` is an array of ``{"DrawIB": "<8-hex hash>",
    "Alias": "..."}`` entries. These are the IBs the user is actually working
    with; the full FrameAnalysis dump also contains many unrelated IBs that must
    not be audited when the workspace provides an explicit list.
    """
    if not workspace_dir or not os.path.isdir(workspace_dir):
        return []
    hashes = []
    seen = set()
    try:
        names = sorted(os.listdir(workspace_dir))
    except Exception:
        return []
    for name in names:
        if not name.upper().startswith("LOD"):
            continue
        lod_dir = os.path.join(workspace_dir, name)
        if not os.path.isdir(lod_dir):
            continue
        cfg = os.path.join(lod_dir, "Config.json")
        if not os.path.isfile(cfg):
            continue
        try:
            data = read_json(cfg)
        except Exception:
            continue
        if not isinstance(data, list):
            continue
        for item in data:
            if not isinstance(item, dict):
                continue
            h = (item.get("DrawIB") or item.get("drawIB") or "").strip().lower()
            if h and h not in seen:
                seen.add(h)
                hashes.append(h)
    return sorted(hashes)


def read_drawib_list_file(path: str) -> list:
    """Read a DrawIB list from an explicit JSON file.

    Accepts either the workspace ``Config.json`` format (an array of
    ``{"DrawIB": "...", ...}`` objects) or a plain array of hash strings.
    """
    if not path or not os.path.isfile(path):
        return []
    try:
        data = read_json(path)
    except Exception:
        return []
    hashes = []
    seen = set()

    def add(value):
        h = (value or "").strip().lower()
        if h and h not in seen:
            seen.add(h)
            hashes.append(h)

    if isinstance(data, list):
        for item in data:
            if isinstance(item, str):
                add(item)
            elif isinstance(item, dict):
                add(item.get("DrawIB") or item.get("drawIB") or item.get("Hash"))
    elif isinstance(data, dict):
        for key in data:
            add(key)
            value = data[key]
            if isinstance(value, str):
                add(value)
    return sorted(hashes)


def discover_frame_analysis_root(explicit=None, settings_file=None) -> str:
    """Resolve the FrameAnalysis dump directory.

    Prefer the explicit argument; otherwise read SSMT4 persisted settings (from
    ``settings_file`` when provided, typically via MCP) and the current workspace
    FrameAnalysis path. Exit with a helpful message if it cannot be determined.
    """
    if explicit:
        root = os.path.abspath(os.path.expanduser(explicit))
        if not os.path.isdir(root):
            print(f"[discover] FrameAnalysis folder not found: {root}", file=sys.stderr)
            sys.exit(2)
        return root

    sp = settings_file or settings_path()
    if not os.path.isfile(sp):
        print(
            f"[discover] SSMT4 settings.json not found at {sp}. "
            "Pass the FrameAnalysis folder explicitly.",
            file=sys.stderr,
        )
        sys.exit(2)

    root = resolve_workspace_frame_analysis(read_json(sp))
    if not root:
        print(
            "[discover] Could not resolve the current workspace FrameAnalysis path. "
            "Select a FrameAnalysis folder on the WorkPage, or pass it explicitly.",
            file=sys.stderr,
        )
        sys.exit(2)

    root = os.path.abspath(os.path.expanduser(root))
    if not os.path.isdir(root):
        print(f"[discover] Resolved FrameAnalysis folder does not exist: {root}", file=sys.stderr)
        sys.exit(2)
    return root


def normalize_filenames(root: str):
    """Return normalized top-level FrameAnalysis names (strip .lnk shortcuts).

    The original implementation stat'ed every entry, which is very slow on a
    large capture directory. Listing names and dropping the well-known deduped
    sub-directory avoids that while still covering every real dump file.
    """
    out = set()
    for name in os.listdir(root):
        if name == "deduped":
            continue
        if name.endswith(".lnk"):
            name = name[:-4]
        out.add(name)
    return sorted(out)


def bw_from_format(fmt: str) -> int:
    """Byte width from a DXGI-style format string (matches the Rust algorithm)."""
    f = fmt.lower()
    total = 0
    for i in range(len(f) - 2):
        w = f[i : i + 3]
        if w[1] == "8":
            total += 1
        elif w[1] == "3" and w[2] == "2":
            total += 4
        elif w[1] == "1" and w[2] == "6":
            total += 2
    return total


def _element_key(semantic_name: str, semantic_index: int) -> str:
    return semantic_name if semantic_index == 0 else f"{semantic_name}{semantic_index}"


def parse_vb_header(path: str):
    """Parse a 3Dmigoto VB .txt header.

    Returns (elements_by_key, vertex_data):
      - elements_by_key maps ``SEM`` / ``SEM<idx>`` to a dict with
        SemanticName, SemanticIndex, Format, ByteWidth, InputSlot and
        AlignedByteOffset.
      - vertex_data is an ordered list of (offset, token) entries from the
        ``vertex-data:`` block.
    """
    elements_by_key = {}
    cur = None

    def store():
        if cur and cur.get("SemanticName") and cur.get("Format"):
            fmt = cur["Format"]
            key = _element_key(cur["SemanticName"], int(cur.get("SemanticIndex") or 0))
            elements_by_key[key] = {
                "SemanticName": cur["SemanticName"],
                "SemanticIndex": int(cur.get("SemanticIndex") or 0),
                "Format": fmt,
                "ByteWidth": str(bw_from_format(fmt)),
                "InputSlot": cur.get("InputSlot"),
                "AlignedByteOffset": cur.get("AlignedByteOffset"),
            }

    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.rstrip("\r\n")
            stripped = line.strip()
            lower = stripped.lower()

            if stripped.startswith("element["):
                store()
                cur = {"SemanticName": "", "SemanticIndex": 0, "Format": ""}
                continue
            if cur is None:
                continue

            if lower.startswith("semanticname:"):
                cur["SemanticName"] = stripped.split(":", 1)[1].strip()
            elif lower.startswith("semanticindex:"):
                cur["SemanticIndex"] = int(stripped.split(":", 1)[1].strip() or 0)
            elif lower.startswith("format:"):
                cur["Format"] = stripped.split(":", 1)[1].strip()
            elif lower.startswith("inputslot:"):
                cur["InputSlot"] = int(stripped.split(":", 1)[1].strip() or 0)
            elif lower.startswith("alignedbyteoffset:"):
                cur["AlignedByteOffset"] = int(stripped.split(":", 1)[1].strip() or 0)
            elif stripped.startswith("vertex-data:"):
                store()
                cur = None
                break
    store()

    vertex_data = []
    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        meet = False
        for raw in handle:
            line = raw.rstrip("\r\n").strip()
            if not meet:
                if line.startswith("vertex-data:"):
                    meet = True
                continue
            if not line.lower().startswith("vb"):
                continue
            left = line.split(":", 1)[0]
            toks = left.split()
            if len(toks) < 2:
                continue
            offset = 0
            if "+" in toks[0]:
                try:
                    offset = int(toks[0].rsplit("+", 1)[1])
                except ValueError:
                    offset = 0
            vertex_data.append((offset, toks[1]))
    return elements_by_key, vertex_data


def group_vb_elements_by_slot(path: str) -> dict:
    """Group a VB header's elements by InputSlot, deduplicating overlapping offsets.

    3Dmigoto repeats the full input layout in every ``vbN`` header and often lists
    instanced/overlapping entries at offset 0. Grouping by InputSlot and keeping the
    first element per AlignedByteOffset recovers the real per-buffer layout.
    """
    elements_by_key, _ = parse_vb_header(path)
    by_slot = {}
    for element in elements_by_key.values():
        slot = element.get("InputSlot")
        if slot is None:
            continue
        by_slot.setdefault(slot, []).append(element)

    for slot in list(by_slot):
        seen = set()
        deduped = []
        for element in by_slot[slot]:
            off = element.get("AlignedByteOffset")
            if off in seen:
                continue
            seen.add(off)
            deduped.append(element)
        by_slot[slot] = deduped
    return by_slot


def is_blend_element(element: dict) -> bool:
    return element["SemanticName"].upper() in ("BLENDINDICES", "BLENDWEIGHTS", "BLENDWEIGHT")


def has_blend(elements) -> bool:
    return any(is_blend_element(element) for element in elements)


def _semantic_segment(semantic: str, index: int, byte_width: str) -> str:
    s = semantic.upper()
    bw = byte_width
    if s == "POSITION":
        return f"P{bw}"
    if s == "NORMAL":
        return f"N{bw}"
    if s == "TANGENT":
        return f"TA{bw}"
    if s == "BINORMAL":
        return f"BN{bw}"
    if s == "BITANGENT":
        return f"BT{bw}"
    if s == "COLOR":
        return f"C{bw}"
    if s == "TEXCOORD":
        return f"T{bw}" if index == 0 else f"T{index}-{bw}"
    if s in ("BLENDWEIGHTS", "BLENDWEIGHT"):
        return f"BW{bw}"
    if s == "BLENDINDICES":
        return f"BI{bw}"
    suffix = str(index) if index else ""
    return f"{s}{suffix}-{bw}"


def encode_type_name(elements, gpu: bool) -> str:
    """Encode an ordered element list into a GameType name."""
    prefix = "GPU" if gpu else "CPU"
    segments = [prefix]
    for element in elements:
        segments.append(
            _semantic_segment(
                element["SemanticName"],
                int(element.get("SemanticIndex") or 0),
                element["ByteWidth"],
            )
        )
    return "_".join(segments) + "_"


def encode_layout_name(gpu: bool, by_slot: dict) -> str:
    """Encode a per-slot layout (slot int -> element list) into a GameType name."""
    segments = ["GPU" if gpu else "CPU"]
    for slot in sorted(by_slot):
        for element in by_slot[slot]:
            segments.append(
                _semantic_segment(
                    element["SemanticName"],
                    int(element.get("SemanticIndex") or 0),
                    element["ByteWidth"],
                )
            )
    return "_".join(segments) + "_"


def build_game_type_json(elements_by_slot: dict, gpu: bool) -> list:
    """Build the D3D11ElementList for a GameType JSON.

    ``elements_by_slot`` may use either ``vb0``/``vb1``/... string keys or integer
    slot numbers.
    """
    technique = "pointlist" if gpu else "trianglelist"
    out: list = []
    for slot in SLOTS:
        elements = elements_by_slot.get(slot)
        if elements is None:
            elements = elements_by_slot.get(int(slot[2:]), [])
        if not elements:
            continue
        category, draw_category = SLOT_CATEGORY.get(slot, ("Texcoord", "Texcoord"))
        for element in elements:
            out.append(
                {
                    "SemanticName": element["SemanticName"],
                    "Format": element["Format"],
                    "ExtractSlot": slot,
                    "ExtractTechnique": technique,
                    "Category": category,
                    "DrawCategory": draw_category,
                    "ByteWidth": element["ByteWidth"],
                }
            )
    return out
