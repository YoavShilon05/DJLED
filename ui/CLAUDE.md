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
npm test            # vitest run — 59 tests, 7 files
npm run typecheck   # tsc --noEmit
npm run build       # typecheck + vite build
npm run smoke       # node scripts/stack-smoke.mjs, against a running engine
```

No linter and no lint script. `tsconfig.json` is strict, with `noUnusedLocals`
and `noUnusedParameters` on — an unused import fails the build, not just the
editor's squiggles.

## Layout

| Path | Responsibility |
|---|---|
| `App.tsx` | Wires the client to the panels. Owns `EditorConfig` state and the debounced send |
| `engine.ts` | `EngineClient` — socket, auto-reconnect, and the engine's wire types *verbatim* |
| `config/editor.ts` | `EditorConfig`, defaults, localStorage (`djled.editor`), and `toEngineConfig` / `fromEngineConfig` |
| `config/scales.ts` | The two axes. Every gizmo, tick and hit test goes through here |
| `config/eq.ts` | RBJ cookbook biquads, verbatim |
| `config/curve.ts` | Intensity curve — cubic Béziers pinned at (0,0)–(1,1) |
| `config/notes.ts` | MIDI note ↔ position on the frequency axis, and the relabelled ticks |
| `color/oklab.ts` | Port of `engine/src/color/oklab.rs` |
| `color/surface.ts` | Port of `engine/src/color/surface.rs` |
| `color/display.ts` | Screen-side conversion. **Not** the same as the LED path |
| `color/reference.json` | Generated from the Rust — do not hand-edit |
| `spectrum/layout.ts` | Pixel geometry for the whole editor block, one coordinate space |
| `spectrum/paint.ts` | Canvas raster: colour field, grid, bars. Pure functions, no React |
| `spectrum/GizmoLayer.tsx` | SVG interaction layer over the canvas |
| `spectrum/render.ts` | Port of the compositing fold in `engine/src/color/render.rs` |
| `components/` | Mantine panels: layer stack, source, MIDI, settings, gizmos, strip preview |
| `theme.ts` | Every colour, radius and spacing decision, including `theme.other.plot` |
| `styles.css` | Two rules. Keep it that way |

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
- **The plot draws one layer, the strip preview draws the stack.** Six sets of
  gizmos on one graph would be unclickable, which is why the active layer is a
  first-class piece of editor state.
- **The whole config goes over the wire on every pointer move.** That's
  deliberate — one message means the two sides can't disagree about which half
  of an edit landed. Keep `toEngineConfig` cheap and allocation-light.
- **Gizmo checkboxes are visibility, not bypass.** Nothing in that panel touches
  the signal.
- `MAX_LAYERS` is 12 and `SAMPLE_LENGTHS` is a fixed list, both in
  `config/editor.ts`.

## Tests

| File | What it defends |
|---|---|
| `color/reference.test.ts` | Parity with the Rust colour path, against the generated fixture |
| `color/oklab.test.ts` | The conversions on their own |
| `spectrum/stack.test.ts` | The browser's copy of the compositing fold |
| `spectrum/render.test.ts` | Spatial mapping — including that an even-length strip's mirror fold lands on one specific index |
| `config/eq.test.ts` | Closed-form properties, not captured values |
| `config/editor.test.ts`, `config/notes.test.ts` | Round-tripping and the note axis |

`scripts/stack-smoke.mjs` is the only check that crosses the real socket: it
sends a two-layer show to a running engine and reports the state echo, the
per-layer frames and the composited strip. Start the engine first.
