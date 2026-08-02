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
  hasStoredConfig,
  loadConfig,
  saveConfig,
  toSurface,
  withSurface,
  type EditorConfig,
} from "./config/editor";
import { GizmoPanel } from "./components/GizmoPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { StripPreview } from "./components/StripPreview";
import { DEFAULT_URL, EngineClient, decodeStrip, type Frame, type Status } from "./engine";
import { SpectrumEditor } from "./spectrum/SpectrumEditor";
import type { SpectrumFrame } from "./spectrum/paint";
import { renderStrip } from "./spectrum/render";

/** Sampled once at load: a later save must not change what a reconnect does. */
const HAD_LOCAL_CONFIG = hasStoredConfig();

export default function App() {
  const [config, setConfig] = useState<EditorConfig>(loadConfig);
  const [status, setStatus] = useState<Status>("connecting");
  const [frame, setFrame] = useState<Frame | null>(null);
  const [brightness, setBrightness] = useState(1);
  const [ledCount, setLedCount] = useState(150);
  const [error, setError] = useState<string | null>(null);

  const clientRef = useRef<EngineClient | null>(null);

  useEffect(() => {
    const client = new EngineClient(DEFAULT_URL, {
      onStatus: setStatus,
      onFrame: setFrame,
      onError: setError,
      onState: (state) => {
        setBrightness(state.brightness);
        setLedCount(state.ledCount);
        // The engine's surface is authoritative on first connect, but only if
        // nothing has been authored here — otherwise a reconnect would throw
        // away unsaved edits.
        if (!HAD_LOCAL_CONFIG) setConfig((c) => withSurface(c, state.surface));
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
      // Only the colour surface exists in the protocol so far; the rest of the
      // config is previewed locally until the engine grows the fields.
      clientRef.current?.setSurface(toSurface(next));
    },
    [persist],
  );

  const applyBrightness = useCallback((value: number) => {
    setBrightness(value);
    clientRef.current?.setBrightness(value);
  }, []);

  const demo = useDemoFrame(status !== "connected");
  const live: SpectrumFrame | null =
    status === "connected" && frame ? { levels: frame.levels, centers: frame.centers } : null;
  const spectrum = live ?? demo;

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
            <StatusBadge status={status} frame={frame} ledCount={ledCount} />
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
            <SettingsPanel
              config={config}
              onChange={applyConfig}
              brightness={brightness}
              onBrightness={applyBrightness}
              sampleRate={frame?.sampleRate ?? 0}
            />
            <GizmoPanel config={config} onChange={applyConfig} />
          </Stack>

          <Paper flex={1} miw={0}>
            <Stack gap="sm">
              <SpectrumEditor
                config={config}
                onChange={applyConfig}
                frame={spectrum}
                ledCount={ledCount}
              />
              <Text size="xs" c="dimmed">
                Right-click the graph to add a colour keyframe · right-click the LED track to add a
                sector · click to select, <Kbd size="xs">Del</Kbd> to remove
              </Text>
            </Stack>
          </Paper>
        </Flex>

        <Paper>
          <Stack gap="md">
            <StripPreview
              label="Preview"
              hint={`${ledCount} LEDs`}
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
}: {
  status: Status;
  frame: Frame | null;
  ledCount: number;
}) {
  if (status === "connected") {
    const rate = frame?.sampleRate ? `${(frame.sampleRate / 1000).toFixed(1)} kHz` : "live";
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
