import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Badge,
  Button,
  Container,
  Flex,
  Group,
  Kbd,
  Paper,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useDebouncedCallback } from "@mantine/hooks";

import { ledBytesToDisplay, type Rgb } from "./color/display";
import {
  DEFAULT_CONFIG,
  activeLayer,
  clearConfig,
  fromEngineConfig,
  hasStoredConfig,
  loadConfig,
  saveConfig,
  toEngineConfig,
  withLayer,
  type EditorConfig,
  type EditorLayer,
} from "./config/editor";
import { DEFAULT_SAMPLE_RATE, EqCurve } from "./config/eq";
import { GizmoPanel } from "./components/GizmoPanel";
import { LayerStack } from "./components/LayerStack";
import { MidiPanel } from "./components/MidiPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { SourcePanel } from "./components/SourcePanel";
import { StripPreview } from "./components/StripPreview";
import {
  DEFAULT_URL,
  EngineClient,
  decodeStrip,
  type Frame,
  type InputDevice,
  type LayerStatus,
  type Status,
} from "./engine";
import { FREQUENCY_AXIS, noteAxis } from "./spectrum/axis";
import { SpectrumEditor } from "./spectrum/SpectrumEditor";
import type { SpectrumFrame } from "./spectrum/paint";
import { applyEq, renderStack } from "./spectrum/render";

/** Sampled once at load: a later save must not change what a reconnect does. */
const HAD_LOCAL_CONFIG = hasStoredConfig();

/** The engine's default dB window, until it reports its own. */
const DEFAULT_DB_SPAN = 60;

const NO_STATUS = new Map<string, LayerStatus>();

export default function App() {
  const [config, setConfig] = useState<EditorConfig>(loadConfig);
  const [status, setStatus] = useState<Status>("connecting");
  const [frame, setFrame] = useState<Frame | null>(null);
  const [brightness, setBrightness] = useState(1);
  const [ledCount, setLedCount] = useState(150);
  const [dbSpan, setDbSpan] = useState(DEFAULT_DB_SPAN);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [layerStatus, setLayerStatus] = useState<LayerStatus[]>([]);
  const [error, setError] = useState<string | null>(null);

  const clientRef = useRef<EngineClient | null>(null);
  /**
   * Whether the engine's own show has already been adopted.
   *
   * Only the *first* state message is allowed to replace what is on screen.
   * The engine announces again whenever the stack changes shape — that is how a
   * newly added layer learns which device it resolved to — and adopting on one
   * of those would re-mint every id and pull the selection out from under the
   * edit that caused it.
   */
  const adopted = useRef(HAD_LOCAL_CONFIG);

  useEffect(() => {
    const client = new EngineClient(DEFAULT_URL, {
      onStatus: (next) => {
        setStatus(next);
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
        // The engine's config is authoritative on first connect, but only if
        // nothing has been authored here — otherwise a reconnect would throw
        // away unsaved edits.
        if (!adopted.current) {
          adopted.current = true;
          setConfig((c) => fromEngineConfig(c, state.config));
        }
      },
    });
    clientRef.current = client;
    client.connect();
    return () => client.close();
  }, []);

  const persist = useDebouncedCallback(saveConfig, 400);

  const applyConfig = useCallback(
    (next: EditorConfig) => {
      setConfig(next);
      setError(null);
      persist(next);
      clientRef.current?.setConfig(toEngineConfig(next));
    },
    [persist],
  );

  /** Every panel except the layer list edits one layer, so they all land here. */
  const applyLayer = useCallback(
    (layer: EditorLayer) => applyConfig(withLayer(config, layer)),
    [applyConfig, config],
  );

  const applyBrightness = useCallback((value: number) => {
    setBrightness(value);
    clientRef.current?.setBrightness(value);
  }, []);

  const refreshSources = useCallback(() => {
    clientRef.current?.listSources();
  }, []);

  const live = status === "connected";
  const layer = activeLayer(config);

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
      if (found) return { levels: found.levels, centers: found.centers };
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

  const enabled = config.layers.filter((l) => l.enabled && l.opacity > 0).length;

  return (
    <Container size={1600} py="md">
      <Stack gap="md">
        <Group justify="space-between" align="center">
          <Group gap="sm" align="baseline">
            <Title order={1} size="h4" tt="uppercase" lts="0.12em">
              DJLED
            </Title>
            <StatusBadge
              status={status}
              ledCount={ledCount}
              layers={config.layers.length}
              statuses={layerStatus}
            />
          </Group>
          <Button
            variant="default"
            onClick={() => {
              clearConfig();
              applyConfig(DEFAULT_CONFIG);
            }}
          >
            Reset
          </Button>
        </Group>

        {error && (
          <Alert color="red" variant="light" title="Engine error">
            {error}
          </Alert>
        )}

        <Flex gap="md" align="flex-start" direction={{ base: "column", md: "row" }}>
          <Stack gap="md" w={{ base: "100%", md: 300 }} style={{ flexShrink: 0 }}>
            <LayerStack config={config} onChange={applyConfig} status={statusById} />
            <SourcePanel
              layer={layer}
              onChange={applyLayer}
              devices={devices}
              status={activeStatus}
              connected={live}
              onRefresh={refreshSources}
            />
            <SettingsPanel
              layer={layer}
              onChange={applyLayer}
              brightness={brightness}
              onBrightness={applyBrightness}
              sampleRate={activeStatus?.sampleRate ?? 0}
            />
            <MidiPanel layer={layer} onChange={applyLayer} live={midi} />
            <GizmoPanel config={config} onChange={applyConfig} />
          </Stack>

          <Paper flex={1} miw={0}>
            <Stack gap="sm">
              <Group justify="space-between" align="baseline">
                <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
                  {layer.name}
                </Text>
                <Text size="xs" c="dimmed">
                  {layerSummary(layer, config.layers.length, enabled)}
                </Text>
              </Group>
              <SpectrumEditor
                layer={layer}
                onChange={applyLayer}
                gizmos={config.gizmos}
                frame={spectrum}
                axis={axis}
                ledCount={ledCount}
                sampleRate={sampleRate}
              />
              <Text size="xs" c="dimmed">
                Right-click the graph for a colour keyframe · double-click for an EQ band ·
                right-click the LED track for a sector · click to select,{" "}
                <Kbd size="xs">Del</Kbd> to remove
              </Text>
            </Stack>
          </Paper>
        </Flex>

        <Paper>
          <Stack gap="md">
            <StripPreview
              label="Preview"
              hint={[
                `${ledCount} LEDs`,
                `${enabled} of ${config.layers.length} layers`,
                layer.mirror && "active layer mirrored",
                layer.reverse && "active layer reversed",
              ]
                .filter(Boolean)
                .join(" · ")}
              colors={previewStrip}
              height={30}
            />
            {engineStrip && (
              <StripPreview
                label="Engine"
                hint={frame && frame.droppedFrames > 0 ? `${frame.droppedFrames} dropped` : "live"}
                colors={engineStrip}
                height={18}
              />
            )}
          </Stack>
        </Paper>
      </Stack>
    </Container>
  );
}

