# UI & Motion 2.0

Baseline: Beta 105 (4f48b139), merged into main at 930633c.
Branch: feat/ui-motion-2. No release-channel changes during development.

## Objectives and constraints
- A coherent native desktop design, not a clone of Atlas.
- Preserve keyboard operation, compact engineering-document lists, split panes,
  project cards and UNC/network workflows.
- No animation delays filesystem operations, blocks keyboard input, or turns a
  watcher refresh into a navigation transition. No synchronous disk access in paint.
- Reduced motion settles immediately. Interrupted animation preserves visual
  continuity; stale callbacks cannot mutate a different view.
- Use bounded visible snapshots, no per-frame unbounded folder scans.
- Validate dark/light themes, three densities, DPI 100/125/150/200%, resizing,
  long Russian names, selection, modal focus, keyboard and mouse interaction.
- Performance goals (not measured yet): 60 Hz frame budget 16.7 ms, p95 frame
  interval near refresh rate in cached folders, no extra animation frames at idle.
  Benchmark baseline and candidate on the same Windows machine and fixtures.
- After every stage: code review, CI and portable Windows test build; record
  manual findings and regressions. Publish beta only after acceptance.

## Roadmap
| Stage | Work | Acceptance |
| --- | --- | --- |
| 1 | Design tokens and shared UI primitives | Consistent control states and geometry; no unplanned density or hit-target change |
| 2 | Motion Lab and common motion lifecycle | Opt-in telemetry, slow motion, reset; disabled by default; no filesystem side effects |
| 3 | Spatial transitions | Matching file identity, size and position, paint-only departing items; cancel and retarget; unchanged scroll anchor |
| 4 | Navigation | Miller reveal interrupted by user scrolling; explicit navigation only; bounded entry stagger |
| 5 | Drag & Drop 2.0 | Copy/move vs manual order remain distinct; preview cancellation and target feedback; no hidden interactive items |
| 6 | Microinteractions | Cohesive selection, disclosures, controls and pane motion; reduced motion |
| 7 | Inspector and preview | Stable metadata hierarchy, async preview readiness, no stale results |
| 8 | Menus, palette, dialogs | Shared state/spacing, keyboard focus and immediate input cancellation |
| 9 | Whole-app polish | Consistency across workspace, themes, density, long paths and narrow panes |
| 10 | Final review and beta | CI, Windows manual matrix, measured performance and release notes |

## Stage 1: source audit and first migration
Existing strengths: semantic palette with seven accent schemes and contrast tests;
layout policy already owns row density and column geometry; shared icon and text
button builders already exist in view.rs. Preserve these instead of creating a
parallel design system.

Findings:
- view.rs contains 74 lines with rounded calls, 105 with text_size, 109 with gap,
  and 45 with border_color (simple line counts, not unique controls).
- Theme widget radius, shared toolbar/status/dialog button sizes, focus marker
  and icon sizes were independent literal dimensions.
- interface.rs, transfer_panel.rs and project_card.rs contain their own explicit
  corner sizes and row/control geometry. Audit intent before consolidating;
  equal numeric values alone do not imply equal semantics.
- Palette tests cover primary selection and basic secondary text; extend role
  coverage after visual review, not by guessing new colors.
- Existing spatial_motion records positions only; mode_motion still fades down
  to 0.12; use this as the baseline for Stage 3 rather than stacking more effects.
- No measured Windows frame-time baseline yet. Source review cannot establish
  actual perceived polish.

First slice: introduce ui_tokens.rs with shared control dimensions; project them
into shared builders and component theme without changing numeric values.
Windows verification is enabled for this branch. Next slice: audit and migrate
repeated menu/list/dialog roles and define typography/spacing after screenshots;
do not change file row height policy or user settings unintentionally.

### Stage 1 — typography and shared settings controls
- Added explicit caption, secondary, body, section and dialog title roles; migrated settings/palette, transfer panel, project card and shared menu rows. Document and preview fonts are not changed.
- Shared choice_chip handles settings choices and update-channel controls. All dimensions retain the baseline values; row density and hit areas remain unchanged.
- Checked settings now use the existing accessible SELECTED / SELECTION_TEXT pair instead of a bright accent with a white glyph (on dark teal/green/orange/graphite the previous contrast was about 2.1:1).
- Destructive menu hover uses theme-aware DANGER_SOFT (text contrast 5.06 dark / 4.81 light); test both themes. No changes to command semantics.
- CI required; Windows screenshots and manual acceptance still pending.

