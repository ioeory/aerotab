# AeroTab on Wayland

AeroTab is a Tauri 2 + WebKitGTK desktop app. On Linux it uses the native
session type: when `WAYLAND_DISPLAY` is set, GTK/WebKit runs as a Wayland client.

## Expected to work

- SSH / local terminals (xterm.js in the webview)
- SFTP browser and file transfers (HTML5 drag-and-drop; Tauri `dragDropEnabled: false`)
- SSH agent forwarding (`SSH_AUTH_SOCK`)
- Config sync, profiles, vault, settings
- In-app hotkeys while the window is focused
- System tray (via libayatana-appindicator / StatusNotifier; DE-specific)

## Limitations on Wayland

### SSH X11 forwarding

X11 forwarding bridges remote X11 apps to a **local X11 socket** (`/tmp/.X11-unix/X*`).
On Wayland this requires **XWayland** so `DISPLAY` is set (e.g. `:0`).

- Install XWayland on Arch: `sudo pacman -S xorg-xwayland`
- Enable X11 forwarding in Settings → SSH
- Optional: set **X11 DISPLAY** if auto-detection fails
- Only **X11/XWayland clients** work—not native Wayland GUI programs from the remote host

If X11 forwarding is enabled but no socket is reachable, SSH connect returns a
clear error instead of failing silently.

### Native terminal embed (experimental)

Embedding Alacritty/Ghostty/Kitty **inside** a pane requires reparenting a foreign
window. Wayland forbids this. Use **Open in external terminal** from the terminal
context menu, or the detached spawn fallback when embed is unavailable.

### Window transparency

Settings → Window → background opacity below 100% may look wrong on some Wayland
compositors. Use 100% if the webview background does not blend correctly.

### Not implemented on any Linux session

- Edge dock mode / global show-hide hotkey (settings are stored only)
- OS-global hotkeys (bindings apply when AeroTab is focused)

## Session introspection

The desktop shell exposes:

- Tauri command: `session_info`
- JSON-RPC: `desktop.sessionInfo`

Returns `{ platform, wayland, display, x11ForwardAvailable }` for UI hints and
diagnostics.

## Manual test checklist

On Arch + GNOME/KDE Wayland:

1. Launch AeroTab, open SSH session, split panes, resize
2. SFTP dock: upload/download via drag-and-drop
3. Tray: minimize to tray and restore
4. Window opacity 80% — check for artifacts
5. SSH X11: with XWayland, run `xclock` or `xeyes` on remote; without XWayland,
   confirm Settings → SSH shows the Wayland hint and connect is blocked when X11
   forwarding is on
6. Terminal menu → **Open in external terminal**
7. Command palette / in-app shortcuts (`Ctrl+Shift+P`, etc.)

There is no Wayland job in GitHub Actions; validate on a real Wayland session
before release.

## Related code

- [`src-tauri/src/desktop_session.rs`](../src-tauri/src/desktop_session.rs)
- [`src-tauri/src/ssh/mod.rs`](../src-tauri/src/ssh/mod.rs) — `resolve_x11_socket_path`, `validate_x11_forward`
- [`src-tauri/src/native_terminal/embed.rs`](../src-tauri/src/native_terminal/embed.rs)
- [`apps/ui/src/lib/sessionInfo.ts`](../apps/ui/src/lib/sessionInfo.ts)