/**
 * What the plot is showing, and what it is not.
 *
 * Worth saying out loud because the plot only ever draws one layer while the
 * strip below draws all of them, and someone looking at a bar that does not
 * match the wall should not have to work out why.
 */
function layerSummary(layer: EditorLayer, total: number, enabled: number): string {
  if (!layer.enabled) return "hidden — not composited, not analysed";
  if (layer.opacity <= 0) return "fully transparent — nothing of it reaches the strip";
  if (total === 1) return "the only layer";
  const opacity = layer.opacity < 1 ? `${Math.round(layer.opacity * 100)}% opacity · ` : "";
  return `${opacity}one of ${enabled} showing`;
}

function StatusBadge({
  status,
  ledCount,
  layers,
  statuses,
}: {
  status: Status;
  ledCount: number;
  layers: number;
  statuses: LayerStatus[];
}) {
  if (status === "connected") {
    // One dead layer is not a dead engine, and burying it in a per-layer panel
    // would let a stack run half-dark without anything saying so up here.
    const failed = statuses.filter((s) => s.error).length;
    if (failed > 0) {
      return (
        <Badge color="red">{`${failed} of ${statuses.length} layers have no source`}</Badge>
      );
    }
    const kinds = new Set(statuses.map((s) => s.kind));
    const label = kinds.size === 1 ? [...kinds][0] : `${kinds.size} kinds`;
    const stack = layers === 1 ? "1 layer" : `${layers} layers`;
    return <Badge color="teal">{`${stack} · ${label} · ${ledCount} LEDs`}</Badge>;
  }
  if (status === "connecting") return <Badge color="gray">connecting…</Badge>;
  return <Badge color="yellow">offline — editing locally</Badge>;
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
