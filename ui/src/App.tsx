import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert, Box, Group, Kbd, Paper, ScrollArea, Stack, Text } from "@mantine/core";
import { useDebouncedCallback } from "@mantine/hooks";

import { ledBytesToDisplay, type Rgb } from "./color/display";
import {
  DEFAULT_CONFIG,
  activeLayer,
  clearConfig,
  filtersMidiChannel,
  fromEngineConfig,
  hasStoredConfig,
  isStatic,
  loadConfig,
  saveConfig,
  toEngineConfig,
  withLayer,
  type EditorConfig,
  type EditorLayer,
} from "./config/editor";
import { DEFAULT_SAMPLE_RATE, EqCurve } from "./config/eq";
import { presetName } from "./config/presets";
import { fieldLayer, keyOf, mergeKeyEdit } from "./config/timeline";
import { HeaderBar } from "./components/HeaderBar";
import { Inspector } from "./components/Inspector";
import { LayerStack } from "./components/LayerStack";
import type { LayerPreview } from "./components/LayerThumb";
import { OverlayMenu } from "./components/OverlayMenu";
import { PresetBar } from "./components/PresetBar";
import { StripPreview } from "./components/StripPreview";
import { TimelineBar } from "./components/TimelineBar";
import {
  DEFAULT_URL,
  EngineClient,
  decodeStrip,
  type EngineState,
  type Frame,
  type InputDevice,
  type LayerStatus,
  type PresetInfo,
  type Status,
} from "./engine";
import { FREQUENCY_AXIS, noteAxis } from "./spectrum/axis";
import { SpectrumEditor } from "./spectrum/SpectrumEditor";
import type { SpectrumFrame } from "./spectrum/paint";
import { applyEq, renderLayer, renderStack } from "./spectrum/render";

/** Sampled once at load: a later save must not change what a reconnect does. */
const HAD_LOCAL_CONFIG = hasStoredConfig();

/** The engine's default dB window, until it reports its own. */
const DEFAULT_DB_SPAN = 60;

const NO_STATUS = new Map<string, LayerStatus>();

const NO_PRESETS: PresetInfo[] = [];

/**
 * Length of a layer-row thumbnail, in LEDs.
 *
 * Far coarser than the wall, and deliberately: a hundred-odd screen pixels
 * cannot show 600 of anything, and this is rendered for every row of the stack
 * on every frame. See `LayerThumb`.
 */
const THUMB_LEDS = 48;

/** The sidebar's width. Wide enough for a device name, narrow enough to leave
 *  the graph the majority of a laptop screen. */
const SIDEBAR_W = 340;

