import { memo, useState } from "react";
import {
  Anchor,
  Button,
  ColorPicker,
  ColorSwatch,
  Divider,
  Group,
  Popover,
  SimpleGrid,
  Stack,
  Text,
  UnstyledButton,
} from "@mantine/core";

import { CHANNEL_GRID, DEFAULT_CHANNEL_COLORS } from "../config/notes";

interface Props {
  /** Sixteen colours, index 0 being channel 1. Always sixteen — see
   *  `sanitiseChannelColors`. */
  colors: string[];
  onChange: (colors: string[]) => void;
}

/**
 * A colour per MIDI channel, behind one button.
 *
 * # Why this is not a list of sixteen fields
 *
 * Sixteen colours is a palette, and a palette is looked at rather than read: the
 * question being asked at it is "which of these is the pad" and the answer is a
 * colour, not the fourth row of a form. So they are a grid of squares, and the
 * grid is the whole control — a square shows a channel's colour, clicking one
 * opens the picker for it, and nothing else is on screen.
 *
 * Numbered *down* each column, 1–4, 5–8, 9–12, 13–16. A DAW's channels are read
 * as a list, so the columns are what stays stable as the eye moves: channels 1
 * to 4 sit together as one block rather than spread across the top of a table
 * nobody is reading across.
 *
 * # One popover, two faces
 *
 * The picker replaces the grid inside the same dropdown rather than opening a
 * second one on top of it. Nested popovers put the thing being edited behind
 * the thing editing it, and the square whose colour is changing is exactly what
 * the eye wants to watch — so the grid comes back with a click on "all
 * channels" and the swatch it comes back to is already updated.
 *
 * Memoised, like every other panel here: nothing in it is driven by a frame.
 */
export const ChannelColors = memo(function ChannelColors({ colors, onChange }: Props) {
  const [opened, setOpened] = useState(false);
  /** Which channel's picker is showing, or null for the grid. */
  const [editing, setEditing] = useState<number | null>(null);

  const set = (channel: number, color: string) =>
    onChange(colors.map((c, i) => (i === channel ? color : c)));

  const close = () => {
    setOpened(false);
    setEditing(null);
  };

  return (
    <Popover
      opened={opened}
      onDismiss={close}
      position="bottom-start"
      withArrow
      width={248}
    >
      <Popover.Target>
        <Button
          variant="default"
          size="xs"
          onClick={() => (opened ? close() : setOpened(true))}
          leftSection={
            // The first four, as a reminder of what is behind the button. Four
            // rather than sixteen because this is a label, not the control.
            <Group gap={3} wrap="nowrap">
              {colors.slice(0, 4).map((color, i) => (
                <ColorSwatch key={i} color={color} size={10} withShadow={false} />
              ))}
            </Group>
          }
        >
          Channel colour
        </Button>
      </Popover.Target>
      <Popover.Dropdown p="sm">
        {editing === null ? (
          <Stack gap="xs">
            <Text size="xs" c="dimmed" lh={1.35}>
              Every note is painted in its channel&rsquo;s colour. Click a square to change
              one.
            </Text>
            <SimpleGrid cols={4} spacing={6}>
              {CHANNEL_GRID.map((channel) => (
                <Square
                  key={channel}
                  channel={channel}
                  color={colors[channel] ?? DEFAULT_CHANNEL_COLORS[channel]}
                  onClick={() => setEditing(channel)}
                />
              ))}
            </SimpleGrid>
          </Stack>
        ) : (
          <Stack gap="xs">
            <Group justify="space-between" gap="xs" wrap="nowrap">
              <Group gap={6} wrap="nowrap">
                <ColorSwatch color={colors[editing]} size={16} />
                <Text size="xs" fw={600}>
                  Channel {editing + 1}
                </Text>
              </Group>
              <Anchor component="button" type="button" size="xs" onClick={() => setEditing(null)}>
                all channels
              </Anchor>
            </Group>
            <Divider />
            {/*
              `hexa` rather than `hex`, for the opacity slider — which is what
              makes this a tint rather than a replacement. At full opacity the
              note is the channel's colour outright; below it the layer's own
              colour field shows through, which is the point of authoring one
              under a MIDI layer at all.
            */}
            <ColorPicker
              format="hexa"
              alphaLabel="Opacity"
              value={colors[editing]}
              onChange={(color) => set(editing, color)}
              swatches={DEFAULT_CHANNEL_COLORS}
              swatchesPerRow={8}
              fullWidth
            />
            <Text size="xs" c="dimmed" lh={1.35}>
              Opacity blends this channel with the colour field underneath. It does not
              change what the layer covers.
            </Text>
          </Stack>
        )}
      </Popover.Dropdown>
    </Popover>
  );
});

/**
 * One channel: its colour, with its number over it.
 *
 * The number is drawn on the swatch rather than beside it, because sixteen
 * labels beside sixteen squares is a table and the thing being scanned is the
 * colour. Both text colours are drawn at once, one clipped to nothing — there is
 * no luminance test here, just a shadow dark enough for a light square and a
 * weight heavy enough for a dark one.
 */
function Square({
  channel,
  color,
  onClick,
}: {
  channel: number;
  color: string;
  onClick: () => void;
}) {
  return (
    <UnstyledButton onClick={onClick} aria-label={`Channel ${channel + 1} colour`}>
      <ColorSwatch color={color} radius="sm" size={46}>
        <Text size="xs" fw={700} c="white" style={{ textShadow: "0 1px 2px rgba(0,0,0,0.9)" }}>
          {channel + 1}
        </Text>
      </ColorSwatch>
    </UnstyledButton>
  );
}
