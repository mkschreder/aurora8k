The current demo uses this trick:

```rust
▀
```

with foreground color for the upper half and background color for the lower half.

So each terminal character cell gives you only:

```text
1 pixel wide × 2 pixels tall
```

That means the **horizontal pixel size is still one full terminal character cell**, which is why it looks chunky.

You have four realistic options.

---

## Option 1: Zoom out your terminal font

The easiest fix:

```text
Ctrl + -
```

or lower the terminal font size.

Because the renderer is terminal-cell based, smaller font = smaller pixels.

No code changes. This preserves the current full RGB color quality.

---

## Option 2: Increase terminal resolution cap

The current code intentionally caps resolution here:

```rust
(c.min(112), r.saturating_sub(2).min(36))
```

That means it never renders wider than 112 columns or taller than 36 terminal rows.

You can raise it:

```rust
(c.min(180), r.saturating_sub(2).min(60))
```

or remove the caps:

```rust
(c, r.saturating_sub(2))
```

But this only helps if your terminal window is large and your CPU can keep up. It does **not** make each terminal cell smaller. It just uses more cells.

---

## Option 3: Use Braille characters for smaller monochrome/detail pixels

Unicode Braille characters can encode:

```text
2 pixels wide × 4 pixels tall
```

inside one terminal cell.

So compared to the current half-block mode:

```text
current:  1 × 2 = 2 samples per cell
braille:  2 × 4 = 8 samples per cell
```

That gives much finer geometry.

The downside: Braille gives you basically **one foreground color per character**, not separate RGB colors per subpixel. So you get finer detail, but worse color precision.

A Braille renderer would replace the current `render()` inner loop with something conceptually like this:

```rust
const DOTS: [u8; 8] = [
    0x01, // x=0,y=0 dot 1
    0x08, // x=1,y=0 dot 4
    0x02, // x=0,y=1 dot 2
    0x10, // x=1,y=1 dot 5
    0x04, // x=0,y=2 dot 3
    0x20, // x=1,y=2 dot 6
    0x40, // x=0,y=3 dot 7
    0x80, // x=1,y=3 dot 8
];
```

Then for each terminal cell, cast 8 rays instead of 2, convert each sample to brightness, set Braille dots where brightness is high, and average the color.

Pseudo-shape:

```rust
let mut mask = 0u8;
let mut acc = V::new(0.0, 0.0, 0.0);
let mut count = 0.0;

for by in 0..4 {
    for bx in 0..2 {
        let virtual_x = x * 2 + bx;
        let virtual_y = y * 4 + by;

        let rd = ...;
        let c = shade(ro, rd, t);

        let lum = c.x * 0.299 + c.y * 0.587 + c.z * 0.114;

        if lum > 0.08 {
            mask |= DOTS[by * 2 + bx];
            acc = acc + c;
            count += 1.0;
        }
    }
}

let color = if count > 0.0 {
    acc / count
} else {
    sky_color_or_black
};

let ch = char::from_u32(0x2800 + mask as u32).unwrap();
```

This would make the image look much more detailed, but less “smoothly colored.”

---

## Option 4: Use terminal graphics protocol: Sixel / Kitty / iTerm2

This is the best real answer if you want genuinely smaller pixels while staying in the terminal.

Instead of abusing text characters as pixels, generate a real RGB bitmap on CPU and send it to the terminal using:

```text
Sixel
Kitty graphics protocol
iTerm2 inline images
```

Then you can render something like:

```text
320 × 180
640 × 360
```

actual pixels.

Still CPU-only. Still no GPU. But no longer pure ANSI text rendering.

This gives the best visual quality by far.

The downside is compatibility:

* Kitty graphics works in Kitty, WezTerm, Ghostty, newer terminals.
* Sixel works in mlterm, foot, xterm with sixel enabled, some others.
* GNOME Terminal historically does not support these well.

---

## My recommendation

For this demo, I would do this progression:

### Fastest improvement

Change terminal font size and raise the cap:

```rust
(c.min(180), r.saturating_sub(2).min(60))
```

### Best text-mode improvement

Add a second renderer using Braille mode:

```bash
./aurora8k --braille
```

This gives 4× more spatial detail per terminal cell than the current renderer.

### Best visual improvement

Add Kitty/Sixel output and render to an actual RGB framebuffer.

That is the point where it stops looking like “big terminal squares” and starts looking like a real tiny CPU demo.

The hard truth: with normal ANSI text, one terminal cell is the smallest addressable unit horizontally. You can fake more vertical resolution with block characters and fake more detail with Braille, but you cannot get true small square pixels without either shrinking the font or using a terminal image protocol.
