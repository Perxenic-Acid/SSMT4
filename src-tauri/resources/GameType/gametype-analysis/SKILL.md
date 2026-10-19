---
name: gametype-analysis
description: Check whether every IB in the current workspace's DrawIB list can be parsed by the existing GameType JSON data types, report covered IBs immediately, and only for missing layouts derive + generate the corresponding GameType JSON files (deduplicated per layout), then package newly added files into a shareable zip. Use when asked to verify IB parseability, find unsupported IB layouts, add GameType JSON files, audit GameType coverage after a new frame capture, or prepare a shareable GameType data-type package.
---

# GameType Analysis

Determine whether every IB in the current workspace's DrawIB list is covered by the
existing GameType data-type library:

- Covered IBs are reported as "already have a data type" and are not re-derived.
- Unrecognizable IBs (no pointlist/VB layout) are marked unable and skipped.
- Only genuinely missing layouts are derived (deduplicated per layout), written as
  GameType JSON files into the installed data-type library
  (`<resources folder>/GameType/<game>`), and packaged into a shareable zip.

The DrawIB list lives in the current workspace's `LOD*/Config.json` (each entry is
`{"DrawIB": "<8-hex hash>", "Alias": ""}`). The whole FrameAnalysis dump also contains
many unrelated IBs, so auditing only the DrawIB list is both correct and fast.

## Golden rule: use MCP first, never hunt through the filesystem

Every path this skill needs should come from MCP. Do not guess paths, do not grep, do
not enumerate directories manually.

Auto-registered module functions use dotted names (`Module.function`). If you need to
discover them, call `list_capabilities` and then `get_tool_schema` before invoking.

| Data | MCP tool |
| --- | --- |
| settings.json absolute path | `GlobalConfig.AppSettingsFilePath()` |
| SSMT4 resources folder | `GlobalConfig.SSMTResourcesFolder()` |
| GameType library folder | `<resources folder>/GameType` |
| (optional) cache folder / workspace | `GlobalConfig.ReadConfig()` fields `DBMTWorkFolder` / `CurrentWorkSpace` |

The scripts read `settings.json` and the workspace `LOD*/Config.json` directly, so
passing the two paths below is enough for them to locate the dump, the game type and
the DrawIB list automatically.

## Workflow

1. Call MCP `GlobalConfig.AppSettingsFilePath()` and save the returned path as `<settings>`.
2. Call MCP `GlobalConfig.SSMTResourcesFolder()` and derive `<gametype-root>` = `<that folder>/GameType`,
   the data-type library shipped with the install (the runtime reads exactly this folder).
3. From this skill's directory, run a preview (writes nothing):

   ```
   python -u scripts/gametype_add_missing_types.py --settings-file "<settings>" --gametype-root "<gametype-root>" --dry-run
   ```

   Read only the script's summary line: `checked / covered / unable / new`.
   - If `new` is 0, tell the user all IBs in the current DrawIB list are already covered
     by existing data types; do nothing more.
4. Only if `new > 0`, run the real pass (drop `--dry-run`):

   ```
   python -u scripts/gametype_add_missing_types.py --settings-file "<settings>" --gametype-root "<gametype-root>"
   ```

5. Report the zip path printed by the script, and explain it can be shared with developers
   to merge into the GameType library.

## Coverage logic (why it is correct)

GPU-skinned meshes store BLEND data in the pre-skinning pointlist draw, while the DrawIB
hash references the post-skinning trianglelist draw. The scripts resolve the pointlist
draw through the FrameAnalysis log and match against the library using the same Unity VS
algorithm as the extractor, so existing GPU types are recognized instead of being re-derived
as new CPU types.

## Constraints

- Never manually read log.txt, never manually list the dump directory, never re-verify
  already-covered IBs.
- Coverage, missing layouts and new types are computed by the scripts in one pass; trust the
  script summary.
- Always run `--dry-run` first; only the "new" pass writes files / creates a zip.
- If MCP is unavailable (for example, running the scripts manually), omit `--settings-file`
  / `--gametype-root` and the scripts fall back to filesystem auto-discovery.

## Scripts

- `scripts/gametype_add_missing_types.py` - one-command preview / generate / package (`--dry-run` previews only).
- `scripts/gametype_ib_check.py` - coverage check + summary only, no writes.
- Common args: `[dump_dir] --settings-file <path> --gametype-root <path> [--drawib-file <path>] [--scan-all] [--verbose] [--json <out>]`.

See `references/PROJECT-CONVENTIONS.md` for GameType JSON conventions.
