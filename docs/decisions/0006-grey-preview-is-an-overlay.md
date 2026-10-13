# ADR 0006: the grey preview is an overlay, never client preedit

- Status: accepted
- Date: 2026-10-13
- Scope: how observed fields display a suggestion: the
  [observer's preview](../../adapters/accessibility/README.md#caret-geometry-and-preview)
  and the [Fcitx addon](../../adapters/fcitx5/README.md)

## Context

Cotypist's suggestions look as if they are inside the field, on the text
baseline. On macOS that is an Accessibility-positioned, non-activating panel
drawn over the caret in the host font, not text inside the application. Badi
can get the same look two ways on Wayland: a layer-shell overlay placed from
AT-SPI caret geometry, or Fcitx client preedit rendered by the application
itself, which would give the app's own font, baseline, scrolling and wrapping.

Client preedit fails the product contract on Omarchy:

- `text-input-v3` has no preedit styling, so applications draw preedit as
  underlined composition rather than grey suggestion text. Chromium's Wayland
  preedit and candidate placement also shift under scaling.
- Preedit is composition. Web fields fire `compositionupdate` and `input`
  events, so search-as-you-type and validation react to text the user never
  accepted, and several toolkits commit preedit on focus loss: an insertion
  without explicit acceptance.
- It exists only where the frontend negotiates the Preedit capability, and the
  IME-parity fields ([ADR 0003](0003-ime-parity-append-only.md)) are exactly
  where that is least reliable. The addon must also keep yielding to a
  foreign input method's preedit.

## Decision

The grey preview is a layer-shell overlay (`badi-writing-preview`) drawn from
calibrated caret geometry in the host font when the field reports one. The
addon never sets client preedit for a suggestion; acceptance stays one
explicit, one-shot commit (or an editor-owned edit), and a field without
proven geometry falls back to a single quiet Fcitx candidate.

## Consequences

- Precision is the overlay's job: Pango-ascent baseline anchoring,
  physical-pixel snapping, a Hyprland `no_anim` layer rule and the
  following-glyph bound. On 2026-10-13 a disposable Chromium textarea and
  Omawrite measured 0 physical px between typed and grey glyph bottoms at
  scale 2.
- Any unproven transform (calibration failure, scroll the toolkit does not
  report, a field outside the viewport) hides the overlay or uses the panel;
  it never guesses a position.
- A per-app preedit experiment may run in the Lab, but promoting it would
  need a new ADR that answers the composition-event and focus-loss problems.
