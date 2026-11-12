# AeroTab patches on vendored tao 0.35.2

Vendored from crates.io `tao 0.35.2` (exact registry sources under
`~/.cargo/registry/src/.../tao-0.35.2`). Kept under `[patch.crates-io]` because
`tauri-runtime-wry 2.11.2` depends on `tao = "0.35.0"` (^0.35), so a crates.io
bump to 0.36+ is not compatible without upgrading Tauri.

## Why

Windows UI-thread self-deadlock: `KEY_EVENT_BUILDERS` (non-reentrant
`parking_lot::Mutex`) was held across `PeekMessageW(PM_NOREMOVE)` inside
`KeyEventBuilder::process_message`. Peek dispatches inbound cross-thread **sent**
messages (`WM_ACTIVATEAPP`, `WM_KILLFOCUS`, …). Those messages are classified as
keyboard-related, so the wndproc locked `KEY_EVENT_BUILDERS` again → hang.

## Patch

Backport of upstream [tauri-apps/tao#1215](https://github.com/tauri-apps/tao/pull/1215)
(`c704261`, shipped in **tao 0.36.0**):

1. Peek the next key/IME message **before** acquiring input locks; pass the
   peeked state into keyboard / minimal IME handlers.
2. Hold `LAYOUT_CACHE` for shorter scopes; read keyboard state outside that lock.

Local defense in depth (not in #1215): `KEY_EVENT_BUILDERS.try_lock()` in
`event_loop.rs`. On re-entry, skip key-event synthesis for that message and let
default / later handling proceed.

### Focus-change bookkeeping impact

`WM_SETFOCUS` / `WM_KILLFOCUS` still update focus via the dedicated match arms
(`Focused` events). A failed `try_lock` only skips `KeyEventBuilder` synthetic
keydown/keyup for physically held keys. After #1215 that path should almost
never hit; if it does, modifiers may briefly disagree with OS key state until
the next real key message.

## Drop condition

Remove `third_party/tao` and the root `[patch.crates-io] tao = …` entry once
AeroTab’s Tauri stack resolves **tao >= 0.36** (fix present since 0.36.0;
current crates.io latest includes it through 0.37.x).

## Layout notes

- Not a workspace member (`Cargo.toml` `exclude = ["third_party/tao"]`) so
  `cargo fmt --all` does not reformat vendor sources.
- Upstream LICENSE / LICENSE.spdx retained.
