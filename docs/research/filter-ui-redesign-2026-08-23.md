# Filter UI redesign research — 2026-08-23

> Design note only. The requested filter-panel direction is Phase Plant-inspired: a compact modular card with an obvious type picker, a useful response visualization, and dense parameter controls. No Rust files were edited while producing this note.

## Recommendation

Keep the existing KURV filter module as one compact card, but make the first control an explicit **TYPE** dropdown. Place the transfer-curve preview in the left/center work area and the dense control strip on the right, matching the reference scan order. Do **not** add an oscilloscope or waveform plot beside the filter. The left rail remains the existing filter identity and drag/remove affordance.

The panel should preserve the current four editable controls and their gestures:

- **Cutoff** — logarithmic frequency, 20 Hz–20 kHz.
- **Q** for SVF, or the existing **Skew** interpretation for Phaser/Fibonacci.
- **dB/oct** — the existing slope control, including the 128-pole upper end.
- **Morph** — 0–100%.

The type menu exposes only KURV's existing modes: **SVF MORPH**, **PHASER**, and **FIBONACCI**. Phase Plant is an interaction reference, not a reason to add its filter types or change KURV's DSP model.

## Evidence standard

- **Verified local:** source inspected in the 2026-08-23 checkout.
- **Verified first-party external:** Kilohearts' Phase Plant documentation.
- **Design recommendation:** an explicit proposal, not a claim about KURV behavior that is not present in source.

## Current checkout map

### The requested “Vizia UI” does not match this checkout

`Cargo.toml` depends on `egui = 0.34.1`, `truce-egui = 6.3.0`, and `truce-core`; it contains no Vizia dependency. The editor constructs `truce_egui::EguiEditor` in [`src/editor.rs`](../../src/editor.rs), and the shell draws with `egui::Ui` in [`src/editor_shell.rs`](../../src/editor_shell.rs). The `rg` scan for `vizia`, `vizia_plug`, `nih_plug_vizia`, and `nice-plug-vizia` found no active KURV UI source.

This proposal therefore describes the current egui card's visual and interaction contract. If another branch contains the Vizia implementation, port the same layout and behavior there rather than assuming Vizia APIs in this checkout.

### Shell and rack placement

- The editor defaults to `1120 × 720` and allows `960 × 480` minimum size ([`src/editor.rs`](../../src/editor.rs)).
- The shell gives the generator rack 72% of workspace width and the modulation/performance column 28% ([`src/editor_shell.rs`](../../src/editor_shell.rs), lines 113–136).
- A filter is a module in a group card. Its height participates in group layout and insertion/drop geometry ([`src/editor_generator/insertion/group_card.rs`](../../src/editor_generator/insertion/group_card.rs), lines 90–105 and 143–183). Changing the card's height or width needs to preserve those reserved rectangles.

### Existing filter card geometry

[`src/editor_filter.rs`](../../src/editor_filter.rs), lines 43–70, already has the right broad structure:

1. a narrow identity rail;
2. a control column;
3. a larger response preview.

The current identity rail is about 5.5% of the inner width (clamped to a small minimum). The controls occupy 27%–38% of the body, and the preview takes the remainder. The body uses the shared `well` surface and the existing 2 px control radius.

There is no left oscilloscope in this filter card. The current plot is the response curve on the right. The waveform-cycle plot belongs to the oscillator card (`src/editor_generator/oscillator_card.rs`, lines 134–180 and 350–371), so this filter redesign must not move or duplicate that oscillator visualization.

### Existing type picker and controls

The code already opens an egui popup from the first control cell:

- `filter_type_popup_open` owns the popup identity ([`src/editor_filter.rs`](../../src/editor_filter.rs), lines 29–32).
- `egui::Popup::menu` lists `FilterMode::ALL` and highlights the selected mode (lines 115–141).
- The displayed readout currently says `TYPE` and uses the short value `SVF`, `PHASE`, or `FIB` (lines 142–155).

The remaining cells are the existing controls (lines 168–220 and 239–277). Their behavior is part of the compatibility surface:

| Control | Existing gesture | Mode-specific display | Reset |
|---|---|---|---|
| Cutoff | Vertical metric drag; direct horizontal position in the response graph; Shift for fine control | Frequency (`Hz`/`kHz`) | Double-click uses mode default |
| Resonance | Vertical metric drag | `Q` for SVF; `SKEW` with LEFT/CENTER/RIGHT summary for Phaser/Fibonacci | Double-click uses mode default |
| Slope | Vertical metric drag | `DB/OCT`; 6–768 dB/oct | Double-click uses mode default |
| Morph | Vertical metric drag | Percentage | Double-click uses mode default |

