# Secondary Motion SSMT package

This package integrates the upstream `sbm.dll` runtime as an SSMT plugin.

This is an SSMT adaptation based on Sp1cHless's original project and is not an
official release of that project. It is maintained by a third party on a best-effort
basis, with no guarantee of continued updates or timely compatibility with future
game versions. Please do not ask the original project's author to expedite updates
or report issues with this adaptation to them.

- The DLL is loaded through SSMT's direct extra-DLL injection path.
- `plugin/sbm.dll` remains inside the installed package.
- Runtime configuration stays in the sibling `SecondaryMotion/` directory.
- No proxy DLL or runtime data is copied to the game root.
- The marketplace plugin's enable switch controls whether SSMT injects the DLL.
- The plugin settings page controls the runtime's global bone-write switch and active preset.

Build the package with:

```powershell
$UpstreamRoot = "C:\Path\To\Arknights-Endfield-Plugin-Secondary-bodyphysics"
pwsh -File scripts/build-secondary-motion-package.ps1 `
  -SecondaryMotionRoot (Join-Path $UpstreamRoot "SecondaryMotion") `
  -DllPath (Join-Path $UpstreamRoot "bin\sbm.dll")
```

The upstream project is licensed under GPL-3.0. Keep its license, notices and source
availability when redistributing a modified build.

## Risk and disclaimer

This plugin injects third-party native code into the game process and modifies Unity
IL2CPP bone updates with MinHook. It may trigger anti-cheat or account risk controls,
cause crashes, broken bones, clipping, or save-related issues, and may stop working
after a game update. Third-party DLLs have full in-process native access; use only a
trusted build.

The package is provided as-is without any express or implied warranty. Its authors,
SSMT integrators, and distributors are not responsible for account bans, data
corruption, device failure, save loss, or other direct or indirect damage. Installing,
enabling, or launching with it means the user accepts these risks and confirms
compliance with applicable law, game terms, and server rules.