export default function App() {
  const [config, setConfig] = useState<EditorConfig>(loadConfig);
  const [status, setStatus] = useState<Status>("connecting");
  /**
   * The engine's latest analysis, and the one piece of state that moves on its
   * own — thirty times a second, for as long as the page is open.
   *
   * Two things follow from that, and both are easy to lose track of. The first
   * is that it must not queue: the client coalesces frames onto an animation
   * frame precisely so that a tab which cannot keep up drops them instead of
   * banking them, which is what used to run the tab out of memory. See
   * `engine.ts`.
   *
   * The second is that everything re-rendered by a frame is re-rendered thirty
   * times a second. Only the plot's canvas and the two strips are actually
   * driven by one, so every panel beside them is memoised and its props are
   * kept referentially stable on purpose. A callback that forgets its
   * `useCallback`, or a prop built inline, quietly puts a whole Mantine panel
   * back on the frame path.
   *
   * The layer thumbnails are the one thing that looks like an exception and is
   * not: they are painted from a ref on their own timer rather than from a
   * prop, so the rows around them never re-render. See `LayerThumb`.
   */
  const [frame, setFrame] = useState<Frame | null>(null);
  const [brightness, setBrightness] = useState(1);
  const [ledCount, setLedCount] = useState(150);
  const [dbSpan, setDbSpan] = useState(DEFAULT_DB_SPAN);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [layerStatus, setLayerStatus] = useState<LayerStatus[]>([]);
  const [presets, setPresets] = useState<PresetInfo[]>(NO_PRESETS);
  const [activePreset, setActivePreset] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const clientRef = useRef<EngineClient | null>(null);
  /**
   * The last preset slot the engine reported, or null before the first state.
   *
   * This is the whole of the editor's rule for when to replace what is on
   * screen, and it needs to be, because presets moved the show's home. The
   * engine holds the twelve of them on disk and a global hotkey can swap them
   * with this page closed — so the engine's copy is authoritative, and a change
   * in *which slot* it is serving is the one unambiguous signal that the show
   * changed underneath us rather than because of us.
   *
   * Everything else stays as it was. The engine announces again whenever the
   * stack changes shape — that is how a newly added layer learns which device
   * it resolved to — and adopting on one of those would re-mint every id and
   * pull the selection out from under the edit that caused it. Those announces
   * carry the same slot, so they are ignored here.
   */
  const seenPreset = useRef<number | null>(null);
  /**
   * Set when the socket comes back after having been up before.
   *
   * Edits made while the engine was away never reached it, so its copy of the
   * live slot is behind ours. Re-asserting on reconnect is what keeps a socket
   * blip from quietly discarding them — the one case where the editor still
   * pushes rather than adopts.
   */
  const reconnected = useRef(false);
  /** The current config, for the reconnect push — which happens inside a socket
   *  callback and cannot close over a render's state. */
  const configRef = useRef<EditorConfig | null>(null);

  /**
   * Take the engine's show as the editor's own.
   *
   * Almost always that is the whole of it — the engine owns the presets, so
   * what it is running is what is being edited. The one thing pushed back is a
   * show the editor cannot represent: a MIDI layer still filtering on a channel
   * from before the palette existed. Adopting normalises it away here, so
   * leaving the engine running the old one would have the preview showing
   * sixteen channels while the wall showed one, with nothing on screen saying
   * why.
   */
  const adopt = useCallback((client: EngineClient, state: EngineState) => {
    const next = fromEngineConfig(configRef.current ?? loadConfig(), state.config);
    setConfig(next);
    if (filtersMidiChannel(state.config)) client.setConfig(toEngineConfig(next));
  }, []);

  useEffect(() => {
    const client = new EngineClient(DEFAULT_URL, {
      onStatus: (next) => {
        setStatus(next);
        // A reconnect, as opposed to the first connect: only then is there
        // anything the engine could have missed.
        if (next === "connected") reconnected.current = seenPreset.current !== null;
        // Nothing is being listened to while the engine is away, and leaving
        // the last devices on screen would invite selecting one into the void.
        if (next !== "connected") {
          setDevices([]);
          setLayerStatus([]);
        }
      },
      onFrame: setFrame,
      onError: setError,
      onState: (state) => {
        setBrightness(state.brightness);
        setLedCount(state.ledCount);
        setDbSpan(Math.max(1, state.dbCeil - state.dbFloor));
        // Unlike the rest of the state, these are not an echo of something
        // authored here: they are the engine reporting what it managed to open.
        setDevices(state.devices);
        setLayerStatus(state.layers);
        setPresets(state.presets ?? NO_PRESETS);
        setActivePreset(state.activePreset ?? 0);

        const first = seenPreset.current === null;
        const switched = !first && state.activePreset !== seenPreset.current;
        const wasReconnect = reconnected.current;
        seenPreset.current = state.activePreset ?? 0;
        reconnected.current = false;

        if (switched) {
          // A hotkey, or another tab. Adopt: the wall is already showing this.
          adopt(client, state);
          return;
        }

        if (first) {
          // An untouched slot on the engine's side and a show authored here is
          // the upgrade path — everything anyone had before presets existed
          // lives in localStorage, and would otherwise be replaced on sight by
          // an empty slot. Seed the slot from it instead. Any other first
          // connect adopts, because the engine's presets are where shows live
          // now and the editor is a client of them.
          if (HAD_LOCAL_CONFIG && state.presets?.[state.activePreset]?.stored === false) {
            if (configRef.current) client.setConfig(toEngineConfig(configRef.current));
          } else {
            adopt(client, state);
          }
        } else if (wasReconnect && configRef.current) {
          // Same slot, but the socket was away — anything edited during the gap
          // is here and not there.
          client.setConfig(toEngineConfig(configRef.current));
        }
      },
    });
    clientRef.current = client;
    client.connect();
    return () => client.close();
  }, []);

  // Kept in step with the state so a socket callback can read the current
  // config without being re-created on every edit.
  configRef.current = config;

  const persist = useDebouncedCallback(saveConfig, 400);

  /**
   * Every edit in the editor lands here.
   *
   * Takes an updater as well as a value, and updates `configRef` *before*
   * setting state, because two edits in one event handler are a real gesture:
   * adding a timeline key selects it in the same click. React has not
   * re-rendered in between, so a second call that closed over the render's
   * `config` would build on the config from before the first and silently
   * throw it away — which is exactly how the new key used to vanish.
   */
  const applyConfig = useCallback(
    (next: EditorConfig | ((prev: EditorConfig) => EditorConfig)) => {
      const value = typeof next === "function" ? next(configRef.current ?? loadConfig()) : next;
      configRef.current = value;
      setConfig(value);
      setError(null);
      persist(value);
      clientRef.current?.setConfig(toEngineConfig(value));
    },
    [persist],
  );

  /** The layer list and the timeline bar edit the layer itself, so they land
   *  here. Everything that edits its *colour field* goes through `applyField`
   *  below, because a field belongs to one key of the loop rather than to the
   *  layer. */
  const applyLayer = useCallback(
    (layer: EditorLayer) => applyConfig((c) => withLayer(c, layer)),
    [applyConfig],
  );

  const applyBrightness = useCallback((value: number) => {
    setBrightness(value);
    clientRef.current?.setBrightness(value);
  }, []);

  const refreshSources = useCallback(() => {
    clientRef.current?.listSources();
  }, []);

  /**
   * Switch which preset is being edited.
   *
   * Nothing is sent with it and nothing is set locally: the engine owns the
   * twelve shows, so it answers with the new one in a `state` and the adoption
   * above picks it up — the same path a global hotkey takes. Two ways in, one
   * way through, so the bar and the keyboard cannot disagree.
   */
  const selectPreset = useCallback((slot: number) => {
    clientRef.current?.selectPreset(slot);
  }, []);

  const renamePreset = useCallback((slot: number, name: string) => {
    clientRef.current?.renamePreset(slot, name);
  }, []);

  /** Replaces the live preset with the default show. Not an undo — edits are
   *  saved as they are made, so there is no older version to put back. */
  const resetShow = useCallback(() => {
    clearConfig();
    applyConfig(DEFAULT_CONFIG);
  }, [applyConfig]);

  const live = status === "connected";
  const layer = activeLayer(config);

  /**
   * The key the graph is showing, resolved against the layer it belongs to.
   *
   * Going through `keyOf` rather than reading the stored id is what makes
   * selecting a different layer safe: a key id from another layer names nothing
   * here and falls back to the start of the loop, the same way an unknown layer
   * id falls back to the top of the stack.
   */
  const activeKeyId = keyOf(layer, config.activeKeyId).id;

  const selectKey = useCallback(
    (id: string) => applyConfig((c) => ({ ...c, activeKeyId: id })),
    [applyConfig],
  );

  /**
   * The layer as the graph and the inspector see it: every property of the
   * layer, with the colour field of the selected key.
   *
   * Editing one is folded back by `mergeKeyEdit`, which is the only place the
   * editor decides whether an edit belonged to the key or to the layer — a
   * colour moved belongs to the key, a colour added or deleted belongs to every
   * key at once. See `config/timeline.ts`.
   */
  const field = useMemo(() => fieldLayer(layer, activeKeyId), [layer, activeKeyId]);

  const applyField = useCallback(
    (edited: EditorLayer) => applyLayer(mergeKeyEdit(layer, activeKeyId, edited)),
    [applyLayer, layer, activeKeyId],
  );

  const statusById = useMemo(() => {
    if (!live) return NO_STATUS;
    return new Map(layerStatus.map((s) => [s.id, s]));
  }, [live, layerStatus]);

  const framesById = useMemo(() => {
    if (!live || !frame) return null;
    return new Map(frame.layers.map((f) => [f.id, f]));
  }, [live, frame]);

  const demo = useDemoFrame(!live);

  /**
   * The spectrum for one layer.
   *
   * Live levels arrive with that layer's EQ already in them — the engine
   * applies it between the AGC and the range map — so applying it again here
   * would double it. The local pass exists only to keep the curve meaningful
   * while offline, or for a layer whose device would not open and which
   * therefore has no live frame of its own.
   */
  const spectrumFor = useCallback(
    (target: EditorLayer): SpectrumFrame => {
      const found = framesById?.get(target.id);
      // Channels come with the levels or not at all — an audio layer has none,
      // and the offline demo spectrum has none either, so a preview with
      // nothing connected shows the colour field rather than guessing at notes
      // nobody played.
      if (found)
        return { levels: found.levels, centers: found.centers, channels: found.channels };
      return applyEq(demo, new EqCurve(target.eq, DEFAULT_SAMPLE_RATE), dbSpan);
    },
    [framesById, demo, dbSpan],
  );

  const activeStatus = statusById.get(layer.id) ?? null;
  const sampleRate = activeStatus?.sampleRate || DEFAULT_SAMPLE_RATE;
  const spectrum = useMemo(() => spectrumFor(layer), [spectrumFor, layer]);

  // MIDI stretches a note range across the same axis, so the marks stay where
  // they are and only their names change. Frequencies while offline: with
  // nothing connected there is no note range in force to label against.
  const midi = activeStatus?.kind === "midi";
  const axis = useMemo(
    () => (midi ? noteAxis(layer.midi.lowNote, layer.midi.highNote) : FREQUENCY_AXIS),
    [midi, layer.midi.lowNote, layer.midi.highNote],
  );

  const previewStrip = useMemo(
    () => renderStack(config, spectrumFor, ledCount, brightness),
    [config, spectrumFor, ledCount, brightness],
  );

  const engineStrip = useMemo<Rgb[] | null>(
    () =>
      live && frame
        ? decodeStrip(frame.strip).map(([r, g, b]) => ledBytesToDisplay(r, g, b))
        : null,
    [live, frame],
  );

  /**
   * A strip per layer, for the thumbnails on the stack.
   *
   * Parked in a ref rather than put into state, because state is what puts
   * something on the frame path: the rows are memoised, and handing each one a
   * fresh array thirty times a second would re-render twelve Mantine panels to
   * repaint twelve canvases. The canvases read this on their own timer instead.
   *
   * Rendered at full opacity and full brightness, which is a deliberate
   * disagreement with the wall: a thumbnail answers "which layer is this", and
   * a row faded to 10% or a master pulled down for a quiet passage would make
   * every one of them an identical black rectangle. How much of a layer is
   * getting through is what its slider and its dimmed row already say.
   */
  const thumbs = useRef(new Map<string, Rgb[]>());
  const preview = useMemo<LayerPreview>(
    () => ({ read: (id) => thumbs.current.get(id) ?? null }),
    [],
  );

  useEffect(() => {
    const next = new Map<string, Rgb[]>();
    for (const l of config.layers) {
      const full = l.opacity === 1 ? l : { ...l, opacity: 1 };
      next.set(l.id, renderLayer(full, spectrumFor(l), THUMB_LEDS));
    }
    thumbs.current = next;
  }, [config.layers, spectrumFor]);

  const enabled = config.layers.filter((l) => l.enabled && l.opacity > 0).length;

  return (
    /*
     * One screen, no page scroll.
     *
     * The editor used to be a tall document: six panels down the left, the
     * graph beside them, and the strip preview at the very bottom — so the
     * picture of what the wall was doing, which is the only ground truth in
     * the whole application, was the one thing you had to scroll to see. Here
     * the header, the twelve shows and both preview strips are pinned, and the
     * two columns under them scroll independently.
     */
    <Box
      h="100dvh"
      p="xs"
      style={{ display: "flex", flexDirection: "column", gap: "var(--mantine-spacing-xs)" }}
    >
      <HeaderBar
        status={status}
        ledCount={ledCount}
        layers={config.layers.length}
        statuses={layerStatus}
        brightness={brightness}
        onBrightness={applyBrightness}
        presetLabel={presetName(activePreset, presets[activePreset])}
        onReset={resetShow}
      />

      <PresetBar
        presets={presets}
        active={activePreset}
        onSelect={selectPreset}
        onRename={renamePreset}
        connected={live}
      />

      <Paper p="xs">
        <Stack gap={6}>
          <StripPreview
            label="Preview"
            hint={[
              `${ledCount} LEDs`,
              `${enabled} of ${config.layers.length} layers`,
            ].join(" · ")}
            colors={previewStrip}
            height={26}
          />
          {/* Kept mounted at zero content rather than unmounted while offline,
              so connecting does not shove the graph down the page. */}
          <StripPreview
            label="Engine"
            hint={
              !engineStrip
                ? "not connected"
                : frame && frame.droppedFrames > 0
                  ? `${frame.droppedFrames} dropped`
                  : "live"
            }
            colors={engineStrip ?? []}
            height={12}
          />
        </Stack>
      </Paper>

      {error && (
        <Alert color="red" variant="light" title="Engine error" py={6}>
          {error}
        </Alert>
      )}

      <Box style={{ display: "flex", gap: "var(--mantine-spacing-xs)", flex: 1, minHeight: 0 }}>
        <ScrollArea h="100%" w={SIDEBAR_W} style={{ flexShrink: 0 }} type="hover">
          <Stack gap="xs" pr="xs">
            <LayerStack
              config={config}
              onChange={applyConfig}
              status={statusById}
              preview={preview}
            />
            <Inspector
              layer={field}
              onChange={applyField}
              devices={devices}
              status={activeStatus}
              connected={live}
              onRefresh={refreshSources}
              midi={midi}
            />
          </Stack>
        </ScrollArea>

        <ScrollArea h="100%" style={{ flex: 1, minWidth: 0 }} type="hover">
          <Paper p="sm" mr="xs">
            <Stack gap="xs">
              <Group justify="space-between" align="center" gap="xs" wrap="nowrap">
                <Group gap="xs" align="baseline" wrap="nowrap" style={{ minWidth: 0 }}>
                  <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em" truncate>
                    {layer.name}
                  </Text>
                  <Text size="xs" c="dimmed" truncate>
                    {layerSummary(layer, config.layers.length, enabled)}
                  </Text>
                </Group>
                <OverlayMenu config={config} onChange={applyConfig} />
              </Group>
              <SpectrumEditor
                layer={field}
                onChange={applyField}
                gizmos={config.gizmos}
                frame={spectrum}
                axis={axis}
                ledCount={ledCount}
                sampleRate={sampleRate}
              />
              {/* Under the graph, not beside it: the graph shows one key and
                  this is where that key is chosen, so the two are one block.
                  It is also in the scrolling half — see the note on the shell
                  in `ui/CLAUDE.md`. */}
              <TimelineBar
                layer={layer}
                onChange={applyLayer}
                activeKeyId={activeKeyId}
                onSelectKey={selectKey}
              />
              <Text size="xs" c="dimmed">
                Right-click the graph for a colour keyframe ·{" "}
                {/* A still layer has no signal, so there is no EQ band to add
                    and no gesture for one — saying otherwise would be the
                    caption describing a different graph. */}
                {!isStatic(layer) && "double-click for an EQ band · "}
                right-click the LED track for a sector · right-click the timeline for a
                key · click to select, <Kbd size="xs">Del</Kbd> to remove
              </Text>
            </Stack>
          </Paper>
        </ScrollArea>
      </Box>
    </Box>
  );
}

