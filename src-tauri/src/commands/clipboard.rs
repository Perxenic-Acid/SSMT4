/* ═══════════════════════════════════════════════════════════════
CheeseCat (芝士猫) — Native Windows clipboard write
═══════════════════════════════════════════════════════════════
Uses the Win32 clipboard directly (OpenClipboard → EmptyClipboard →
GlobalAlloc → SetClipboardData(CF_UNICODETEXT) → CloseClipboard)
via the tiny `clipboard-win` crate instead of the generic
clipboard-manager plugin.

Why:
- The plugin's arboard path can fail intermittently when another
  process owns the clipboard, or when the WebView loses focus.
- arboard serves text through delayed rendering (WM_RENDERFORMAT),
  which Windows clipboard history (Win+V) often does not capture.
  Writing the data immediately makes it show up in Win+V history
  and survive even if the app exits right after copying.
═══════════════════════════════════════════════════════════════ */

/// Write plain text to the system clipboard using the Win32 clipboard
/// API. Fails with a descriptive message if the system clipboard is
/// held by another process for too long.
#[tauri::command]
pub fn clipboard_write_text(text: String) -> Result<(), String> {
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text)
        .map_err(|error| format!("无法写入系统剪贴板: {error}"))
}
