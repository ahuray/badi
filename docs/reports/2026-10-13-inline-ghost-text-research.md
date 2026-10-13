# Inline ghost text research — 2026-10-13

Question: Cotypist's suggestions sit *inside* the input field on the text
baseline (screenshots: terminal prompt, X composer, Slack reply, Mail). Badi's
suggestions visibly hover. What is the highest-quality way to get the same
result on Omarchy (Hyprland 0.56.2, Fcitx 5.1.22, eDP-1 at scale 2)?

## Why Badi hovers today (three display paths, only one is inline)

1. **Fcitx candidate panel** — every native field without observer geometry,
   plus every fallback when calibration fails ("Any inconsistency yields no
   geometry, and the addon uses the Fcitx panel",
   [observer runbook](../../adapters/accessibility/README.md#caret-geometry-and-preview)).
   `showCandidate` in `adapters/fcitx5/src/addon.cpp` renders the suggestion
   as a single candidate plus an auxiliary "keys" line. A candidate window is
   a floating box by definition; this is most of the hovering the user sees.
2. **Dark chip** — when words follow the caret on its line (`text_follows`),
   the layer-shell preview deliberately draws a pill below/above the line
   instead of covering the following words (`place_chip` in
   `adapters/accessibility/preview.py`).
3. **Inline layer-shell ghost** — the only Cotypist-like path, used when
   geometry calibrates, no text follows, and the text fits inside the field
   box. Three defects keep it from feeling native:
   - **Hyprland animates it.** The preview uses namespace
     `badi-writing-preview` on the overlay layer, but nothing ships a
     `layerrule`. This machine's `layersIn` animation is enabled
     (easeOutQuint, speed 4), so every ghost fades/slides in over ~400 ms —
     perceived as floating, and it eats the latency budget the broker won.
   - **Baseline is estimated, not measured.** Without AT-SPI font attributes
     (Qt apps such as Omawrite report none), size falls back to
     `line_height * 0.86` in the overlay's own face, and the baseline is
     placed by ratio (`place_inline`), so the ghost can sit a few pixels off
     the field's baseline with mismatched glyph widths.
   - **Fit-or-nothing at the field's right edge.** Text that would pass
     `field_right` is not drawn inline at all, dropping to chip/panel even
     when the visible line continues.

## What Cotypist actually does

Cotypist (and the open-source TabType) do **not** inject text into the app's
own rendering. On macOS they read the focused element through Accessibility,
take the caret/range bounds (`AXBoundsForRange`), and draw a borderless,
non-activating panel exactly on that baseline in the host font, inserting via
Accessibility on Tab. The "inside the field" look is an overlay placed with
pixel discipline — the same architecture as Badi's path 3, executed tighter.
Wayland has no global window positioning, which is why Badi uses
`zwlr_layer_shell` anchored to the monitor with margins; that part is correct
and matches what other overlay tools do (they universally add
`layerrule = noanim, <namespace>` on Hyprland).

## Option A: true inline via Fcitx client preedit — rejected as primary

`setClientPreedit` would let each app render the ghost in its own text
layout (perfect font, baseline, scrolling, wrapping for free). It fails the
product contract on Omarchy today:

- `text-input-v3` has **no preedit styling** (a known protocol gap vs v1):
  apps render preedit as underlined composition, not grey ghost text, and
  Chromium's Wayland IME preedit/candidate placement has open displacement
  bugs under scaling.
- Preedit is semantically *composition*: web fields fire `compositionupdate`/
  `input` events, search-as-you-type and validation react to text the user
  never accepted, and several toolkits commit preedit on focus loss — an
  insertion without explicit acceptance. That breaks ADR 0003/0004, the
  foreign-composition yield, and "acceptance is explicit and one-shot".
- It only exists where the Fcitx frontend negotiates the Preedit capability;
  observed IME-parity fields are exactly the ones where this is unreliable.

Verdict: do not adopt. At most, a per-app Lab experiment could measure it
later; production stays overlay + append-only commit.

## Option B (recommended): Cotypist-grade overlay

Keep the layer-shell architecture; close the precision and fallback gaps, in
this order. Each step is independently shippable and screenshot-verifiable.

1. **Kill the compositor animation for the preview namespace.** Ship a
   Hyprland layer rule (`noanim`, no blur/shadow) for
   `badi-writing-preview`: applied at observer startup via `hyprctl keyword`
   for the running session and persisted by the installer as a user-config
   snippet (Omarchy's Lua config: `hl.layer_rule`). Zero protocol risk;
   removes the floating feel and ~400 ms of perceived latency. Validate with
   `hyprctl configerrors` and a live capture.
2. **Measure the baseline from real font metrics.** When the field reports a
   family/size, anchor the ghost baseline at `glyph_top + Pango ascent` for
   that exact font description instead of the line-height ratio. When it
   reports none (Qt), derive ascent from the caret glyph box of the preceding
   character via AT-SPI range extents and keep the current face. Success
   metric: ≤ 1 physical px baseline error on disposable Chromium and
   Omawrite captures (the two existing live lanes).
3. **Snap to the physical pixel grid.** Compute the layer margins in
   physical pixels at the monitor scale, round there, then convert back to
   logical — at scale 2 the current logical rounding can land half a
   physical pixel off, which reads as blur/shift against the field's text.
4. **Prefer inline; demote chip and panel.** Instrument first (the addon
   already records `display_overlay` vs `display_dispatched`): publish the
   fallback rate per app in the runbook, then relax `place_inline` only where
   the field box is provably not the visible line edge (the existing
   `field_box_ends_before_caret` case generalizes). Keep the chip only when
   following text would genuinely be covered. Target: candidate panel becomes
   the rare exception on observed apps, never the default.
5. **Restyle the remaining Fcitx panel fallback** (native fields): drop the
   auxiliary keys line once the user has accepted once, and ship a minimal
   classic-UI theme so the single candidate reads as quiet grey text at the
   caret rectangle the app reports. Still a panel, but visually adjacent to
   the field instead of a dialog.
6. **Follow the field, not just scroll events.** Bounds-changed already
   moves/clears on scroll; add window move/resize via the same AT-SPI frame
   events so a dragged window never leaves a ghost behind (fail closed:
   hide on any unproven transform, as today).

## The other screenshot features

- **Inline typo fix** (`recieve → receive`, Tab to apply): this is Badi's
  spelling replacement. Where replacement authority exists (editor-owned,
  native exact fields), the same layer surface can draw the correction over
  the typo range from AT-SPI range extents (strike + replacement) with the
  existing exactly-bound, one-shot acceptance. IME-parity apps stay
  append-only per ADR 0003 — presentation must never imply an edit the
  adapter cannot make exactly.
- **Conversation-aware replies** (Slack screenshot): same-window context,
  already the top "Better suggestions" backlog item, gated by the VISION
  context firewall and independent Lab evidence. No shortcut.
- **Terminal agent prompts** (first screenshot): terminals send no
  surrounding text; each needs its own channel (backlog, "Remaining
  surfaces"). The Bash Readline client is the pattern; Ghostty/agent TUIs
  stay unsupported until such a channel exists.

## Verification plan for Option B

- Fixture: existing preview/geometry unit tests extended for ascent anchoring
  and physical snapping (`npm run accessibility:check`).
- Real-broker: `observed-desktop.py` lane unchanged and green.
- Live (graphical session, disposable text): Chromium textarea + Omawrite,
  screenshot pairs measuring baseline offset and first-frame position (no
  animation), at scale 2 and one fractional-scale monitor if available.
  Report fixture, real-broker and live evidence separately, per AGENTS.md.

## Sources

- Fcitx5/Wayland preedit and per-app capability model: ArchWiki "Fcitx5";
  `text-input-v3` known gaps (no preedit styling) from the Wayland protocol
  review literature; Chromium Wayland IME candidate displacement reports.
- Hyprland layer rules (`noanim` per namespace): Hyprland wiki, Window
  Rules; overlay tools on Hyprland ship the same rule for responsiveness.
- TabType (open-source Cotypist alternative, macOS): AX-tree context +
  overlay ghost text, confirming the overlay-not-IME architecture.
