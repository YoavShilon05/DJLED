/**
 * The single place any colour, radius or spacing decision is made.
 *
 * Two rules keep this honest:
 *
 *  1. Components are never restyled. Everything below is either a theme token
 *     Mantine already reads (`colors`, `radius`, `fontSizes`) or a `defaultProps`
 *     entry — no `classNames`, no `styles`, no CSS module per component.
 *  2. The canvas layers are not Mantine components and cannot inherit anything,
 *     so their palette lives on `theme.other.plot`. Drawn pixels and rendered
 *     components therefore still resolve to one source of truth.
 */

import { createTheme, type MantineColorsTuple } from "@mantine/core";

/** Azure. Used for focus, selection, and every filled control. */
const brand: MantineColorsTuple = [
  "#e5f6ff",
  "#cbe9fb",
  "#97d1f5",
  "#5fb7ef",
  "#33a2ea",
  "#1795e8",
  "#008fe8",
  "#007ccf",
  "#006eba",
  "#005ea3",
];

/**
 * Replaces Mantine's `dark` scale. The stops that matter are the ones Mantine
 * maps to semantic variables, and they are chosen to give three distinct
 * surface levels without a single background override anywhere else:
 *
 *   dark[8] page      (set once, in styles.css)
 *   dark[7] panel     (`--mantine-color-body`, what Paper uses)
 *   dark[6] control   (`--mantine-color-default`, what inputs and buttons use)
 *
 * dark[4] is the default border, dark[2] is dimmed text, dark[0] is body text.
 */
const shell: MantineColorsTuple = [
  "#cbd2de",
  "#b3bbc9",
  "#8a93a4",
  "#666f80",
  "#333a48",
  "#2b3140",
  "#242a36",
  "#191d26",
  "#101319",
  "#090b0f",
];

/** Colours for the hand-drawn layers: the spectrum canvas and the gizmo SVG. */
export interface PlotPalette {
  /** Grid line at an unlabelled subdivision. */
  grid: string;
  /** Grid line at a labelled pivot. */
  gridStrong: string;
  /** Outline the plot area is drawn inside. */
  frame: string;
  /** Live spectrum bars. */
  bar: string;
  /** Spectrum bars, at the top edge where the cap reads. */
  barCap: string;
  /** Threshold, clamp and LED gizmo lines — deliberately plain white. */
  gizmo: string;
  /** The same lines when they are being dragged. */
  gizmoActive: string;
  /** Region of the plot the threshold has cut off. */
  muteWash: string;
  /** Ring drawn around a colour keyframe so it reads against any field colour. */
  keyframeRing: string;
  /** Axis tick text. */
  tick: string;
}

const plot: PlotPalette = {
  grid: "rgba(203, 210, 222, 0.07)",
  gridStrong: "rgba(203, 210, 222, 0.14)",
  frame: "#333a48",
  bar: "rgba(255, 255, 255, 0.72)",
  barCap: "rgba(255, 255, 255, 0.95)",
  gizmo: "#ffffff",
  gizmoActive: "#008fe8",
  muteWash: "rgba(9, 11, 15, 0.55)",
  keyframeRing: "#0b0d12",
  tick: "#8a93a4",
};

declare module "@mantine/core" {
  export interface MantineThemeOther {
    plot: PlotPalette;
  }
}

export const theme = createTheme({
  primaryColor: "brand",
  // One shade for both schemes: the UI is dark-only, and the bright azure is
  // wanted at full strength on sliders and checkboxes.
  primaryShade: { light: 6, dark: 6 },
  autoContrast: true,
  colors: { brand, dark: shell },

  fontFamily: 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  fontFamilyMonospace: 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Menlo, monospace',
  defaultRadius: "md",
  focusRing: "auto",
  cursorType: "pointer",

  // A control-surface density: everything one step smaller than Mantine's
  // default, set here rather than as a `size` prop on 40 call sites.
  components: {
    Paper: { defaultProps: { withBorder: true, p: "md" } },
    Text: { defaultProps: { size: "sm" } },
    Select: { defaultProps: { size: "xs", comboboxProps: { size: "xs" } } },
    NumberInput: { defaultProps: { size: "xs" } },
    Checkbox: { defaultProps: { size: "xs" } },
    Slider: { defaultProps: { size: "sm", thumbSize: 14 } },
    SegmentedControl: { defaultProps: { size: "xs" } },
    Button: { defaultProps: { size: "xs" } },
    ActionIcon: { defaultProps: { size: "sm", variant: "subtle" } },
    Badge: { defaultProps: { variant: "light", size: "sm" } },
    Tooltip: { defaultProps: { openDelay: 400, withArrow: true, fz: "xs" } },
    Divider: { defaultProps: { color: "dark.5" } },
    Popover: { defaultProps: { shadow: "md", withinPortal: true, radius: "md" } },
    ColorPicker: { defaultProps: { format: "hex", size: "sm" } },
  },

  other: { plot },
});
