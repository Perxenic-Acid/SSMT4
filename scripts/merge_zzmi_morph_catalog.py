"""Append ZZMI morph ranges observed in later FrameAnalysis captures.

The input workspace remains the neutral basis. Catalog captures supply
dispatch ranges only; their frame-specific weights never enter the workspace.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil

import numpy as np


BUFFERS = {
    "ShapeKeyOffset": "offsets_hash",
    "ShapeKeyVertexId": "vertex_ids_hash",
    "ShapeKeyVertexOffset": "vertex_offsets_hash",
    "ShapeKeyVertexAttributeOffset": "vertex_attribute_offsets_hash",
}


def read_capture(capture):
    catalog, = capture.glob("*-resourcemorphcatalog.buf")
    source, = capture.glob("*-resourcemorphcatalogsource=*.buf")
    match = re.search(r"=([0-9a-f]{8})\.buf$", source.name, re.IGNORECASE)
    if not match:
        raise ValueError(f"Missing source hash in {source.name}")
    records = np.fromfile(source, dtype="<u4").reshape(-1, 10)
    observed = np.fromfile(catalog, dtype="<u4").reshape(-1, 4)
    ranges = set()
    for index, (offset, count, _, marker) in enumerate(observed.tolist()):
        if marker == 1:
            if offset != index or not count or offset + count > len(records):
                raise ValueError(f"Invalid morph range in {catalog}: {offset}, {count}")
            ranges.add((offset, count))
    if not ranges:
        raise ValueError(f"No observed native morph dispatches in {catalog}")
    return match[1].lower(), records, ranges


def merge_captures(captures):
    sources = {}
    for capture in captures:
        source_hash, records, ranges = read_capture(capture)
        if source_hash in sources:
            original, collected = sources[source_hash]
            if not np.array_equal(records, original):
                raise ValueError(f"Different source data for {source_hash}")
            collected.update(ranges)
        else:
            sources[source_hash] = (records, ranges)
    return sources


def append_workspace_keys(metadata, sources):
    info = metadata.get("ShapeKeysInfo") or {}
    keys = info.get("native_morph_keys") or []
    source_hashes = {key["source_hash"].lower() for key in keys}
    if not source_hashes.intersection(sources):
        return {}
    files = {entry["Type"]: metadata["_folder"] / entry["FileName"]
             for entry in metadata["CategoryBufferList"] if entry["Type"] in BUFFERS}
    offsets = np.fromfile(files["ShapeKeyOffset"], dtype="<u4")
    ids = np.fromfile(files["ShapeKeyVertexId"], dtype="<u4")
    positions = np.fromfile(files["ShapeKeyVertexOffset"], dtype="<f2").reshape(-1, 3)
    deltas = np.fromfile(files["ShapeKeyVertexAttributeOffset"], dtype="<f4").reshape(-1, 9)
    if len(ids) != len(positions) or len(ids) != len(deltas) or int(offsets[-1]) != len(ids):
        raise ValueError(f"Inconsistent existing shape key buffers in {metadata['_folder']}")

    known = set()
    for key in keys:
        source_hash = key["source_hash"].lower()
        offset, count = int(key["offset"]), int(key["count"])
        known.add((source_hash, offset, count))
        if source_hash not in sources:
            continue
        records, _ = sources[source_hash]
        idx = int(key["shape_key_index"])
        start, end = offsets[idx:idx + 2]
        raw = records[offset:offset + count]
        if not (
            np.array_equal(ids[start:end], raw[:, 0])
            and np.array_equal(deltas[start:end].view("<u4"), raw[:, 1:])
        ):
            raise ValueError(f"Captured source differs from existing Deform {idx} in {metadata['_folder']}")

    added_ids = [ids]
    added_deltas = [deltas]
    added_positions = [positions]
    new_keys = list(keys)
    total = len(ids)
    vertex_count = int(metadata["OriginalVertexCount"])
    next_index = max(int(key["shape_key_index"]) for key in keys) + 1
    for source_hash in sorted(source_hashes.intersection(sources)):
        records, ranges = sources[source_hash]
        format_keys = [key for key in keys if key["source_hash"].lower() == source_hash]
        template = format_keys[0]
        if len({(key["shader_hash"], key["constant_buffer_hash"]) for key in format_keys}) != 1:
            raise ValueError(f"Multiple dispatch layouts for source {source_hash}")
        for offset, count in sorted(ranges):
            if (source_hash, offset, count) in known:
                continue
            if next_index + 1 >= len(offsets):
                raise ValueError("The workspace shape-key offset table is full")
            raw = records[offset:offset + count]
            delta = raw[:, 1:].copy().view("<f4")
            if (
                raw[:, 0].max() >= vertex_count
                or not np.isfinite(delta).all()
                or abs(delta[:, :3]).max() > np.finfo(np.float16).max
            ):
                raise ValueError(f"Invalid vertex data for {source_hash} range {offset}:{count}")
            added_ids.append(raw[:, 0])
            added_deltas.append(delta)
            added_positions.append(delta[:, :3].astype("<f2"))
            total += count
            offsets[next_index + 1:] = total
            new_keys.append(dict(template, offset=offset, count=count, shape_key_index=next_index))
            next_index += 1

    if len(new_keys) == len(keys):
        return {}
    info.update(
        native_morph_keys=new_keys,
        vertex_count=total,
        dispatch_y=(total + 31) // 32,
        checksum=sum(int(value) for value in offsets[:4]) & 0xFFFFFFFF,
    )
    buffers = dict(zip(BUFFERS, (offsets, np.concatenate(added_ids),
                                np.concatenate(added_positions), np.concatenate(added_deltas))))
    changed = {}
    for kind, arr in buffers.items():
        data = arr.tobytes()
        info[BUFFERS[kind]] = hashlib.sha256(data).hexdigest()[:8]
        changed[files[kind]] = data
    return changed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--capture", required=True, type=Path, action="append")
    args = parser.parse_args()
    workspace, output = args.workspace.resolve(), args.output.resolve()
    if output.exists() or output.is_relative_to(workspace):
        parser.error("Output must be a new directory outside the input workspace")
    sources = merge_captures(args.capture)
    changed = {}
    for path in workspace.rglob("TYPE_*/*.json"):
        metadata = json.loads(path.read_text(encoding="utf-8-sig"))
        metadata["_folder"] = path.parent
        buffers = append_workspace_keys(metadata, sources)
        del metadata["_folder"]
        if buffers:
            changed.update(buffers)
            changed[path] = (json.dumps(metadata, ensure_ascii=False, indent=4) + "\n").encode("utf-8")
            keys = metadata["ShapeKeysInfo"]["native_morph_keys"]
            print(f"{path.stem}: {len(keys)} native shape keys")
    if not changed:
        parser.error("No matching, previously unknown dispatch ranges")
    shutil.copytree(workspace, output)
    for path, data in changed.items():
        (output / path.relative_to(workspace)).write_bytes(data)


if __name__ == "__main__":
    main()