## Stage 2 — Motion Lab foundation
Opt in with PowerShell: `$env:PAPKA_MOTION_LAB="1"; .\\Papka.exe`.
The bottom bar displays p95 render-call intervals (idle gaps above 250 ms excluded)
and CPU UI-construction time, not compositor/GPU frame timing. Click to reset.
Rolling sample capacity 120. No background timer, no forced frames, no disk writes.
Optional `$env:PAPKA_MOTION_SLOW="4"` (1/2/4) at process start scales the shared
Tween/Spring and spatial FLIP durations. It is intentionally not global time dilation:
filesystem, drag dwell, scroll and mode-commit clocks keep real time. Default 1;
normal behavior unchanged. More motion clocks can migrate once paired expiration
and sampling paths are covered. Added standalone telemetry tests and exact
zero-time spring continuity. CI only, no intermediate portable builds per user.

## Stage 3 — spatial geometry, first slice
Spatial snapshots now store both origin and size and retarget from shown rectangles.
The live destination is clipped to an interpolated rectangle while its virtual list
slot remains fixed; this is a container morph, not a separate icon/text morph or
an outgoing-content snapshot. Measured destination bounds become available after
the first destination frame, so the existing mode fade stays for that handover.
Tests cover interpolated dimensions, interrupted transitions and stale callbacks.
Windows visual verification and paint-only departure remain outstanding.

## Stage 4 — Miller column reveal (first slice)
Replace the abrupt automatic reveal with a pane-local retained 180 ms scroll.
Retarget from actual offset, never from an obsolete destination. External offset
changes cancel the reveal; disabled animations settle immediately. Deferred
layout callbacks validate the pane path, tab and preview before touching scroll.
Prune state when a pane closes or switches presentation. Added independent tests.
Column appearance/exit snapshots and bounded item entrance stagger still pending.

## Stage 5 — drag presentation, first slice
Manual reorder has a visible paint-only insertion gap instead of invisible
selected tiles/rows. The ghost labels a valid same-folder preview as
"Переставить"; Ctrl/Shift and actual folder/staging targets retain copy/move
semantics. The label is derived from the current rendered preview, reset each
root render, and does not authorize a drop. Drop revalidation is unchanged.
Ghosts use generic file/stack icons and truncate long names without disk I/O.
Cancellation/landing effects and target-transition polish remain pending.

## Stage 6 — microinteractions, first slice
Disclosure height and chevron use the shared critically damped spring with a
bounded tail; clamp physical output to [0,1]. Closing disclosure bodies have
an input shield and fully closed bodies are no longer rendered. Choice chips
use the same retained hover background as toolbar controls. Hover transitions
now settle their internal endpoints and obey the diagnostic visual slowdown.
Selection plate and broader panel spring tuning remain outstanding.

## Stage 7 — inspector visual stability, first slice
Image/document preview has a fixed-size framed placeholder while the helper or
GPUI decoding is pending. Fade starts only after ImgResourceLoader returns
success, and request/path guards prevent stale results; failure is shown once
per request. Reset readiness on selection/refresh. Add file kind subtitle and
align inspector metadata labels. No new disk access in rendering; reduced-motion
pictures snap. Text-preview layout and full visual comparison remain pending.

## Stage 8 — menus, palette and dialogs (first slice)
Palette and folder jump lists fit the actual viewport height and use shared
geometry; context menu width and height are bounded. Keyboard navigation in
command mode skips disabled commands, query changes select the first available
command, disabled rows no longer attach click handlers. Prompt uses shared
panel/title tokens. Added pure viewport and keyboard selection tests.
Remaining: full small-window visual review, menu presence and content polish.

## Stage 9 — workspace-wide polish, first slice
Primary-action hover and drag action chip no longer use the decorative
selection-border color as a text background. The new semantic PRIMARY_HOVER
is derived from each scheme's accessible selection color; test all seven
schemes, light and dark. Centralized title/status/navigation and sidebar row
geometry without changing heights. Status-bar project and selection labels
can shrink and truncate instead of competing with the controls.
Actual appearance and DPI comparison require the deferred Windows manual pass.

### Stage 4 follow-up — bounded thumbnail entrance
The first eight thumbnail positions receive a subtle vertical and opacity
stagger during explicit folder entrance only. The tail ends inside the
existing 140 ms clock; no extra animation frame requests. Same-folder mode
changes, watcher refresh, rename, marquee, file drag and spatial transitions
skip the stagger. Test that delay and end time are bounded.
