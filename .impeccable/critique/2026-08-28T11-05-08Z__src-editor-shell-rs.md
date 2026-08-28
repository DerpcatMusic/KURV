---
target: KURV native editor group and masthead
total_score: 18
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 4
timestamp: 2026-08-28T11-05-08Z
slug: src-editor-shell-rs
---
# KURV native editor — Impeccable critique

## Method

- Visual review: the supplied Pure Sub/Phase Plant references, the prior KURV capture, and the final one-pass native render.
- Independent assessment A: heuristic, specificity, cognitive-load, and persona review.
- Independent assessment B: source tracing and PNG pixel evidence.
- Detector: `[]`; this web-oriented detector does not meaningfully cover Rust/egui, so zero findings were not treated as a clean bill of health.
- Browser inspection was inapplicable because this is a native host-embedded egui surface with no DOM.

## Pre-rework score

18/40 across the ten Nielsen heuristics: visibility 2, real-world match 2, control 3, consistency 1, prevention 3, recognition 1, efficiency 3, aesthetic minimalism 1, recovery 1, help 1.

## Specificity

The direction was visually specific but internally incoherent. The masthead, group strip, oscillator body, and output controls looked like separate redesign passes rather than one instrument. The strongest original decisions were the graph-first editing surfaces, large preset waveform, and clear generator/modulator split.

## Priority findings and remediation

### P1 — Blank icon controls

Phosphor was installed from inside the first UI pass, while egui activates new font definitions on the next pass. The screenshot renderer runs one pass, so the group toggle, settings, save, preset, and power affordances appeared blank. Fonts now load in the shared live/headless editor initialization seam before rendering. The Matari texture cache also no longer takes a re-entrant egui data lock.

### P1 — Group did not read as one container

Only the top edge used the group accent; the sides and bottom remained gray. The result looked like a colored toolbar placed above an unrelated oscillator card. The card now has accent side rails that fade from the header into a restrained bottom edge, preserving containment without a glow.

### P1 — Group identity and toggle were ambiguous

The dark square at the top left was the collapse button with its delayed glyph missing. It now shows a Phosphor minus when expanded and plus when collapsed. The identity remains a solid group-color tab with the softened shoulder flowing into the dim accent control field.

### P1 — Gain, Pan, and Route used conflicting visual grammars

Gain and Pan were unfilled centered readouts with unequal widths, while Route was an inset filled button with different vertical anchors. They now share equal cells, one tonal output deck, common label/value layout, and subtle dividers. The envelope power control sits at the envelope/output boundary.

## Remaining P2 observations

- The masthead performance metrics are intentionally dense at the minimum editor size; they should be judged in a live host at common scale factors before adding more information.
- Orange, group lime, oscillator cyan, and modulation purple are still a loud palette, but their ownership is now explicit: shell, group, oscillator, and modulation.
- Help and error-recovery remain mostly tooltip-driven; no new onboarding or documentation UI was added because it was outside this visual rework.

## Persona check

- Alex, experienced sound designer: stronger scan path and faster group/output recognition.
- Jordan, newer producer: visible symbols and connected containment reduce guessing.
- Sam, keyboard/automation-heavy user: existing focus, automation, and compact-control behavior remains intact.

## Run notes

- Final native one-pass render: `/tmp/kurv-brutal-rework.png`.
- Detached snapshot build passed; no tests were added or run.
- The live dirty checkout's full build is independently blocked by in-progress SVF DSP symbols, so validation used a detached HEAD snapshot with only UI files overlaid.
- Final screenshot review found no P0 or P1 visual blocker.

Questions skipped: the user already specified the target and requested implementation.
