---
tags:
  - appearance
---
# `cursor_animation`

{{since('nightly')}}

Animates the text cursor when it jumps somewhere else or changes shape.
The animation is purely visual: the position of the cursor that the
terminal and applications see is never changed.

The feature is off unless `enabled = true` is set. It has two independent
parts which can be combined:

* `motion`: a trail drawn between the old and the new cursor position
  when the cursor moves a long enough distance.
* `mode_change`: an effect drawn around the cursor when its shape changes,
  for example when Neovim switches between the block cursor of Normal mode
  and the bar cursor of Insert mode.

A minimal, Kitty-like cursor trail:

```lua
config.cursor_animation = {
  enabled = true,
  fps = 120,

  motion = {
    preset = 'Tail',
    duration_ms = 90,
    easing = 'EaseOutCirc',
    min_distance = 1.5,
    blur = 2.0,

    tail = {
      max_length = 0.20,
    },
  },

  mode_change = {
    preset = 'None',
  },
}
```

A more stylised combination:

```lua
config.cursor_animation = {
  enabled = true,
  fps = 120,

  motion = {
    preset = 'Warp',
    duration_ms = 150,
    easing = 'EaseOutCirc',
    min_distance = 1.0,

    warp = {
      trail_size = 0.8,
      thickness = 1.0,
      thickness_x = 0.9,
      fade = true,
      fade_exponent = 5.0,
    },
  },

  mode_change = {
    preset = 'Ripple',
    duration_ms = 150,
    easing = 'EaseOutCirc',
    max_radius = 0.026,
    ring_thickness = 0.02,
    blur = 3.5,
    animation_start_offset = 0.01,
  },
}
```

## Top level fields

* `enabled` - `true` to turn the animations on. Default `false`.
* `fps` - the frame rate used while an effect is running, in the range
  1-480. When omitted, [animation_fps](animation_fps.md) is used. Because
  the default `animation_fps` is only 10, which shows at most one frame of a
  90ms trail, setting `fps` to 60 or 120 is recommended. Frames are only
  requested while an effect is on screen; once it finishes, the cursor
  animation stops requesting frames entirely.
* `motion` - the movement effect, described below.
* `mode_change` - the shape change effect, described below.

## Units

* `min_distance` is measured in cell heights, between the centers of the
  old and new cursor. The default of `1.5` means that moving by a
  character or two does not draw a trail, while jumps such as `gg`, `G`,
  `0`, `$` or search matches do.
* `blur` is in pixels (see [Edge softness](#edge-softness)).
* `tail.max_length`, `max_radius`, `max_size` and `ring_thickness` use the
  units of the shaders they are modelled on: a fraction of half of the
  window height. For example, `0.05` in a 1000 pixel tall window is 25
  pixels.
* `warp.trail_size`, `warp.thickness`, `warp.thickness_x` and
  `sweep.trail_length` are fractions (0.0 - 1.0).

## `motion`

* `preset` - one of `"None"`, `"Tail"` (the default), `"Warp"` or `"Sweep"`.
* `duration_ms` - length of the animation. Defaults to the preset's value.
* `easing` - the easing curve. Defaults to the preset's value.
* `min_distance` - minimum distance before a trail is drawn. Default `1.5`.
* `color` - the color of the trail. When omitted, the effective color of
  the cursor is used, including colors set by the application, the color
  scheme or
  [force_reverse_video_cursor](force_reverse_video_cursor.md).
* `blur` - edge softness in pixels. Defaults to the preset's value.
* `tail.max_length` - Tail: the length a long jump travels before the end
  of the trail starts to follow. Default `0.2`.
* `warp.trail_size` - Warp: `0.0` moves all corners together, `1.0` makes
  the leading corners jump immediately. Default `0.8`.
* `warp.thickness` / `warp.thickness_x` - Warp: height and width of the
  stretched shape relative to the cursor. Defaults `1.0` and `0.9`.
* `warp.fade` / `warp.fade_exponent` - Warp: fade the shape out towards the
  old position. Defaults `false` and `5.0`.
* `sweep.trail_length` - Sweep: how much of the distance back to the old
  position the trail initially covers. Default `0.5`.

| Preset | `duration_ms` | `easing` | `blur` | Description |
|--------|---------------|----------|--------|-------------|
| `Tail` | 90 | `EaseOutCirc` | 2 | A comet-like trail. The front and back of the trail move with separate progress curves, so on long jumps the front arrives quickly and the back catches up. |
| `Warp` | 200 | `EaseOutCirc` | 1 | The cursor is stretched between the old and the new position, with the leading corners arriving first. |
| `Sweep` | 200 | `EaseOutCubic` | 2 | A trail attached to the new cursor shrinks back into it. |

Straight moves draw a rectangular trail; diagonal moves draw a
parallelogram between the two positions. The trail is drawn beneath the
text, never covers the cursor itself, and is clipped to the pane, so it
cannot spill into neighbouring panes, the tab bar or the padding.

## `mode_change`

* `preset` - one of `"None"` (the default), `"Ripple"`, `"SonicBoom"`,
  `"RectangleBoom"` or `"RectangleRipple"`.
* `duration_ms` - default `150`.
* `easing` - default `EaseOutCirc`.
* `color` - when omitted the effective cursor color is used.
* `blur` - edge softness in pixels. Default `3.0`, or `1.0` for
  `RectangleRipple`.
* `animation_start_offset` - added to the animation progress (0.0 - 1.0);
  positive values skip the start of the animation. Default `0.0`.
* `max_radius` - `Ripple` and `SonicBoom` size. Default `0.05` for
  `Ripple` and `0.06` for `SonicBoom`.
* `max_size` - how far `RectangleBoom` and `RectangleRipple` grow beyond
  the cursor. Default `0.05`.
* `ring_thickness` - `Ripple` and `RectangleRipple` ring thickness.
  Default `0.02`.

| Preset | Description |
|--------|-------------|
| `Ripple` | An expanding, fading ring. |
| `SonicBoom` | An expanding, fading disc. |
| `RectangleBoom` | A filled rectangle that grows out of the cursor and fades. |
| `RectangleRipple` | A rectangular ring that grows out of the cursor and fades. |

The effect is triggered by a change between the block, bar and underline
cursor shapes. Switching between the blinking and steady variant of the
same shape does not trigger it, and neither does the cursor blinking,
focus changes, switching panes or reloading the configuration.

## Easing

`easing` accepts `"Linear"`, `"Ease"`, `"EaseIn"`, `"EaseOut"`,
`"EaseInOut"`, `"EaseOutCubic"`, `"EaseOutCirc"`, `"EaseOutBack"` and
`"EaseOutElastic"`. `EaseOutBack` and `EaseOutElastic` overshoot before
settling.

## Edge softness

The effects are drawn natively as a handful of polygons using WezTerm's
normal renderer, so they work with both the OpenGL and WebGpu
[front_end](front_end.md)s and cost much less than a full screen shader.
`blur` does not apply a Gaussian blur: it adds a band of the given width
around each shape whose opacity falls off linearly, which gives a similar
soft edge.

When nothing is animating, the cursor animation does no work beyond
recording the cursor position, and does not cause any redraws.

## Acknowledgements

The presets and their default values are modelled on the Ghostty shaders
at <https://github.com/sahaj-b/ghostty-cursor-shaders> (MIT License,
Copyright (c) 2026 Sahaj Bhatt), reimplemented here as native geometry.
