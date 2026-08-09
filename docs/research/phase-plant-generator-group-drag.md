# Phase Plant generator grouping and drag/drop

Research date: 2026-08-10. Sources are limited to first-party Kilohearts
documentation, changelogs, and Kilohearts videos.

## Implementation answer

Phase Plant's public contract is a one-level generator hierarchy: the generator
lane contains groups, and groups contain generator-area modules. A generator
cannot exist outside a group, group headers break automatic audio flow, and a
group moves as one unit. Nested generator groups are not a supported target.

The reported **Alt-drag to put an oscillator/group inside the hovered section**
conflates separate behaviors:

- Dragging reorders or moves a module; dragging a group moves the whole group.
- On Windows, **Alt** exposes or enlarges the narrow insertion area between
  existing items. On macOS the documented equivalent is **Command**. The docs
  describe this as easier access to the same between-item slot, not as a
  nesting or copy mode.
- On Windows, **Ctrl** is the copy modifier while dragging. Kilohearts' generic
  host documentation says the macOS copy modifier is **Alt/Option**, although
  the Phase Plant-specific page instead says Command. That macOS contradiction
  should not be silently resolved in KURV.

For KURV, use distinct drop targets:

```text
TopLevelGap(group_index)          // groups only
GroupGap(group_id, module_index)  // generator-area modules only
```

Alt should only make the already-valid gap target easier to hit. It may expose
a `TopLevelGap` or `GroupGap` according to the hovered container, but it should
not change the payload type, copy the payload, or allow `Group -> GroupGap`.
Keep Ctrl as copy on Windows/Linux. A group can only land on `TopLevelGap`.

## Documented facts

### Hierarchy and signal flow

- The first generator added to an empty generator area automatically creates a
  group and an output module. The area has a 32-module limit.
- A generator module cannot be placed outside a group.
- A group header breaks the stack's automatic routing; audio never flows
  automatically between groups. Cross-group audio must be routed explicitly.
- Groups may be renamed, collapsed, moved, and copied as whole units.

Sources: [Phase Plant manual: Generator Area](https://kilohearts.com/docs/phase_plant#generator_area)
and [Generator Groups](https://kilohearts.com/docs/phase_plant#generator_groups).

The [2.2.5 changelog](https://kilohearts.com/changelog#2.2.5) says Kilohearts
fixed a bug that allowed a group to be added inside another group through
search. This is strong first-party evidence that group nesting is invalid, not
a hidden Alt-drop feature.

### Movement, copying, and modifiers

- Modules are reordered or moved between compatible lanes by dragging their
  title bars.
- Kilohearts' host documentation maps copy-on-drop to `Ctrl` on Windows and
  `Alt/Option` on macOS. It applies the same copy operation to groups and their
  contents.
- The current Phase Plant page instead says `Ctrl/Command` for copying modules
  and groups. The two official pages therefore disagree for macOS.
- Windows behavior is corroborated by Kilohearts' own UI in the official
  beginner video: its status bar says to hold Ctrl while dragging a generator
  to duplicate it.
- The [2.3.0 changelog](https://kilohearts.com/changelog#2.3.0) also describes
  `ctrl+click/alt+click` as the Windows/macOS copy gestures in the generator
  section.

Sources: [Host Plugins: Lanes and Groups](https://kilohearts.com/docs/host_plugins#lanes),
[Phase Plant: Generator Area and Groups](https://kilohearts.com/docs/phase_plant#generator_area),
and [Kilohearts Beginner's Guide: Start Here, 00:33](https://www.youtube.com/watch?v=ADISX6oSkSs&t=33s).

### Drop zones and Alt

Kilohearts documents two insertion zones:

1. Empty lane space exposes a large add-module area on hover.
2. A gap between existing modules exposes a dashed insertion line on hover.

Holding **Alt on Windows** or **Command on macOS** expands the between-module
area for easier access. The original [1.7.5 changelog](https://kilohearts.com/changelog#1.7.5)
describes the feature as adding modules in the middle of a lane; the current
[Host Plugins documentation](https://kilohearts.com/docs/host_plugins#lanes)
describes it as expanding the insertion area. Neither source says Alt changes
which container owns a dragged item.

While dragging, Kilohearts keeps a drop area visible and scrolls a lane when
the pointer approaches its edge. These are explicit [2.0.8 changelog](https://kilohearts.com/changelog#2.0.8)
behaviors.

### Visual feedback

- Normal hover feedback is a blue dashed rectangle for a large empty insertion
  area and a dashed line for an interior gap. Kilohearts calls these "blue
  areas" in its official beginner video.
- The drop placeholder remains visible throughout a drag, and edge hover can
  auto-scroll the lane.
- A full target may show `lane is full`; Kilohearts references that drag state
  in the [2.3.1 changelog](https://kilohearts.com/changelog#2.3.1).
- A generator-area processor placed without required upstream input shows a red
  arrow after placement. That is a signal-flow warning, not a drop target.

Sources: [Start Here, 00:33](https://www.youtube.com/watch?v=ADISX6oSkSs&t=33s),
[Host Plugins: Lanes](https://kilohearts.com/docs/host_plugins#lanes), and
[Phase Plant: Generator Area](https://kilohearts.com/docs/phase_plant#generator_area).
The video visually shows Phase Plant 2.1.3, so it is evidence for the basic
blue/dashed language, not proof of every current animation or hitbox dimension.

## Explicitly not verified by public first-party material

Kilohearts' public docs and official beginner videos do **not** explicitly state
what happens when an existing generator is dragged to a top-level gap outside
all current groups. Two facts constrain the result—generators cannot remain
ungrouped, and adding the first generator auto-creates a group—but they do not
prove whether such a drop is rejected or automatically wrapped in a new group.

If KURV adopts **drop outside group -> create a new singleton group**, label it
as a product decision inferred from those invariants, not as verified Phase
Plant parity. The same inference must not be extended to groups: a group stays
top-level and cannot be nested in the hovered group.

## Minimal KURV behavior to implement

- Render a stable placeholder for the resolved drop target before mouse-up.
- Use a large dashed target in empty space and a dashed gap marker between
  items; Alt widens/reveals the gap hit region without changing the target.
- Move by default; Ctrl copies on Windows/Linux.
- Permit module-to-module-gap and group-to-top-level-gap only.
- Reject group nesting visibly and leave the document unchanged.
- If desired, auto-wrap a module dropped on a top-level gap in a new group, but
  keep that as an explicit KURV rule because Kilohearts does not document it.
