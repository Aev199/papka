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
