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
  LAYER_KIND_LABEL,
  findLayer,
  makeLayer,
  moveLayer,
  replaceLayer,
  type GizmoFlags,
  type Layer,
  type LayerKind,
  type SpectrumState,
  type StaticState,
} from "./config/layers";
import {
  clearDoc,
  defaultDoc,
  drivingLayer,
  fromEngineConfig,
  hasStoredDoc,
  loadDoc,
  saveDoc,
  toEngineConfig,
  type MasterConfig,
  type ShowDoc,
} from "./config/show";
import { DEFAULT_SAMPLE_RATE, EqCurve } from "./config/eq";
import { addKeyAt, removeKey, moveKey, stateAt, writeStateAt } from "./config/timeline";
import { Inspector } from "./components/Inspector";
import { LayerList } from "./components/LayerList";
import { MasterPanel } from "./components/MasterPanel";
import { SourcePanel } from "./components/SourcePanel";
import { StripPreview } from "./components/StripPreview";
import { Timeline } from "./components/Timeline";
import {
  DEFAULT_URL,
  EngineClient,
  decodeStrip,
  type AudioSource,
  type AudioState,
  type Frame,
  type Status,
} from "./engine";
import { GradientEditor } from "./gradient/GradientEditor";
import { compositeShow, previewLayer, type RenderInput } from "./render/composite";
import { SpectrumEditor } from "./spectrum/SpectrumEditor";
import type { SpectrumFrame } from "./spectrum/paint";
import { applyEq } from "./spectrum/render";
import { useTransport } from "./useTransport";

/** Sampled once at load: a later save must not change what a reconnect does. */
const HAD_LOCAL_DOC = hasStoredDoc();

/** The engine's default dB window, until it reports its own. */
const DEFAULT_DB_SPAN = 60;

