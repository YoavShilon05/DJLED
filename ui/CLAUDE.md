# ui/ — React + TypeScript + Mantine

The editor. A pure WebSocket client of the engine: it can be opened, reloaded or
closed without the strip noticing, and it stays fully usable while disconnected —
you can author a whole stack with nothing running and it is sent when the engine
appears.

See the root `CLAUDE.md` for the Rust↔TS parity invariants; they are the ones
that make this directory dangerous.

## Commands

```bash
npm install
npm run dev         # vite on 5173
npm test            # vitest run — 86 tests, 9 files
npm run typecheck   # tsc --noEmit
npm run build       # typecheck + vite build
npm run smoke       # node scripts/stack-smoke.mjs, against a running engine
npm run preset-smoke  # node scripts/preset-smoke.mjs, same
```

No linter and no lint script, and no prettier config either. The files are
written at about 100 columns; running `npx prettier` on one reflows it at
prettier's default 80 and rewrites the whole file, which buries a real diff
exactly the way `cargo fmt` does on the Rust side. Match the surrounding style
by hand. `tsconfig.json` is strict, with `noUnusedLocals` and
`noUnusedParameters` on — an unused import fails the build, not just the
editor's squiggles.

## Layout

| Path | Responsibility |
|---|---|
| `App.tsx` | Wires the client to the panels. Owns `EditorConfig` state and the debounced send |
| `engine.ts` | `EngineClient` — socket, auto-reconnect, and the engine's wire types *verbatim* |
| `config/editor.ts` | `EditorConfig`, defaults, localStorage (`djled.editor`), `toEngineConfig` / `fromEngineConfig`, and `isStatic` / `toRenderSurface` |
| `config/scales.ts` | The two axes. Every gizmo, tick and hit test goes through here |
| `config/eq.ts` | RBJ cookbook biquads, verbatim |
| `config/curve.ts` | Intensity curve — cubic Béziers pinned at (0,0)–(1,1) |
| `config/notes.ts` | MIDI note ↔ position on the frequency axis, and the relabelled ticks |
| `config/presets.ts` | Slot ↔ function key, and the preset bar's labels. The *only* place 0-based slots meet 1-based keys |
| `color/oklab.ts` | Port of `engine/src/color/oklab.rs` |
| `color/surface.ts` | Port of `engine/src/color/surface.rs` |
| `color/display.ts` | Screen-side conversion. **Not** the same as the LED path |
| `color/reference.json` | Generated from the Rust — do not hand-edit |
| `spectrum/layout.ts` | Pixel geometry for the whole editor block, one coordinate space |
| `spectrum/paint.ts` | Canvas raster: colour field, grid, bars. Pure functions, no React |
| `spectrum/GizmoLayer.tsx` | SVG interaction layer over the canvas |
| `spectrum/render.ts` | Port of the compositing fold in `engine/src/color/render.rs` |
| `components/` | Mantine panels — see the shell below |
| `components/strip.ts` | `paintStrip`, shared by the preview strips and the row thumbnails |
| `theme.ts` | Every colour, radius and spacing decision, including `theme.other.plot` |
| `styles.css` | Two rules. Keep it that way |

### The shell

`App.tsx` lays the editor out as one screen with no page scroll. Four blocks are
pinned and two columns scroll under them:

```
HeaderBar      status · master brightness · the ⋯ menu (Reset lives here)
PresetBar      all twelve shows, always visible; click the live one to rename
StripPreview   Preview over Engine — the wall, and what it is really doing
────────────────────────────────────────────────────────────────
LayerStack     │  SpectrumEditor + OverlayMenu
Inspector      │
 (scrolls)     │   (scrolls)
```

The pinned half is the point: the preview strip is the only ground truth in the
application and it used to be at the bottom of a tall document. Anything added
to that half costs the graph the same height on every screen, so it is a
decision rather than a placement.

`Inspector` is a tabbed box — Source, Shape, Notes — over the *selected* layer.
Those three were separate panels, each with its own border and its own heading
repeating the layer's name; the MIDI one was on screen at full height for audio
layers purely to say that it did nothing. The tab persists across selecting a
different row, which is what makes comparing one setting across a stack a matter
of clicking rows.

Long explanations live behind `Field`'s and `PanelHeading`'s `info` prop — an
`InfoDot`, hoverable and focusable — rather than under the control. The prose is
worth keeping and was what made the column unreadable; `hint` is for the one
line worth having on screen permanently.

## Rules that are easy to break

- **`config/editor.ts` is the only place the wire shape is known.** A protocol
  change lands in `toEngineConfig` / `fromEngineConfig`, never in a component.
- **`color/surface.ts`, `color/oklab.ts` and `spectrum/render.ts` must stay
  numerically identical to their Rust originals.** If they drift, the Preview
  strip lies about the wall. After touching either side, regenerate
  `color/reference.json` (command in the root `CLAUDE.md`) and run **both**
  suites.
- **Styling is theme-only.** No `classNames`, no `styles` props, no per-component
  CSS. Everything is a Mantine theme token or a `defaultProps` entry, and the
  canvas palette lives on `theme.other.plot` so drawn pixels and rendered
  components resolve to one source of truth.
- **`display.ts` is not the LED path.** `oklabToLedBytes` is linear-light because
  that is what WS2812 wants; painting those bytes into a canvas is a category
  error, since the browser reads them as sRGB. Use `oklabToDisplay` (composites
  onto black — an unlit LED) or `oklabToDisplayRgba` (keeps opacity as opacity,
  for the colour field).
- **All geometry goes through `config/scales.ts` and `spectrum/layout.ts`.** A
  keyframe at 250 Hz lands on the 250 Hz grid line by construction, not because
  two call sites agree.
