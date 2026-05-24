# AeroTab Design System (MASTER)

> **Source:** [UI UX Pro Max](https://github.com/nextlevelbuilder/ui-ux-pro-max-skill) query + Tokyo Night brand alignment.  
> **Rule:** Page-specific overrides live in `docs/design-system/pages/[page].md` if present.

**Project:** AeroTab  
**Category:** Developer terminal / SSH IDE desktop client  
**Visual baseline:** Dark OLED-friendly shell; xterm ANSI colors remain in `theme.ts` / `TerminalPane.svelte`.

---

## Pattern

- **Layout:** Data-dense shell — sidebar profiles, horizontal tabs, split panes, optional SFTP dock.
- **Density:** Compact 12–13px UI type; generous hit targets (min 32px row height).
- **Depth:** Dimensional layering via `panel` / `panel-2` / `border-soft`, not glassmorphism.

## Style

- **Mode:** Dark (Tokyo Night)
- **Motion:** `transition-colors` 150–200ms; respect `prefers-reduced-motion`
- **Icons:** Lucide only (no emoji as icons in chrome)
- **Interactions:** `cursor-pointer` on clickables; visible `:focus-visible`

## Color Tokens (CSS — `apps/ui/src/app.css`)

| Role | Hex | Variable |
|------|-----|----------|
| Background | `#0b0d12` | `--color-bg` |
| Panel | `#13161d` | `--color-panel` |
| Panel elevated | `#1a1e27` | `--color-panel-2` |
| Border | `#2a2f3b` | `--color-border` |
| Border soft | `#1f2330` | `--color-border-soft` |
| Text | `#e6e9ef` | `--color-fg` |
| Text muted | `#8b93a7` | `--color-fg-muted` |
| Accent | `#7aa2f7` | `--color-accent` |
| Success | `#9ece6a` | `--color-success` |
| Danger | `#f7768e` | `--color-danger` |
| Warning | `#e0af68` | `--color-warning` |

**Do not use:** AI-style purple/pink gradients, enterprise marketing green (`#22C55E`) as primary CTA.

## Typography

- **UI body:** IBM Plex Sans (Google Fonts)
- **UI mono / headings:** JetBrains Mono
- **Terminal:** existing mono stack in `@theme --font-mono`

```html
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet" />
```

## Spacing

| Token | Value |
|-------|-------|
| `--space-1` | 4px |
| `--space-2` | 8px |
| `--space-3` | 12px |
| `--space-4` | 16px |

## Elevation

| Token | Usage |
|-------|-------|
| `--shadow-panel` | Modals, command palette |
| `--ring-focus` | Keyboard focus ring |

## Component utilities (global classes)

| Class | Use |
|-------|-----|
| `.btn` / `.btn-primary` / `.btn-secondary` / `.btn-ghost` | Actions |
| `.input` | Text fields outside settings sections |
| `.panel` | Raised surfaces |
| `.list-item` / `.list-item-active` | Selectable rows |
| `.kbd` | Shortcut badges |
| `.shell-nav-item` | Settings sidebar nav |

## Brand

- Logo: `docs/assets/logo.png` → bundled as `apps/ui/src/assets/logo.png`
- Replace `›_` placeholders in sidebar and empty state.

## Anti-patterns

- Emojis as chrome icons
- Layout-shifting hover transforms
- Invisible focus states
- Instant state changes (no transition)

## Pre-delivery checklist

- [ ] Lucide icons only in shell
- [ ] `cursor-pointer` on clickables
- [ ] Hover/focus transitions 150–300ms
- [ ] `prefers-reduced-motion` honored
- [ ] Text contrast WCAG AA on `--color-fg` / `--color-bg`
- [ ] xterm theme logic unchanged
