# GameType Project Conventions

## Repository layout

- Single source of truth: src-tauri/resources/GameType/*.json (one file per data type; each game type has its own folder, e.g. ZZMI/, GF2/, APMI/). The old Rust registry type_zzmi.rs was removed.
- Runtime data types: the GameType library shipped with the install (<install dir>/resources/GameType/<GameType>/). The runtime reads only this folder; nothing is copied to user config.
- Constants: src-tauri/src/constants/gametype_{format,element_name,extract_slot,extract_technique,category_name}.rs.
- Extractor matching logic: src-tauri/src/extract_new/*.rs (get_possible_gametype_list_unity_vs, filter_trianglelist_index_unity_vs).
- Frame analysis plumbing: src-tauri/src/common/frame_analysis/frameanalysis_log.rs, frameanalysis_data.rs.

## JSON data type format

Each file contains exactly one data type:

    {
      "D3D11ElementList": [
        {
          "SemanticName": "POSITION",
          "Format": "R32G32B32_FLOAT",
          "ExtractSlot": "vb0",
          "ExtractTechnique": "pointlist",
          "Category": "Position",
          "DrawCategory": "Position",
          "ByteWidth": "12"
        }
      ]
    }

Fields: SemanticName, Format, ExtractSlot, ExtractTechnique, Category, DrawCategory, ByteWidth (optional; computed from Format when missing).

## Type-name encoding

Format: CPU|GPU prefix followed by segments P.., N.., TA.., C.., T.., T1-.., T2-.., optional BW.., optional BI.., ending with an underscore.

Segments seen in practice:

- P12 = POSITION R32G32B32_FLOAT (12 B); P16 = R32G32B32A32_FLOAT (16 B); P8 = R16G16B16A16_FLOAT (8 B).
- N12 = NORMAL 12 B; N4 = NORMAL R8G8B8A8_SNORM (4 B).
- TA16 = TANGENT R32G32B32A32_FLOAT (16 B); TA4 = TANGENT R8G8B8A8_SNORM (4 B).
- C4 = COLOR R8G8B8A8_UNORM (4 B); C16 = COLOR R32G32B32A32_FLOAT (16 B).
- T4/T8 = the TEXCOORD slot pattern (4 B R16G16_FLOAT / 8 B R32G32_FLOAT per channel block).
- T1-8 = TEXCOORD1 R32G32_FLOAT (8 B); T2-4 = TEXCOORD2 R16G16_FLOAT (4 B); T3-4, T4-4, T5-4 similarly.
- BW16 = BLENDWEIGHTS R32G32B32A32_FLOAT (16 B); BW8 = R32G32_FLOAT (8 B).
- BI16 = BLENDINDICES R32G32B32A32_UINT (16 B); BI8 = R32G32_UINT (8 B); BI4 = R32_UINT (4 B).

Examples: CPU_P12_N12_TA16_C4_T4_T1-8_T2-4_, GPU_P12_N12_TA16_C16_T8_T1-8_T2-8_T3-8_BW16_BI16_, GPU_P12_N12_TA16_C4_T4_T1-8_T2-4_T3-4_T4-4_BW16_BI16_.

## Category / slot conventions

- Position: vb0 - POSITION, NORMAL, TANGENT.
- Texcoord: vb1 (or vb0 in some CPU types) - COLOR, TEXCOORD, TEXCOORD1..7.
- Blend: vb2 - BLENDWEIGHTS, BLENDINDICES.

GPU-pre-skinning types use pointlist ExtractTechnique; CPU types use trianglelist. gpu_pre_skinning is true when the type contains BLENDINDICES or any pointlist element.

## Matching algorithm (as implemented in the checker script)

The default coverage check is a fast layout-signature comparison, not the full file-size walk. For each IB it derives the per-buffer layout from the VB header (grouping by `InputSlot`, deduplicating overlapping `AlignedByteOffset` entries) and compares the signature against the registry. Covered IBs are returned immediately. The full algorithm below is kept as a deep-verify fallback.


1. For each candidate trianglelist index (from IB txt files sharing the draw IB hash), require every category whose topology is trianglelist to have a <index>-<slot>.buf frame file. Pick the first index that satisfies all.
2. For each category, pick the extract index: pointlist index when the category topology is pointlist and a pointlist index was found from the log; otherwise the trianglelist index.
3. Resolve <extract_index>-<slot>.buf and .txt to real paths through the log.txt Dumping Buffer map; fail if the buf path is missing or the txt exists without a deduped path.
4. File size per category: GPU type -> buf size; CPU type -> txt stride x vertex count.
5. vertex_number = size / category_stride (sum of element ByteWidth in the category). Must be > 0, identical across categories, and for CPU types the size must divide evenly.
6. GPU Texcoord extra check (when pointlist index exists): each Texcoord element ByteWidth must equal the corresponding element in the pointlist txt; fallback compares total Texcoord byte length and element count.
7. Single-category CPU types: txt stride must equal the type's total stride.
8. Post-filter: if all matches are CPU types, keep only those with the maximum category count. Once a GPU type matches, later CPU types are skipped.

## Pointlist index derivation (log.txt)

- Take the first trianglelist draw index for the IB hash (get_drawcall_index_list_by_hash).
- Within that draw's lines, find the first IASetVertexBuffers call, then read the "0: resource=... hash=<vb0hash>" line for slot 0.
- Scan all log lines; for lines containing hash=<vb0hash> (excluding Dst lines), record the current draw index when it is less than the first trianglelist index. Return the last such index.

## Adding a data type (checklist)

1. Identify unsupported IB hashes with `scripts/gametype_ib_check.py` (matched=[]).
2. Let `scripts/gametype_add_missing_types.py` derive and generate the missing files (dry-run first, review, then `--write`).
3. Re-run the checker: the new hash must match, and no previously matched hash may lose its match.
4. The `--write` run also creates a `gametype-additions-<GameType>-<timestamp>.zip` package. Share it with developers so they can add the new files to the library directly.
5. The new files are written straight into the installed library (`<resources folder>/GameType/<game>`), so they take effect on the next extraction — no restart needed. In a dev checkout, run `bun tauri dev` once so the file is picked up from `src-tauri/resources/GameType`.

For manual review/fallback when the automatic derivation is not applicable:

- Open the paired VB txt headers for a representative trianglelist draw (and pointlist draw if present): record SemanticName/SemanticIndex/Format/InputSlot/AlignedByteOffset/stride per slot, and the actual buffer bytes.
- Build the element list (order matters: POSITION, NORMAL, TANGENT, COLOR, TEXCOORD0..N, BLENDWEIGHTS, BLENDINDICES) with ExtractSlot/ExtractTechnique/Category/DrawCategory/ByteWidth; infer CPU vs GPU from topology and blend presence.
- Choose a type name following the encoding table.
- Write `src-tauri/resources/GameType/<GameType>/<TYPE_NAME>.json` with the D3D11ElementList. A same-name variant uses a `_2` file suffix (the file stem becomes the distinct game type name).
