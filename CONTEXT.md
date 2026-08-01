# AeroTab domain glossary

- **Profile** — A saved connection definition (SSH, serial, RDP, VNC, etc.) shown in the sidebar.
- **Session** — A live terminal pane backed by an open local/SSH/serial process.
- **Inline rename** — Editing a Profile (or group) name in place via the sidebar pencil control.
- **SFTP dock** — Per-tab SFTP browser column; uses a dedicated SFTP connection (not the interactive shell session).
- **SFTP pin** — Fallback dock target used only when the tab’s active pane is not SSH.
- **Transfer center** — Dedicated transfer tab hosting `FileTransferWindow` (queue, modes, progress).
- **Direct transfer** — Server-to-server copy executed on the **source** host (`rsync`/`scp`); destination auth uses AeroTab-managed SSH agent forwarding (no private key file written on the source). Traffic flows source ↔ destination.
- **Relay transfer** — Copy via AeroTab (backend SFTP read/write preferred; frontend chunk fallback). Traffic flows through the client machine.
- **Transfer mode** — User choice for remote↔remote: `auto` (direct then relay), `direct`, or `relay`.
- **Agent forward consent** — Per-source-host permission to forward the local/ephemeral SSH agent during Direct transfer; first use prompts, can be remembered.
