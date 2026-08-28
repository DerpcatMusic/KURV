# KURV Editor Visual Language

Apply these rules to new oscillator cards and LFO editors. Reuse the existing editor modules and tokens before adding UI code.

## Shared oscillator contract

- A Noise oscillator is an oscillator variant, not a separate card system. Keep the identity rail, enable/remove/reset/reorder behavior, host automation, modulation destinations, audio-rate source routing, and group placement shared with VA oscillators.
- Noise is intrinsically single-lane. Do not expose or apply unison voices, distribution, jitter, width, or pan-shape controls; runtime publication must force one lane so legacy presets and automation cannot re-enable them.
- Start from `OscillatorConfig::default()` and override only Noise-specific fields. Double-click and context-menu reset must restore that same default contract.
- Use the compact filter-height card for Noise. Its body has two regions: one filled DSP-faithful waveform on the left and `LEVEL`, `PAN`, `TILT`, `GAPS`, and `STEREO` on the right.
- Do not expose pitch or phase controls: the procedural Noise source does not consume oscillator tuning.
- Audio-rate generator routing uses the small identity-rail source port and an accent outline on valid later targets. Keep route depth as a compact overlay on the carrier graph; never reuse the six-dot reorder grip or imply cyclic/cross-group routing.

## Layout

- Follow `editor_generator/oscillator_card.rs`: a narrow left identity rail and contiguous functional panels separated by subtle vertical dividers. VA uses oscillator, unison, and pan panels; Noise uses preview and shaping panels. Use `editor_theme::space`, `shape`, `graph_inset`, and `compact_gap`; no local spacing scale.
- Keep graphs as the dominant surface and parameter readouts as compact aligned rails/cells. Prefer proportional splits that survive resizing over fixed pixel layouts.
- LFO cards retain the existing header order: collapse, reorder grip, modulation-source handle/name, contextual status, remove. Expanded cards place the curve beside one compact vertical control column.
- Use tonal wells and dividers for hierarchy. Do not nest rounded cards inside cards or add decorative section containers.

## Graphs and curves

- A graph represents the actual implemented signal or modulation curve. Sample the same evaluator used by playback where practical; label an approximation when it cannot be exact.
- Oscillator previews apply the oscillator Level to vertical amplitude. Noise previews normalize Tilt-dependent source energy first, then apply Level, and distinguish left/right channels with semantic accent colors.
- Use the graph's full useful rect with a small tokenized inset. Draw a restrained grid/baseline, one accent stroke, and a low-alpha fill or glow only when it improves reading.
- Cache static meshes by every input that changes geometry, including rect, scale factor, theme color, and source generation. During drags, use bounded point counts and request repaint at display cadence; never couple painting to audio-thread work.
- Curve interaction follows the existing editor: crosshair on empty graph, grab/grabbing on points and handles, Shift for fine movement, Alt to bypass snapping, double-click for add/remove/reset, and right-click for contextual reset.
- Show snap guides only while relevant. Playheads may use a short fading trail; avoid permanent animation, spectrum decoration, or motion unrelated to sound.

## Type, color, and interaction

- Use `editor_theme::font::{caption,label,title,value}` and short uppercase control labels. Values remain concise and use consistent units.
- Use `editor_theme::semantic()` roles, never literal UI colors. The owning oscillator/group/source accent marks active values, focus, curve strokes, modulation handles, and drag feedback; neutral surfaces carry structure.
- Use `editor_theme::control_visuals` for enabled, hover, active, focus, and disabled states. Disabled content dims but remains legible.
- Every compact target gets a useful tooltip, correct cursor, keyboard focus, and Enter/Space activation where applicable. Reorder uses the shared six-dot grip and a drag ghost; destructive actions use the danger role.
- Keep corners tight (`CONTROL_RADIUS`), strokes tokenized, and depth tonal. Do not add shadows, glass, gradients, emoji, Unicode-as-icon decoration, or oversized typography.

## Anti-patterns

- Do not fork an entire oscillator card for Noise.
- Do not expose DSP implementation details as controls without a stable musical meaning.
- Do not invent a new knob, graph, palette, spacing scale, font size, or gesture when an existing helper covers it.
- Do not render a decorative curve that disagrees with playback, allocate continuously for unchanged geometry, or perform analysis in paint code.
- Do not add controls “for later.” Every visible control must change the current sound or editing workflow.