The response graph is also an interaction surface. It maps cutoff horizontally and Q vertically. For Phaser/Fibonacci, its vertical gesture is documented as stage-bank skew. Its grid includes 100 Hz, 1 kHz, and 10 kHz marks, a 0 dB guide, a cutoff guide, and stage ticks where applicable ([`src/editor_filter/painting.rs`](../../src/editor_filter/painting.rs), lines 66–165 and 167–220).

### Existing structural and modulation behavior

The card is more than paint. `draw_compact_filter` in [`src/editor_generator/mod.rs`](../../src/editor_generator/mod.rs), lines 46–205, attaches behavior that must remain intact:

- drag handle reorders or moves the module between groups;
- right-click offers `RESET FILTER` and `REMOVE FILTER`;
- the close affordance removes the module and clears its modulation/host bindings;
- each of Cutoff, Resonance, Slope, and Morph remains a modulation destination;
- host automation is applied for display and its gesture is updated during edits;
- changing a mode starts that mode's existing defaults (`defaults_for_mode`), rather than silently carrying incompatible values.

The group card deliberately redraws a filter when its type popup is open, even if the card is outside the visible rack clip ([`src/editor_generator/insertion/group_card.rs`](../../src/editor_generator/insertion/group_card.rs), lines 158–165). A replacement dropdown must keep a stable owner ID and this popup-open behavior.

### Existing visual language

Use the shared theme. The current palette is charcoal/near-black with cyan primary, amber secondary, and violet tertiary accents (`src/editor_theme.rs`, lines 14–18 and 140–170). Spacing is 2/4/8/12/16 px, text sizes are approximately 9.5/10/10.75/12 px, and the control radius is 2 px (`src/editor_theme.rs`, lines 194–241). Active, focused, hovered, disabled, and modulation states already have centralized color rules (`control_visuals`, lines 237–302). Do not introduce a second filter-only style system.

## Phase Plant reference

Kilohearts' first-party Phase Plant documentation describes three useful patterns:

1. The UI is organized around a modular generator area. Modules are added individually and can be moved by drag-and-drop.
2. Generator modules expose a visual representation of their output.
3. The Filter Effect has a **Type** selector followed by named parameters such as Cutoff, Q, Gain, and Slope.

Sources:

- [Phase Plant User Interface](https://kilohearts.com/docs/phase_plant#user_interface)
- [Phase Plant Generator Area](https://kilohearts.com/docs/phase_plant#generator_area)
- [Phase Plant Filter Effect](https://kilohearts.com/docs/phase_plant#filter_effect)

The KURV adaptation should borrow the scan order and modular-card relationship, not copy Phase Plant's visual assets or add controls that KURV does not support. Kilohearts' docs also describe scopes as module visualizations. This request explicitly removes a left oscilloscope, so KURV's response curve stays the single visualization in the filter card.

## Proposed panel

### Wireframe

```text
┌─ FLT ─┐┌──────────────────────────────────────────────┬──────────────┐
│  1   ││             transfer-curve preview            │ TYPE ▾       │
│      ││      100       1k       10k                   │ CUTOFF       │
│ ⋮⋮   ││      cutoff handle / 0 dB                    │ Q / SKEW      │
│ ⋮⋮   ││                                                │ DB/OCT       │
│      ││                                                │ MORPH        │
└──────┘└──────────────────────────────────────────────┴──────────────┘
```

This is a layout sketch, not a pixel specification. The identity rail stays narrow. The type dropdown occupies the top of the controls column and has a clear chevron, a selected-row check mark in the menu, and the full mode label. The left/center side is a response-curve preview, never an oscilloscope.

### Geometry

Keep the current outer proportions first, then tune only the internal control grammar:

- Preserve the identity rail at roughly 5.5% of the inner width, with its `FLT` label, number, close affordance, and six-dot drag grip.
- Preserve a control column at roughly 27%–38% of the body width. Give its type row a stable height equal to one metric row; do not let a long mode name resize the row.
- Put a fixed-width trailing chevron in the type row. Reserve width for `FIBONACCI` so opening and selecting the menu never shifts the graph.
- Keep the graph as the flexible left/center region and the control strip fixed on the right. Give the graph a small minimum width at the editor's minimum window size; if space is tight, reduce grid labels before reducing the four controls' hit areas.
- Keep the existing card `well` background and subtle separator between controls and graph. Avoid a nested card around every value.
- Preserve the existing filter card height contract used by group insertion and drag previews.

### Type dropdown behavior

Display the full selected label in the closed control:

- `SVF MORPH`
- `PHASER`
- `FIBONACCI`

On click, open a vertical menu anchored to the type row. The menu should:

- show exactly those three choices;
- mark the current choice with a check or accent rule;
- remain within the editor/host viewport when the card is near an edge;
- accept pointer selection and the normal egui keyboard path (focus, arrows, Enter, Escape);
- retain the module identity and control IDs while open;
- close after selection without moving the card or scrolling the rack;
- keep double-click on the closed type control as “reset the whole filter,” matching the current behavior.

Selecting a different mode must retain the current defaults contract: SVF defaults to the default `FilterConfig`; Phaser defaults to cutoff 800 Hz, Q 1.8, slope 48 dB/oct, morph 100%; Fibonacci defaults to cutoff 500 Hz, Q 1.8, slope 48 dB/oct, morph 100%. Do not preserve incompatible values merely to make the dropdown appear less destructive; the existing code deliberately resets them.

### Control presentation

Keep the existing metric readout grammar: small muted label above a larger value, accent on hover/active, fixed cell widths, and vertical drag. Use the full `TYPE` label for the dropdown, but keep the existing mode-dependent `Q`/`SKEW` labels and value formatting.

Do not add Gain, Drive, Mix, filter-family submenus, or a second resonance control. These would change the DSP/data contract and violate “preserve existing controls.”

Keep the response-curve direct manipulation as a power-user shortcut:

- horizontal pointer position sets logarithmic Cutoff;
- vertical pointer position sets Q or mode-specific skew;
- Shift applies fine relative movement;
- double-click restores the current mode defaults;
- the visible handle and curve update continuously;
- modulation/host-automation overlays continue to use the existing destination tracks.

### No left oscilloscope

The left side is reserved for identity and module operations only. It must contain no waveform, audio scope, analyzer, or duplicate response plot. In particular:

- do not call the oscillator waveform view from the filter card;
- do not allocate a new left plot that changes the card's height;
- do not label the response graph as `SCOPE`; label it as the filter response/curve if a label is needed;
- keep frequency-grid labels and stage ticks in the existing right preview.

This leaves the visual hierarchy clear: identity → type/values → response curve.

## State and interaction contract

The redesign is presentation-only. A future implementation should preserve these contracts:

| Surface | Must remain true |
|---|---|
| Type selection | Writes the existing `FilterConfig.mode` and applies existing per-mode defaults. No new parameter ID. |
| Cutoff/Q/Slope/Morph | Writes the same four `FilterConfig` fields through existing custom drag paths. Keep logarithmic/linear ranges unchanged. |
| Modulation | Each existing `FilterControl` remains a destination. A dropdown open state must not steal the graph/control destination IDs. |
| Host automation | Existing begin/set/end gesture behavior remains; preview automation still updates from the preview gesture when it owns the gesture. |
| Reorder | The six-dot grip remains the only reorder affordance. The dropdown must not start a module drag. |
| Remove/reset | Close icon and context-menu actions keep their current behavior and tooltips. |
| Persistence/history | A mode or value edit still causes the shell's ordinary persisted-state/history commit path. No separate UI-only state is serialized. |
| Disabled/offscreen | The card can be skipped when clipped, but its type popup owner remains drawable while the popup is open. |

## Acceptance checklist

A visual implementation is ready for review when all of the following are true:

1. The filter card shows a clear closed-state `TYPE` dropdown with one of the three existing full labels.
2. Opening the dropdown shows all three existing modes and marks the current mode.
3. No oscilloscope or waveform is present on the left side of the filter card.
4. Cutoff, Q/Skew, dB/oct, Morph, response-curve direct manipulation, drag/reorder, remove, reset, modulation, and host automation still work.
5. Mode changes use the existing defaults and mode-dependent labels.
6. The graph remains the flexible region and still shows the curve, cutoff handle/guide, grid labels, 0 dB line, and non-SVF stage ticks.
7. The card does not shift when `FIBONACCI` is selected, the menu opens, or a value grows from `Hz` to `kHz`.
8. The layout remains legible at 960 × 480 and at the default 1120 × 720 editor sizes.
9. Popup focus and Escape behavior do not close the settings/preset overlays unexpectedly.
10. No Rust source file or DSP behavior was changed as part of this design-only phase.

## Suggested implementation seam (for a later task)

This note does not implement the design. If implementation is requested later, start in the existing filter UI seam:

- `src/editor_filter.rs`: type-control presentation, popup row geometry, and interaction IDs;
- `src/editor_filter/painting.rs`: header/readout/response-curve paint only;
- `src/editor_generator/mod.rs`: only if interaction plumbing needs to expose a new response, while preserving existing modulation and host-automation paths;
- `src/editor_generator/insertion/group_card.rs`: only if popup visibility or height contracts change.

Do not touch filter DSP/configuration or add parameters for this visual redesign.

## Decision summary

**Adopt:** explicit dropdown, full mode names, current four controls, right-side response curve, existing KURV tokens, existing drag/modulation/automation contracts.

**Reject:** left oscilloscope, duplicated waveform plot, new filter families, new parameters, Phase Plant-specific visual assets, or a second control model.
