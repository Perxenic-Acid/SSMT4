# GameType

This folder is the **single source of truth** for D3D11GameType definitions.
The hardcoded Rust registries (`src-tauri/src/gametype/type_*.rs`) have been removed.

## Runtime location

The runtime reads data types straight from this folder, as shipped with the app:

    <install dir>\resources\GameType      (installed / portable build)
    src-tauri/resources/GameType          (dev build)

This is the **only** source: `gametype::get_game_type_list` resolves the game
folder here and nowhere else. Older builds copied the whole library into
`%LOCALAPPDATA%\SSMT4GlobalConfigs\GameType` and read only that copy, which is why
uninstalling left stale types behind and reinstalling could not clean them up.
The runtime no longer reads, writes or syncs anything under user config.

## Adding a type

1. Put one JSON file per type under `src-tauri/resources/GameType/<game>/`
   so the next build/install ships it.
2. Update the install (or run `bun tauri dev`) — it is picked up in place.

Types are program assets: an update replaces them wholesale, they live and die
with the install, and nothing is copied into user data.

Each subfolder is one game preset. Each .json file inside is one D3D11GameType.

File format:

{
  "D3D11ElementList": [
    {
      "SemanticName": "POSITION",
      "Format": "R32G32B32_FLOAT",
      "ExtractSlot": "vb0",
      "ExtractTechnique": "trianglelist",
      "Category": "Position",
      "DrawCategory": "Position",
      "ByteWidth": "12"
    }
  ]
}

Element fields:

- SemanticName
- Format
- ExtractSlot
- ExtractTechnique
- Category
- DrawCategory
- ByteWidth (optional; computed from Format when missing)

The file stem is used as GameTypeName unless GameTypeName is present in the JSON. Use a _2 suffix for same-name variants and set GameTypeName to the real name.

To add a new game: create a new folder under this directory and add JSON files.
The runtime scans folders by case-insensitive game name, so no Rust registry
change is required. IDENTITYV intentionally maps to the `IdentityV` folder.
