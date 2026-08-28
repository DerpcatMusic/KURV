---
target: KURV drastic masthead reconstruction
total_score: 22
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 4
timestamp: 2026-08-28T11-16-22Z
slug: src-editor-shell-header-rs
---
Method: dual-agent (A: design_review · B: detector_evidence)

# KURV masthead critique

## Design Health Score

| # | Heuristic | Score | Key issue |
|---|---|---:|---|
| 1 | Visibility of system status | 3/4 | State is visible, but preset save success/error feedback is weak. |
| 2 | Match system / real world | 2/4 | Familiar wheels are rendered too small; abbreviations and routing dots require expertise. |
| 3 | User control and freedom | 3/4 | History, preset stepping, direct manipulation, and resets are present. |
| 4 | Consistency and standards | 2/4 | Brand, scope, utility icons, wheels, XY, and readout grid use incompatible scales and materials. |
| 5 | Error prevention | 3/4 | Save Default is too close to routine save despite having a different consequence. |
| 6 | Recognition rather than recall | 2/4 | Tiny icon actions and abbreviated fields depend on tooltips. |
| 7 | Flexibility and efficiency | 3/4 | Direct controls and keyboard history work well for experts. |
| 8 | Aesthetic and minimalist design | 2/4 | Strong concept, fragmented hierarchy, excessive micro-information. |
| 9 | Error recovery | 1/4 | Preset errors are reduced to a tiny hover-dependent dot. |
| 10 | Help and documentation | 1/4 | Tooltips carry nearly all explanation. |
| **Total** | | **22/40** | **Acceptable foundation; substantial reconstruction needed.** |

## Design Specificity Verdict

KURV is recognizable, but the craft does not yet match the specificity. The orange field, vertical version rail, Matari mark, large wordmark, and DSP-derived scope are authored for this instrument. The center and right then collapse into generic egui controls and engineering telemetry. It feels like a confident poster containing a compressed settings panel.

The deterministic scan returned zero findings because it is web-oriented and does not meaningfully parse Rust/egui. Manual source and raster evidence remains authoritative.

## Overall Impression

The three-zone concept is correct: brand, signal deck, performance. The failure is inside those zones. The scope loses height to an orphaned utility row; the preset picker covers the signal it names; and twelve controls are forced into a right-hand region comparable to the four-control mock.

## What Pure SUB Gets Right

- Utility and preset management live in a separate thin dark toolbar, leaving the bright hero panel musical rather than administrative.
- The bright panel is divided into three large masses: brand, visual processor, and a repeated 3×2 knob bank.
- One light shell color and one near-black ink dominate. Purple is reserved for signal data instead of becoming general text chrome.
- Repetition creates professionalism: six controls share one size, label grammar, arc style, and spacing.
- The panel has strong bookends, deliberate diagonal shoulders, and enough negative space for each control to read as physical.

## Priority Issues

### [P1] The preset picker consumes the signal deck

**Why it matters:** Source gives the picker 46% of the scope width, roughly 230 px for “Init.” Its gray rectangle becomes the center’s dominant object and turns the waveform into wallpaper.

**Fix:** Make the picker a compact dark overlay sized from text plus padding, capped at 25–30% of the scope. Keep at least 70% of the waveform unobstructed.

**Suggested command:** `$impeccable layout`

### [P1] The scope is shortened for an orphaned action row

**Why it matters:** The scope receives exactly 72% of the center height. Four ~23 px actions are then centered into a remaining strip only ~22 px high, producing a structurally cramped footer.

**Fix:** Give the scope 90–94% of the header height. Replace the footer with two vertical side banks: previous/undo/redo on the left; next/save/save-as on the right. Remove Save Default from permanent chrome and place it in the save menu or settings.

**Suggested command:** `$impeccable layout`

### [P1] The right side exposes three times too many controls

**Why it matters:** Two wheels, XY, and nine readouts create twelve primary targets inside approximately 335×74 px. The mock uses four; Pure SUB uses six in a hero panel almost twice as tall. KURV’s controls therefore become toy-like and unreadable.

**Fix:** Build one performance pod containing two wider/simple rails, one usable XY pad, and four primary fields: Voices, Glide, Semi, Octave. Put PB/MPE ranges and velocity/pressure/timbre amounts behind a performance disclosure.

**Suggested command:** `$impeccable distill`

### [P1] The masthead has two competing color authorities

**Why it matters:** Orange is fixed at `#FF6000`, while wheels and fields consume configurable semantic cyan/lime/purple tokens intended for the dark editor body. The result is a parts-bin effect and theme changes cannot reconcile it.

**Fix:** Give the masthead its own strict material roles: orange shell, near-black ink/wells, off-white values inside dark wells, and orange/white indicators. Keep semantic source colors only on tiny routing jacks and active drag feedback.

**Suggested command:** `$impeccable colorize`

### [P2] Relative scale is inverted

The wordmark is huge, important values are microscopic, the scope is broad but shallow, and the picker is enormous despite containing one word. Establish four levels: brand/scope, preset/live values, utility actions, disclosed advanced settings.

## Recommended Reconstruction

- Brand: 24% width. Keep the wordmark, version rail, and Matari mark. Integrate settings into the rail or a deliberate corner cell.
- Signal deck: 48% width. Reserve 28–30 px action banks on each side and let the black scope occupy nearly the full height between them.
- Preset identity: compact overlay inside the scope; measured text width plus padding; 30% hard cap.
- Performance: 28% width. One coherent pod with two wheels, XY, and a 2×2 primary field grid.
- Secondary performance preferences: one disclosure/popover, not permanent header telemetry.
- Color: black-on-orange outside wells; neutral-on-black inside wells; one signal accent only.
- No fake macro knobs: KURV does not currently have a macro system, so Pure SUB’s repeated-knob principle should be copied through real performance controls, not decorative placeholders.

## Persona Red Flags

**Alex, power user:** Save Default and Save As are icon twins; the oversized picker masks useful signal; tiny wheel/XY targets slow repeated interaction.

**Jordan, first-timer:** The scope, picker, arrows, and detached history/save row do not read as one preset workflow. `PB RANGE`, `MPE RANGE`, and the source dots require learned interpretation.

**Sam, low-vision user:** The 3×3 field grid is too small, wheel ribs are low-contrast, and cyan/purple on saturated orange creates chromatic vibration.

## Minor Observations

- The current center width is already close to both mocks; the problem is internal composition, not total allocation.
- The settings button is stranded under the wordmark.
- The waveform has generous internal padding despite already being vertically constrained.
- The two wheel labels are visually wider than their controls.
- Pure SUB places pitch/mod wheels in its lower performance area, not its bright hero header; keeping them in KURV’s masthead requires a deliberately redesigned hardware pod.

## Questions to Consider

- Is this masthead primarily a preset browser or a complete performance settings panel?
- Which four performance controls genuinely need permanent visibility?
- Does Save Default deserve permanent visual status?