/**
 * What the plot is showing, and what it is not.
 *
 * Worth saying out loud because the plot only ever draws one layer while the
 * strip above draws all of them, and someone looking at a bar that does not
 * match the wall should not have to work out why.
 */
function layerSummary(layer: EditorLayer, total: number, enabled: number): string {
  if (!layer.enabled) return "hidden — not composited, not analysed";
  if (layer.opacity <= 0) return "fully transparent — nothing of it reaches the strip";
  const opacity = layer.opacity < 1 ? `${Math.round(layer.opacity * 100)}% opacity · ` : "";
  // Said before anything about the stack, because it is the thing that explains
  // the shape of the graph underneath — a lane rather than a plot, with no
  // level axis, no bars and no thresholds.
  const still = isStatic(layer) ? "no source · a still colour · " : "";
  if (total === 1) return `${still}${opacity}the only layer`;
  return `${still}${opacity}one of ${enabled} showing`;
}

const DEMO_BANDS = 48;

/**
 * A slow synthetic spectrum, so the editor is not a dead flat line offline.
 * Centres are log-spaced across the same range the axis draws, so the demo bars
 * land on the grid exactly as real bands would.
 *
 * Shared by every layer that has no live frame. Two layers offline showing the
 * same bars is honest — neither is hearing anything.
 */
function useDemoFrame(active: boolean): SpectrumFrame {
  const centers = useMemo(
    () => Array.from({ length: DEMO_BANDS }, (_, i) => 20 * Math.pow(1000, i / (DEMO_BANDS - 1))),
    [],
  );
  const [levels, setLevels] = useState<number[]>(() => new Array(DEMO_BANDS).fill(0));

  useEffect(() => {
    if (!active) return;
    let raf = 0;
    const start = performance.now();
    const tick = () => {
      raf = requestAnimationFrame(tick);
      const t = (performance.now() - start) / 1000;
      setLevels(
        Array.from({ length: DEMO_BANDS }, (_, i) => {
          const x = i / (DEMO_BANDS - 1);
          const envelope = Math.exp(-x * 1.4);
          const wobble = 0.5 + 0.5 * Math.sin(t * 2 + x * 9);
          return Math.min(1, 0.15 + envelope * wobble * 1.5);
        }),
      );
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active]);

  return useMemo(() => ({ levels, centers }), [levels, centers]);
}