export default function App() {
  const [doc, setDoc] = useState<ShowDoc>(() => loadDoc());
  const [status, setStatus] = useState<Status>("connecting");
  const [frame, setFrame] = useState<Frame | null>(null);
  const [brightness, setBrightness] = useState(1);
  const [ledCount, setLedCount] = useState(150);
  const [dbSpan, setDbSpan] = useState(DEFAULT_DB_SPAN);
  const [audio, setAudio] = useState<AudioState | null>(null);
  const [error, setError] = useState<string | null>(null);

  const { time, playing, setPlaying, seek } = useTransport(doc.duration);
  const clientRef = useRef<EngineClient | null>(null);

  useEffect(() => {
    const client = new EngineClient(DEFAULT_URL, {
      onStatus: (next) => {
        setStatus(next);
        // Nothing is being captured while the engine is away, and leaving the
        // last device on screen would invite selecting one into the void.
        if (next !== "connected") setAudio(null);
      },
      onFrame: setFrame,
      onError: setError,
      onState: (state) => {
        setBrightness(state.brightness);
        setLedCount(state.ledCount);
        setDbSpan(Math.max(1, state.dbCeil - state.dbFloor));
        // Unlike the rest of the state, this is not an echo of something
        // authored here: it is the engine reporting what it managed to open.
        setAudio(state.audio);
        // The engine's config is authoritative on first connect, but only if
        // nothing has been authored here — otherwise a reconnect would throw
        // away unsaved edits.
        if (!HAD_LOCAL_DOC) setDoc((d) => fromEngineConfig(d, state.config));
      },
    });
    clientRef.current = client;
    client.connect();
    return () => client.close();
  }, []);

  const persist = useDebouncedCallback(saveDoc, 400);

  /**
   * Every edit goes through here.
   *
   * The engine still runs one analyser and one `ShowConfig`, so only the
   * driving layer can cross the wire — see `toEngineConfig`. The local preview
   * shows the true composite regardless, which is why the two strips at the
   * bottom can legitimately disagree until the backend grows a stack.
   */
  const applyDoc = useCallback(
    (next: ShowDoc) => {
      setDoc(next);
      setError(null);
      persist(next);
      const config = toEngineConfig(next, time);
      if (config) clientRef.current?.setConfig(config);
    },
    [persist, time],
  );

  const applyBrightness = useCallback((value: number) => {
    setBrightness(value);
    clientRef.current?.setBrightness(value);
  }, []);

  // Deliberately not optimistic: whether a device opens is the engine's to
  // answer, and showing a selection that failed would be a lie the size of a
  // silent strip. The panel updates when the engine says it switched.
  const applySource = useCallback((source: AudioSource) => {
    clientRef.current?.setSource(source);
  }, []);

  const refreshSources = useCallback(() => {
    clientRef.current?.listSources();
  }, []);

  /* ------------------------------------------------------------ live input */

  const demo = useDemoFrame(status !== "connected");
  const live: SpectrumFrame | null =
    status === "connected" && frame ? { levels: frame.levels, centers: frame.centers } : null;
  const captured = live ?? demo;

  const sampleRate = frame?.sampleRate || DEFAULT_SAMPLE_RATE;

  const focused = findLayer(doc.layers, doc.focusedId);
  const focusedState = focused ? stateAt(focused, time) : null;

  // Live levels arrive with the EQ already in them — the engine applies it
  // between the AGC and the range map. Applying it again here would double it,
  // so the local pass exists only to keep the curve meaningful while offline.
  const focusedEq =
    focused?.kind === "spectrum" ? (focusedState as SpectrumState).eq : undefined;
  const eqCurve = useMemo(
    () => new EqCurve(focusedEq ?? [], sampleRate),
    [focusedEq, sampleRate],
  );
  const spectrum = useMemo(
    () => (live ? captured : applyEq(captured, eqCurve, dbSpan)),
    [live, captured, eqCurve, dbSpan],
  );

  // MIDI has no engine support yet, so a MIDI layer previews against silence
  // rather than against the audio — showing it react to the microphone would be
  // a straightforward lie about what the strip will do.
  const midiFrame = useMemo<SpectrumFrame>(() => ({ levels: [], centers: [] }), []);

  const input = useMemo<RenderInput>(
    () => ({ audio: spectrum, midi: midiFrame, ledCount, time }),
    [spectrum, midiFrame, ledCount, time],
  );

  const previewStrip = useMemo(
    () => compositeShow(doc, input, brightness),
    [doc, input, brightness],
  );

  const layerStrip = useMemo(
    () => (focused ? previewLayer(focused, input, brightness) : []),
    [focused, input, brightness],
  );

  const engineStrip = useMemo<Rgb[] | null>(
    () =>
      live && frame
        ? decodeStrip(frame.strip).map(([r, g, b]) => ledBytesToDisplay(r, g, b))
        : null,
    [live, frame],
  );

  /* --------------------------------------------------------------- editing */

  const patchLayer = useCallback(
    (id: string, patch: Partial<Layer>) => {
      const layer = findLayer(doc.layers, id);
      if (!layer) return;
      applyDoc({
        ...doc,
        layers: replaceLayer(doc.layers, id, { ...layer, ...patch } as Layer),
      });
    },
    [applyDoc, doc],
  );

  /**
   * An edit to the focused layer's state, at the playhead.
   *
   * With no keys this simply replaces the layer's state. With keys it auto-keys
   * — the key under the playhead is rewritten, or one is created there — which
   * is the whole of the snapshot model's authoring story.
   */
  const editState = useCallback(
    (state: SpectrumState | StaticState) => {
      if (!focused) return;
      applyDoc({
        ...doc,
        layers: replaceLayer(doc.layers, focused.id, writeStateAt(focused, time, state)),
      });
    },
    [applyDoc, doc, focused, time],
  );

  const addLayer = useCallback(
    (kind: LayerKind) => {
      const layer = makeLayer(kind, ledCount);
      applyDoc({ ...doc, layers: [...doc.layers, layer], focusedId: layer.id });
    },
    [applyDoc, doc, ledCount],
  );

  const removeLayer = useCallback(
    (id: string) => {
      if (doc.layers.length <= 1) return;
      const layers = doc.layers.filter((l) => l.id !== id);
      applyDoc({
        ...doc,
        layers,
        focusedId: doc.focusedId === id ? layers[layers.length - 1].id : doc.focusedId,
        soloId: doc.soloId === id ? null : doc.soloId,
      });
    },
    [applyDoc, doc],
  );

  const setGizmos = useCallback(
    (gizmos: GizmoFlags) => applyDoc({ ...doc, view: { ...doc.view, gizmos } }),
    [applyDoc, doc],
  );

  const setMaster = useCallback(
    (master: MasterConfig) => applyDoc({ ...doc, master }),
    [applyDoc, doc],
  );

  const driving = drivingLayer(doc);
  const focusedIndex = doc.layers.findIndex((l) => l.id === doc.focusedId);

  return (
    <Container size={1720} py="md">
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
              clearDoc();
              applyDoc(defaultDoc(ledCount));
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

        <Flex gap="md" align="flex-start" direction={{ base: "column", lg: "row" }}>
          <Stack gap="md" w={{ base: "100%", lg: 250 }} style={{ flexShrink: 0 }}>
            <LayerList
              layers={doc.layers}
              focusedId={doc.focusedId}
              soloId={doc.soloId}
              onFocus={(id) => applyDoc({ ...doc, focusedId: id })}
              onToggle={(id) =>
                patchLayer(id, { enabled: !findLayer(doc.layers, id)?.enabled })
              }
              onSolo={(soloId) => applyDoc({ ...doc, soloId })}
              onMove={(id, delta) => applyDoc({ ...doc, layers: moveLayer(doc.layers, id, delta) })}
              onAdd={addLayer}
              onRemove={removeLayer}
            />
            <SourcePanel audio={audio} onSource={applySource} onRefresh={refreshSources} />
            <MasterPanel
              master={doc.master}
              onChange={setMaster}
              brightness={brightness}
              onBrightness={applyBrightness}
              sampleRate={frame?.sampleRate ?? 0}
            />
          </Stack>

          <Stack gap="md" flex={1} miw={0} w={{ base: "100%", lg: "auto" }}>
            <Paper>
              <Stack gap="sm">
                <Group justify="space-between" gap="xs">
                  <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
                    {focused ? focused.name : "No layer"}
                  </Text>
                  {focused && (
                    <Text size="xs" c="dimmed" ff="monospace">
                      {LAYER_KIND_LABEL[focused.kind]}
                      {focused.kind === "spectrum" &&
                        ` · ${(focusedState as SpectrumState).source}`}
                    </Text>
                  )}
                </Group>

                {focused?.kind === "spectrum" && (
                  <>
                    <SpectrumEditor
                      state={focusedState as SpectrumState}
                      onChange={editState}
                      gizmos={doc.view.gizmos}
                      frame={
                        (focusedState as SpectrumState).source === "midi" ? midiFrame : spectrum
                      }
                      ledCount={ledCount}
                      sampleRate={sampleRate}
                    />
                    <Text size="xs" c="dimmed">
                      Right-click the graph for a colour keyframe · double-click for an EQ
                      band · right-click the LED track for a sector · click to select,{" "}
                      <Kbd size="xs">Del</Kbd> to remove
                    </Text>
                  </>
                )}

                {focused?.kind === "static" && (
                  <>
                    <GradientEditor
                      state={focusedState as StaticState}
                      onChange={editState}
                      ledCount={ledCount}
                    />
                    <Text size="xs" c="dimmed">
                      Right-click the ramp for a colour stop · drag to move · click to
                      select, <Kbd size="xs">Del</Kbd> to remove
                    </Text>
                  </>
                )}

                {focused && (
                  <StripPreview
                    label="This layer"
                    hint={`${focused.blendMode} · ${Math.round(focused.opacity * 100)}%`}
                    colors={layerStrip}
                    height={18}
                  />
                )}
              </Stack>
            </Paper>

            <Timeline
              layers={doc.layers}
              focusedId={doc.focusedId}
              time={time}
              playing={playing}
              duration={doc.duration}
              allLanes={doc.view.allLanes}
              onTime={seek}
              onPlaying={setPlaying}
              onDuration={(duration) => applyDoc({ ...doc, duration })}
              onAllLanes={(allLanes) => applyDoc({ ...doc, view: { ...doc.view, allLanes } })}
              onAddKey={(id) => {
                const layer = findLayer(doc.layers, id);
                if (layer) patchLayer(id, addKeyAt(layer, time));
              }}
              onRemoveKey={(id, keyId) => {
                const layer = findLayer(doc.layers, id);
                if (layer) patchLayer(id, removeKey(layer, keyId));
              }}
              onMoveKey={(id, keyId, t) => {
                const layer = findLayer(doc.layers, id);
                if (layer) patchLayer(id, moveKey(layer, keyId, t));
              }}
              onFocus={(id) => applyDoc({ ...doc, focusedId: id })}
            />
          </Stack>

          <Stack gap="md" w={{ base: "100%", lg: 275 }} style={{ flexShrink: 0 }}>
            <Inspector
              layer={focused}
              state={focusedState}
              isBottom={focusedIndex === 0}
              ledCount={ledCount}
              gizmos={doc.view.gizmos}
              onLayer={(patch) => focused && patchLayer(focused.id, patch)}
              onState={editState}
              onGizmos={setGizmos}
            />
          </Stack>
        </Flex>

        <Paper>
          <Stack gap="md">
            <StripPreview
              label="Preview"
              hint={[
                `${ledCount} LEDs`,
                `${doc.soloId ? 1 : doc.layers.filter((l) => l.enabled).length} layers`,
                doc.master.mirror && "mirrored",
                doc.master.reverse && "reversed",
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
            {engineStrip && doc.layers.filter((l) => l.enabled).length > 1 && (
              <Text size="xs" c="dimmed" lh={1.4}>
                The engine still runs a single analyser and a single surface, so the wall
                is showing only <b>{driving?.name ?? "one layer"}</b>. Compositing is
                local until the backend grows a stack — which is why these two strips
                disagree.
              </Text>
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
