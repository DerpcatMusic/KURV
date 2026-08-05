# KURV Shape versus Polaar Curve

Research date: 2026-08-04. Truce dependency: 6.3.0 (`Cargo.toml` and
`Cargo.lock`).

## The exact mismatch

Before this audit, KURV's Shape editor was not the Polaar Curve editor. KURV
had five fixed handles (center, two endpoints, and one curve handle per side)
and two hidden curve-time parameters. Audio evaluated one cubic/bezier-like
formula in `src/voice.rs::pan_shape_curve_value_side`. That model cannot add a
second interior point, cannot bend separate segments, and cannot represent the
independent curves the editor appeared to promise.

Polaar's implementation is materially different:

- `src/dsp/transformer_curve.rs` stores ordered knots with input, output,
  incoming bend, and outgoing bend values. Each polarity has its own half.
- A half supports up to 12 knots. Interior knots can be inserted, moved, or
  removed while endpoints remain protected.
- Runtime audio evaluates fixed compiled segments. The UI data is guarded by a
  normal lock, while an atomic generation/seqlock publishes fixed-size segment
  arrays to the real-time path.
- `src/ui/curve_transfer.rs::draw_bipolar_transformer_editor` resolves input
  in this order: Alt segment hit, anchor, knot, then empty-space insertion.
  Alt-drag bends the selected segment; normal drag moves a knot; right-click
  removes an interior knot; Shift snaps; Ctrl/Cmd-Z history is kept outside the
  audio callback.
- Rendering samples each segment at high resolution and fills the area with a
  transparent gradient, so a curve remains visible even at short ranges.

## Porting decision

KURV's Shape is a unipolar per-side pan/detune shape rather than Polaar's
bipolar transfer function. The correct port is therefore the interaction and
runtime model, not the polarity semantics:

- left and right halves use local input/output values in `[0, 1]`;
- local input `0` is the center and `1` is that side's edge;
- center and edge endpoints remain protected anchors;
- the default is exactly three shared anchors (center, left edge, right edge);
  empty-space clicks are what create additional knots;
- the editor now supports interior knot insertion, movement, right-click
  removal, Shift snapping, and Alt segment bending;
- `PanShapeCurveState` is a persisted Truce `#[persist]` field;
- audio consumes only the compiled fixed-size runtime snapshot, never the UI
  lock or a growing vector.

KURV's segment bend is intentionally a signed cubic control-point bulge. This
is a small product-specific improvement over Polaar's monotonic easing tension:
Alt-drag can visibly bow a segment up or down while the output remains bounded
to the Shape domain.

Legacy fixed parameters remain readable for old sessions until the new curve is
edited or loaded. They are used to seed a two-segment curve, while the new
persisted spline becomes authoritative after its first edit.

## Sources

- Polaar source (primary):
  `/mnt/Windows11/DEV_PROJECTS/Repos/asymmetry-rider/src/ui/curve_transfer.rs`
- Polaar source (primary):
  `/mnt/Windows11/DEV_PROJECTS/Repos/asymmetry-rider/src/dsp/transformer_curve.rs`
- KURV implementation: `src/pan_curve.rs`, `src/editor.rs`, `src/voice.rs`,
  and `src/lib.rs`.
- Truce GUI guide: https://truce.audio/docs/guide/gui/
- Truce parameters guide: https://truce.audio/docs/guide/parameters/
- Truce state guide: https://truce.audio/docs/guide/state/
- Truce hot reload guide: https://truce.audio/docs/guide/hot-reload/
- Truce 6.3.0 rustdoc: https://rustdoc.truce.audio/

The Truce documentation describes editor parameter binding, persisted custom
state, and the requirement that audio-thread code remain lock-free; the local
6.3.0 source is the authority for the exact derive and persistence behavior.