- **The axis never changes; only its labels do.** MIDI mode swaps the tick scale
  in `spectrum/axis.ts` — everything else stays written in Hz.
- **A layer listening to nothing has no vertical axis, and the collapse lives in
  two functions.** `computeLayout(w, h, flat)` sets `layout.flat`; `yOfDb`
  returns the lane's centre for every dB and `dbOfY` returns a constant. That is
  deliberate and is the whole trick: every gizmo, hit test, drag offset and
  popover anchor already goes through those two, so a flat plot places all of
  them correctly without any of them knowing there is a second kind of plot. The
  day one starts doing its own arithmetic is the day handles drift off the lane
  — `layout.test.ts` is there to fail first.
- **Drag discards the vertical on a flat plot rather than writing `dbOfY` back.**
  The stored dB is still what crosses the wire and still what a preset keeps, so
  nudging a still layer's colours sideways must not flatten the field it was
  authored with. `toRenderSurface` is the read-side projection; `toSurface` is
  what is sent.
- **A still layer's gizmos are absent, not hidden.** `GizmoLayer` drops the
  thresholds, the curve, the EQ and the dB labels when `layout.flat`. That does
  not contradict the rule below about visibility: the overlay checkboxes say
  what to draw of what a layer *has*, and a still layer has none of these.
- **The plot draws one layer, the strip preview draws the stack.** Six sets of
  gizmos on one graph would be unclickable, which is why the active layer is a
  first-class piece of editor state.
- **The whole config goes over the wire on every pointer move.** That's
  deliberate — one message means the two sides can't disagree about which half
  of an edit landed. Keep `toEngineConfig` cheap and allocation-light.
- **Frames are dropped, not queued, and almost nothing should re-render on
  one.** The engine publishes thirty times a second whether anyone keeps up or
  not, and a WebSocket has no back pressure — so `EngineClient` parks the newest
  frame and hands it over on the next animation frame, and everything not driven
  by a frame is memoised with referentially stable props. Without the first, a
  tab that cannot keep up banks the backlog until it is killed for running out
  of memory; without the second, it cannot keep up. Three things are on the
  React frame path on purpose: `SpectrumCanvas` and the two `StripPreview`s.
  Adding a fourth is a decision, not an accident.
- **A layer row's thumbnail is pulled, not passed.** `LayerThumb` looks the
  latest strip up in a ref that `App` refreshes per frame, and paints it on its
  own timer at a third of the rate. Handing each row a fresh array of colours
  instead would re-render twelve memoised Mantine panels thirty times a second
  to repaint twelve canvases — which is the rule above, broken in the one place
  it looks unavoidable. The same trick is what any future live readout beside a
  control should use.
- **Thumbnails ignore layer opacity and master brightness.** A deliberate
  disagreement with the wall: they answer "which layer is this", and a row
  faded to 10% or a master pulled down for a quiet passage would make every one
  of them an identical black rectangle. How much of a layer is getting through
  is what its slider and its dimmed row already say.
- **Do not reallocate a canvas backing store per frame.** Assigning to
  `canvas.width` or `.height` reallocates even when the value is unchanged.
  Guard it with a size comparison, as `StripPreview` and `SpectrumCanvas` both
  do.
- **The engine owns the presets, so it owns the show while it is connected.**
  `App.tsx` adopts `state.config` whenever `state.activePreset` changes and
  ignores it otherwise — a hotkey can swap the show with this page closed, and
  that field is the only signal it happened. localStorage is now the *offline*
  cache: it seeds an engine whose live slot has never been authored into, and
  is re-asserted after a reconnect, and is otherwise deferred to.
- **`PresetBar` selects, it does not load.** `selectPreset` sends a slot and
  nothing else; the new show arrives in the next `state`, on the same path a
  global hotkey takes. Sending a config with it would give the bar and the
  keyboard two different ways to disagree. Rename is the one thing the bar does
  send directly, and it is committed on blur or Enter rather than per keystroke.
- **Gizmo checkboxes are visibility, not bypass.** Nothing in `OverlayMenu`
  touches the signal.
- `MAX_LAYERS` is 12 and `SAMPLE_LENGTHS` is a fixed list, both in
  `config/editor.ts`.

## Tests

| File | What it defends |
|---|---|
| `color/reference.test.ts` | Parity with the Rust colour path, against the generated fixture |
| `color/oklab.test.ts` | The conversions on their own |
| `spectrum/stack.test.ts` | The browser's copy of the compositing fold, and that `renderLayer` and a one-layer `renderStack` are the same bytes |
| `spectrum/render.test.ts` | Spatial mapping — including that an even-length strip's mirror fold lands on one specific index |
| `spectrum/layout.test.ts` | The plot's geometry, and that a flat plot collapses the dB axis in `yOfDb` / `dbOfY` and nowhere else |
| `config/eq.test.ts` | Closed-form properties, not captured values |
| `config/editor.test.ts`, `config/notes.test.ts` | Round-tripping and the note axis |
| `config/presets.test.ts` | Slot ↔ function key numbering, and that the bar lists all twelve in hotkey order |

`scripts/stack-smoke.mjs` and `scripts/preset-smoke.mjs` are the only checks
that cross the real socket. The first sends a two-layer show to a running engine
and reports the state echo, the per-layer frames and the composited strip. The
second authors two presets, switches between them and confirms each comes back
intact. Start the engine first.

The hotkeys themselves cannot be checked from either — they are registered with
Windows, so pressing them is the test. What `preset-smoke` covers is the switch
they trigger, which is the same code path.
