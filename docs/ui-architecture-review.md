# KURV editor architecture review

`src/editor.rs` is now a 307-line Truce boundary. Its only external API remains
`pub fn create(Arc<KurvParams>) -> Box<dyn Editor>`; visual implementations live in
focused sibling modules. The current screenshot evidence is
`target/screenshots/kurv-modular-final.png`.

## Implemented shape

`src/editor.rs` owns the Truce boundary, panel composition, factory reset, and output
meter. `src/editor_shell.rs` owns the responsive shell and header. No parameter
wrapper, new parameter type, or ID mapping was introduced: modules continue to call
`begin_edit`, `set_param`, and `end_edit` with the existing `P` values.

| Module | Extract current group | Minimal interface | Dependencies |
| --- | --- | --- | --- |
| `shell` / `layout` | `src/editor_shell.rs` | `draw(ui, state)` | panel boundary and `output_meter` |
| `theme` / `widgets` | `src/editor_theme.rs`, `src/editor_widgets.rs` | panel frame, header, status, gradient | egui only |
| `controls` | `src/editor_controls.rs` | `param_knob`, `param_box`, drag/default gestures | Truce parameter metadata |
| `oscillator` | `src/editor_oscillator.rs` | `waveform_view` | engine `Antialiasing`, `sample_shape_with_antialiasing` |
| `unison_shaper` | `src/editor_unison.rs` | distribution, stereo triangle, direct point shaper | `voice` preview helpers; `pan_shape_settings`; `pan_curve` |
| `envelope` | `src/editor_envelope.rs` | `envelope_view` | existing `P` IDs and shared curve geometry |

`KnobDrag`, `DragAxis`, `update_parameter_drag`, `accumulate_drag`, and
`magnetic_shape_snap` live in `editor_controls`. `PanShapeHistory` and
`PanShapePointDrag` live in `editor_unison` because they are local UI state,
not reusable curve-model state.

## Dependency seams worth preserving

- The editor is read/write only through `PluginContext`; `P` remains the single
  automation identity. Never replace IDs 0-45, including hidden legacy IDs.
- `PanShapeCurveState` is the owned/persisted seam: UI edits call `edit`/`replace`;
  audio reads `segments_rt`. Keep its lock/seqlock implementation in `pan_curve.rs`.
  The UI module should not compile or publish its own runtime segments.
- Unison previews intentionally share `voice` math (`*_seeded` helpers) and live
  meters (`stereo_seed`, `swarm_phase`). Retain those calls rather than duplicating
  positioning math in UI.
- `create` must retain the current `EguiEditor` construction, visuals, sizing, and
  callback signature. Submodules draw within that callback; none creates an editor
  or owns a Truce lifecycle.

The split keeps parameter persistence, host gestures, and editor construction
untouched while making the visual modules independently readable.

## Current UX / architecture defects

- Factory reset is now restricted to the KURV header response; control double-click
  defaults no longer share a global reset gesture.
- The envelope is now explicitly labeled `ENVELOPE & EXPRESSION` and uses the full
  width below the oscillator/unison row; the empty filter card was removed because
  no filter DSP exists in the current parameter contract.
- The output meter has no visible `OUTPUT` label (400-489); its value is shown, but
  the horizontal drag target is not discoverable from the header.
- Waveform preview rebuilds 1,025 points plus a per-segment gradient mesh every
  frame (943-990). It is correct but unnecessarily allocates while idle; cache the
  preview geometry keyed by shape, PWM, quality, AA mode, and rect size only if
  profiling shows editor-frame pressure.
- Pan-shape undo/redo is temporary egui UI state (1579-1595), so it disappears when
  the editor closes and does not use host gesture/undo boundaries. Keep it local
  unless host-visible undo for custom persisted state is explicitly required.
