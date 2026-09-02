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
  clearConfig,
  fromEngineConfig,
  hasStoredConfig,
  loadConfig,
  saveConfig,
  toEngineConfig,
  type EditorConfig,
} from "./config/editor";
import { DEFAULT_SAMPLE_RATE, EqCurve } from "./config/eq";
import { GizmoPanel } from "./components/GizmoPanel";
import { MidiPanel } from "./components/MidiPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { SourcePanel } from "./components/SourcePanel";
import { StripPreview } from "./components/StripPreview";
import {
  DEFAULT_URL,
  EngineClient,
  decodeStrip,
  type Frame,
  type InputSource,
  type InputState,
  type SourceKind,
  type Status,
} from "./engine";
import { FREQUENCY_AXIS, noteAxis } from "./spectrum/axis";
import { SpectrumEditor } from "./spectrum/SpectrumEditor";
import type { SpectrumFrame } from "./spectrum/paint";
import { applyEq, renderStrip } from "./spectrum/render";

/** Sampled once at load: a later save must not change what a reconnect does. */
const HAD_LOCAL_CONFIG = hasStoredConfig();

/** The engine's default dB window, until it reports its own. */
const DEFAULT_DB_SPAN = 60;

export default function App() {
  const [config, setConfig] = useState<EditorConfig>(loadConfig);
  const [status, setStatus] = useState<Status>("connecting");
  const [frame, setFrame] = useState<Frame | null>(null);
  const [brightness, setBrightness] = useState(1);
  const [ledCount, setLedCount] = useState(150);
  const [dbSpan, setDbSpan] = useState(DEFAULT_DB_SPAN);
  const [input, setInput] = useState<InputState | null>(null);
  const [error, setError] = useState<string | null>(null);

  const clientRef = useRef<EngineClient | null>(null);

  useEffect(() => {
    const client = new EngineClient(DEFAULT_URL, {
      onStatus: (next) => {
        setStatus(next);
        // Nothing is being listened to while the engine is away, and leaving
        // the last device on screen would invite selecting one into the void.
        if (next !== "connected") setInput(null);
      },
      onFrame: setFrame,
      onError: setError,
      onState: (state) => {
        setBrightness(state.brightness);
        setLedCount(state.ledCount);
        setDbSpan(Math.max(1, state.dbCeil - state.dbFloor));
        // Unlike the rest of the state, this is not an echo of something
        // authored here: it is the engine reporting what it managed to open.
        setInput(state.input);
        // The engine's config is authoritative on first connect, but only if
        // nothing has been authored here — otherwise a reconnect would throw
        // away unsaved edits.
        if (!HAD_LOCAL_CONFIG) setConfig((c) => fromEngineConfig(c, state.config));
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

  const applyBrightness = useCallback((value: number) => {
    setBrightness(value);
    clientRef.current?.setBrightness(value);
  }, []);

  // Deliberately not optimistic: whether a device opens is the engine's to
  // answer, and showing a selection that failed would be a lie the size of a
  // silent strip. The panel updates when the engine says it switched.
  const applySource = useCallback((source: InputSource) => {
    clientRef.current?.setSource(source);
  }, []);

  const refreshSources = useCallback(() => {
    clientRef.current?.listSources();
  }, []);

  const demo = useDemoFrame(status !== "connected");
  const live: SpectrumFrame | null =
    status === "connected" && frame ? { levels: frame.levels, centers: frame.centers } : null;
  const captured = live ?? demo;

  const sampleRate = frame?.sampleRate || DEFAULT_SAMPLE_RATE;

  // MIDI stretches a note range across the same axis, so the marks stay where
  // they are and only their names change. Frequencies while offline: with
  // nothing connected there is no note range in force to label against.
  const midi = input?.kind === "midi";
  const axis = useMemo(
    () => (midi ? noteAxis(config.midi.lowNote, config.midi.highNote) : FREQUENCY_AXIS),
    [midi, config.midi.lowNote, config.midi.highNote],
  );

  // Live levels arrive with the EQ already in them — the engine applies it
  // between the AGC and the range map. Applying it again here would double it,
  // so the local pass exists only to keep the curve meaningful while offline.
  const eqCurve = useMemo(() => new EqCurve(config.eq, sampleRate), [config.eq, sampleRate]);
  const spectrum = useMemo(
    () => (live ? captured : applyEq(captured, eqCurve, dbSpan)),
    [live, captured, eqCurve, dbSpan],
  );

  const previewStrip = useMemo(
    () => renderStrip(config, spectrum, ledCount, brightness),
    [config, spectrum, ledCount, brightness],
  );

  const engineStrip = useMemo<Rgb[] | null>(
    () =>
      live && frame
        ? decodeStrip(frame.strip).map(([r, g, b]) => ledBytesToDisplay(r, g, b))
        : null,
    [live, frame],
  );

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
            frame={frame}
            ledCount={ledCount}
            kind={input?.kind ?? null}
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
          <Stack gap="md" w={{ base: "100%", md: 280 }} style={{ flexShrink: 0 }}>
            <SourcePanel
              input={input}
              outOfRange={live ? (frame?.notesOutOfRange ?? 0) : 0}
              onSource={applySource}
              onRefresh={refreshSources}
            />
            <SettingsPanel
              config={config}
              onChange={applyConfig}
              brightness={brightness}
              onBrightness={applyBrightness}
              sampleRate={frame?.sampleRate ?? 0}
            />
            <MidiPanel config={config} onChange={applyConfig} live={midi} />
            <GizmoPanel config={config} onChange={applyConfig} />
          </Stack>

          <Paper flex={1} miw={0}>
            <Stack gap="sm">
              <SpectrumEditor
                config={config}
                onChange={applyConfig}
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
              hint={[`${ledCount} LEDs`, config.mirror && "mirrored", config.reverse && "reversed"]
                .filter(Boolean)
                .join(" · ")}
              colors={previewStrip}
              height={30}
            />
            {engineStrip && (
              <StripPreview
                label="Engine"
                hint={
                  frame && frame.droppedFrames > 0 ? `${frame.droppedFrames} dropped` : "live"
                }
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

function StatusBadge({
  status,
  frame,
  ledCount,
  kind,
}: {
  status: Status;
  frame: Frame | null;
  ledCount: number;
  kind: SourceKind | null;
}) {
  if (status === "connected") {
    // MIDI has no sample rate, so the badge names the source instead of
    // showing the "0.0 kHz" the field would otherwise read as.
    const rate =
      kind === "midi"
        ? "MIDI"
        : frame?.sampleRate
          ? `${(frame.sampleRate / 1000).toFixed(1)} kHz`
          : "live";
    return <Badge color="teal">{`${rate} · ${ledCount} LEDs`}</Badge>;
  }
  if (status === "connecting") return <Badge color="gray">connecting…</Badge>;
  return <Badge color="yellow">offline — editing locally</Badge>;
}

const DEMO_BANDS = 48;

/**
 * A slow synthetic spectrum, so the editor is not a dead flat line offline.
 * Centres are log-spaced across the same range the axis draws, so the demo bars
 * land on the grid exactly as real bands would.
 */
function useDemoFrame(active: boolean): SpectrumFrame {
  const centers = useMemo(
    () =>
      Array.from({ length: DEMO_BANDS }, (_, i) =>
        20 * Math.pow(1000, i / (DEMO_BANDS - 1)),
      ),
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
