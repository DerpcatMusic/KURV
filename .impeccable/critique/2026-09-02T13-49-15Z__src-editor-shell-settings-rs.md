---
target: KURV settings panel
total_score: 20
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 3
timestamp: 2026-09-02T13-49-15Z
slug: src-editor-shell-settings-rs
---
# Settings panel critique

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 2/4 | License and calibration status exist; most changes have no clear saved/applied state. |
| 2 | Match System / Real World | 2/4 | “RICH,” “Split density,” and “FAST/EXACT” require domain knowledge. |
| 3 | User Control and Freedom | 3/4 | Escape, close, and outside-click dismissal work; theme edits lack an obvious reset/cancel path. |
| 4 | Consistency and Standards | 2/4 | Shared tokens are strong, but labels and control layouts vary. |
| 5 | Error Prevention | 2/4 | Ranges and disabled calibration help; theme experimentation is not obviously reversible. |
| 6 | Recognition Rather Than Recall | 2/4 | Quality selectors lack visible labels and explanations are hidden in tooltips. |
| 7 | Flexibility and Efficiency | 2/4 | No section navigation, scale presets, or quick path for frequent users. |
| 8 | Aesthetic and Minimalist Design | 2/4 | Restrained visual language, but a content-heavy scroll-dependent modal. |
| 9 | Error Recovery | 2/4 | Errors surface; there is no unified reset or revert model. |
| 10 | Help and Documentation | 1/4 | Tooltips exist, but technical terms remain unexplained in primary UI. |
| **Total** | | **20/40** | **Poor; information architecture needs major reduction.** |

## Design Specificity Verdict

KURV’s palette, semantic roles, compact typography, and tight geometry are product-specific. The settings information architecture is generic: one tall inspector containing unrelated licensing, theme authoring, layout, modulation, quality, and CPU concerns.

The deterministic UI detector returned `[]` because this is native Rust/egui source rather than markup. Browser visualization and overlay injection were skipped because there is no browser-renderable target; the supplied native screenshot was used for visual evidence.

## Overall Impression

The controls are usable and the visual tokens are coherent, but the panel reads like an implementation inventory. The single biggest opportunity is to replace the linear scroll with category-first settings and expose scale immediately.

## What’s Working

- `editor_theme.rs` provides semantic palette, spacing, typography, and state visuals.
- The modal supports close, Escape, and outside-click dismissal.
- Quality settings use bounded existing selectors rather than speculative controls.

## Priority Issues

### [P1] Scroll-first settings architecture

The panel is capped at 72% of workspace height and always renders a vertical `ScrollArea` (`src/editor_shell/settings.rs:85-165`). Users scan unrelated categories and must scroll for basic preferences. Use a compact category rail or tabs; keep scrolling only inside a category when the host window is genuinely constrained.

### [P1] Missing user-controlled 100/125% UI scale

`ui_scale` is persisted but has no reader, writer, or UI consumer (`src/params/editor_state.rs:22-64`). `set_scale_factor` only forwards host/native scale (`src/editor.rs:123-128`). Add a persisted Interface scale control for 100%, 125%, and later 150% if needed; compose it with native DPI rather than replacing host scale.

### [P1] Verbose, jargon-heavy labels

“Pitch curve density and RICH reconstruction hop. Playback looks the curve up.” and the FAST/EXACT explanation add implementation detail to the primary scan (`src/editor_shell/settings.rs:448-504`). Use short labels such as `Layout density`, `Modulation quality`, and `Resynth quality`; keep technical detail in tooltips. Add visible `Anti-aliasing` and `Oversampling` labels.

### [P2] Incoherent save and recovery model

Theme edits autosave after a delay while other settings update directly, with no shared applied/saved state or reset model (`src/editor_shell/settings.rs:38-60`, `224-229`). Add a compact status and theme reset/revert behavior, then make close flush pending theme work.

## Lifecycle Findings

- Theme migration updates schema and several colors but does not persist migrated primary RGB data (`src/editor_shell/settings/theme_state.rs:13-32`).
- Delayed theme-library writes can be lost on host-driven close because `ThemeUi` is egui temp state and `prepare_to_close` cannot flush it (`src/editor_shell/settings.rs:38-60`, `src/editor.rs:52-59`).
- `save_as` changes active memory before the write succeeds, so a failed write can leave memory and disk divergent (`src/editor_theme/library.rs:75-84`).
- License access has change listeners, but KURV does not expose or subscribe a UI repaint adapter; async status can remain stale without another repaint (`../derpcat-access/src/access.rs:372-378`, `src/licensing.rs:69-100`).
- Startup license-load failure is ignored and the access worker can treat load failure as anonymous access; product policy needs a decision before changing this (`src/shell.rs:15-18`, `../derpcat-access/src/access.rs:122-138`).
- State revision notification is centralized around pointer release and structural commits; direct custom-state writers rely on those paths. This is a risk to verify, not a confirmed bug (`src/editor.rs:26-37`, `src/editor_shell.rs:219-230`).

## Persona Red Flags

- **Alex, power user:** scrolls through unrelated license/theme content to reach quality controls; no scale shortcut or category jump; unlabeled quality selectors slow recognition.
- **Jordan, first-timer:** meets license and RGB editing before basic interface preferences; `RICH`, `FAST`, and `Split density` are not self-explanatory.
- **Sam, high-DPI/laptop user:** host DPI is handled, but there is no user-facing 100/125% preference; resizing is an accessibility workaround.

## Questions to Consider

- Should Settings open directly to Interface, since scale and theme are the most immediate accessibility needs?
- Can every visible setting be explained in five words or fewer, with technical detail moved to a tooltip?
