# Claude Deck design system (MASTER)

Made with the ui-ux-pro-max skill, 2026-10-20. Every component follows this file. The tokens live in `src/style.css` (Tailwind v4 `@theme`).

**Character:** a calm, dense, professional developer tool, like a modern IDE side panel. The interface stays dark and quiet so that state colours (green, blue, amber) carry the meaning. There are no gradients, glows or decoration.

## Typography
- UI text uses `font-sans`, which is IBM Plex Sans Variable, bundled with @fontsource.
  - Base size is 13 px.
  - Secondary text is 12 px.
  - Section labels are 11 px, uppercase, `tracking-wider`, `text-deck-faint`.
- Paths, file names, shell names, ids and the terminals use `font-mono`, which is JetBrains Mono Variable.
- Weights: 400 for body text, 500 for names, 600 for headings and the active item. Never use 700 or above in chrome UI.

## Colour and surfaces
- **Back to front:** `deck-bg` (window), `deck-sidebar`, `deck-panel` (headers, tab bar, footer), `deck-term` (terminal areas), `deck-elevated` (menus, dialogs, popovers).
- **Hover:** `deck-hover`.
- **Borders:** `deck-border` for 1 px dividers; `deck-border-strong` for inputs and focus outlines.
- **Text:** `deck-text` is primary, `deck-muted` secondary, `deck-faint` for labels and counts. Never use anything dimmer than `deck-faint` for readable text.
- **Accent `deck-accent`:** the primary button, the active indicator bar, focus rings and links.
- **Selection:** the active item uses `bg-deck-active` plus a 2 px accent bar on the left (sidebar) or top (tabs).
- **Needs you is amber, never accent:** `bg-needs-soft`, a 2 px `dot-needs` bar on the left (sidebar) or top (tab), and amber text for the label.
  - A row that is both active and Needs you shows the amber treatment.
  - Active and Needs you must never look alike.
- **Errors:** `deck-danger`.

## Icons
- Use `lucide-vue-next` only.
  - Stroke width is 1.75. Size 16 for toolbar buttons, 14 inline.
- No emoji, and no text glyphs (`+`, `×`, `↻`) used as icons.
- Every icon-only button has an `aria-label` and a `title`.

## Spacing and size
- Use a 4 px grid.
- Sidebar rows are 30 px high, with 8 px horizontal padding and a 16 px indent per level.
- Tab bar is 36 px. Panel headers are 36 px. The footer is 26 px.
- Icon buttons are 28×28 px, with a hover background of `deck-hover` and `rounded-md`.
- Terminals have **12 px inner padding**: text must never touch a panel edge.

## Dividers and splitters
- Panels are separated by 1 px `deck-border` lines, not thick bars.
- A splitter is the 1 px line with an invisible 8 px hit area.
  - On hover or drag the line becomes 2 px `deck-accent` after a 150 ms delay.
  - The cursor is `col-resize`.

## Motion
- Use 150 ms `transition-colors` for hover and focus.
- Only colour and opacity change on hover, so nothing shifts the layout.
- Working dots pulse slowly, over 2.5 s. Respect `motion-reduce`, so nothing moves when reduced motion is set.

## Focus and accessibility
- Every focusable element shows `focus-visible:ring-2 ring-deck-accent ring-offset-0`.
- Text contrast is at least 4.5:1, which the faint colour meets on every surface.
- State is never shown by colour alone: dots have a `title` and an `aria-label`, and Needs you also has the words.
- `cursor-pointer` goes on everything clickable.

## Dialogs
- Width is 440–480 px, `rounded-xl`, `bg-deck-elevated`, 1 px `deck-border-strong`, `shadow-2xl`, and a `bg-black/60` backdrop.
- Padding is 20 px. The title is 15 px weight 600, and a short explanation sits under it in `deck-muted`.
- Buttons go on one right-aligned row, in this order: Cancel (ghost), secondary (outline), primary (accent, filled).
  - Height is 32 px with 12 px horizontal padding.
  - Labels never wrap. Widen the dialog rather than wrap.
- Inputs are 32 px high, `bg-deck-bg`, with a 1 px `deck-border-strong` border and an accent border on focus. Use a toggle switch for on/off settings.

## Empty states
- Show a centred icon, a 15 px title, one line of `deck-muted` explanation, and one primary button.
- Optionally add a keyboard hint in `font-mono` `deck-faint`.
