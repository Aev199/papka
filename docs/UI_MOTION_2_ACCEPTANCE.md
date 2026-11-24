# UI & Motion 2.0 — Windows acceptance matrix

Status: planned, not executed. Compare the current candidate against Beta 105
on the same Windows computer with the same theme, scale and fixtures. Do not
publish a beta until critical cases pass and the user accepts the visual result.

## Fixtures

- Local folder: 20 files, several subfolders, grouped document versions.
- Large local folder: 5,000+ entries and a truncated listing if available.
- Network share: ordinary engineering project directory, plus slow/unavailable
  fixture only if safe; never use production data for destructive tests.
- Russian and long names, PDF/DOCX/XLSX, DWG, text (UTF-8 Cyrillic), images and
  intentionally unreadable sample files.
- Use disposable directories for rename/delete/copy/move testing.

## Acceptance checklist

| Area | Exercise | Expected |
| --- | --- | --- |
| Modes | List -> columns -> thumbnails -> list, rapidly reverse | Stable anchor/selection, no duplicate interactive rows, no blank flash or jumps |
| Modes | Scroll, resize, start selection or drag mid-transition | Correct hit targets, no stale departing shapes across pane bounds |
| Miller | Open/back/forward, preview child, deep paths | New columns appear, auto-scroll reveals intended column, wheel/scrollbar wins |
| Miller | Switch tabs, cover with project card, rapid close/reopen | No stale columns or delayed reveal in another tab |
| Drag | Single/multiple manual reorder, Escape, leave pane, Ctrl/Shift | Preview gap, correct return/landing, unchanged actual files; modifier transfer semantics |
| Drag | Actual transfer, conflict, failure, Windows Shell handoff | Existing confirmation/transfer behavior; success not shown for failed operations |
| Panels | Rapidly toggle sidebar/inspector, resize, reverse | Smooth bounded widths; closed controls cannot accept clicks |
| Inspector | Switch PDF/images/text rapidly; failed preview; select many | No stale image/text; stable placeholder; metadata/thumbnail arrival independent |
| Palettes | Type local path, reopen while scan running, arrows, Enter, Escape | Responsive input; selection identity retained; no retired input actions |
| Menus | Open at all window edges, small window, disabled commands | No clipped inaccessible commands; correct keyboard and pointer behavior |
| Status | Narrow window, long selection, filters, active transfer | View controls pinned; information and transfer controls remain reachable |
| Themes | Light/dark, seven accents, three row densities | Legible selected/hover/danger states; no changed logical row heights |
| DPI | 100%, 125%, 150%, 200%; resize between displays | Text/icon layout and hit targets stay aligned |
| Reduced motion | Disable animations during transition, repeat above | Immediate settling; no lingering animation frames |
| Network | Navigate cached/uncached UNC folder, preview, watcher refresh | No new synchronous paint-time I/O; refresh does not replay navigation |
| Workflows | Project card, staging, comparison, version groups, search | No feature regression |

## Performance recording

Run a normal session, then opt-in with PowerShell:
```powershell
$env:PAPKA_MOTION_LAB="1"
.\Papka.exe
```
Reset the diagnostic counters before each scenario. Record window dimensions,
DPI, refresh rate, directory size, cached/uncached state and approximate p95 UI
construction / render-call interval for repeated mode switches and drag reflow.
The diagnostic intervals exclude long idle gaps and are not GPU frame timings.
Use an external Windows presentation profiler if actual compositor frame timing
is required. Repeat with reduced motion. Measure baseline using equivalent
external tooling, since Beta 105 has no Motion Lab.

Target: no material regression in interaction latency or frame pacing versus
Beta 105. 16.7 ms at 60 Hz is a goal, not a measured result. Record observations
and severity rather than declaring acceptance on CI alone.
