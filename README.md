# aurora8k

Two size-coded CPU raytracers for Linux x86-64. No GPU. No libc. No crates. Pure math and raw syscalls, packed into the smallest possible ELF binaries.

| Variant | Binary size | Output |
|---|---|---|
| `aurora8k` | **8,123 bytes** (within 8 KiB) | ANSI half-block characters, direct to terminal |
| `aurora16k` | **~16.5 KB UPX-packed** | Raw RGB24 stream → pipe to GStreamer or ffmpeg |

Both render the same class of scene: a crystal temple in a forest clearing, with rolling terrain, conifer forest, aurora ribbons, reflective water, fireflies, volumetric mist, energy beam, gateway arches, standing stones, moon shaft, and animated camera choreography. The 16k variant is the fuller scene with more procedural systems.

---

## Quick start

```bash
# Build everything
make

# Run the 8k terminal demo
./aurora8k

# Stream the 16k variant to a window (requires GStreamer)
make run16

# Record 90 seconds to MP4 (requires ffmpeg)
make record16
```

---

## Building

The Makefile handles all variants. Prerequisites: `rustc` (stable, edition 2021), UPX at `/tmp/upx-5.1.1-amd64_linux/upx` (override with `UPX=...`), GStreamer for `run16`, ffmpeg for `record16`.

```
make              # aurora8k + aurora8k_packed + aurora16k (UPX)
make aurora8k     # custom minimal ELF, ~8.1 KB uncompressed
make aurora16k    # UPX-packed, targets ≤16 KB
make run          # build and run aurora8k in current terminal
make run16        # build aurora16k_standard and pipe to GStreamer
make record16     # record 90 s to aurora16k.mp4 via ffmpeg
make clean
```

For profiling (requires `~/.cargo/bin/flamegraph` and `perf`):

```
make profile8     # produces flamegraph8k.svg
make profile16    # produces flamegraph16k.svg
```

---

## Running aurora8k (terminal renderer)

`aurora8k` renders directly to the terminal using ANSI true-color escape codes and Unicode half-block characters (`▀`). Each character cell encodes two pixel rows: foreground color for the upper half, background color for the lower half.

```bash
./aurora8k
```

The renderer adapts to the terminal window size at startup. The demo runs for 90 seconds then exits.

**Tips for best image quality:**

- Use a small font size. Each character cell is one pixel wide; smaller font = finer pixels.
- A dark-background terminal (e.g. kitty, alacritty, WezTerm) looks best.
- The demo does not require sixel or kitty graphics support — any 24-bit color terminal works.

---

## Running aurora16k (raw RGB stream)

`aurora16k` writes raw RGB24 frames (320×180 pixels, 172,800 bytes/frame) to stdout. It does not open a window; connect it to a display tool via a pipe.

### GStreamer (live preview)

```bash
./aurora16k | gst-launch-1.0 fdsrc fd=0 \
  ! video/x-raw,format=RGB,width=320,height=180,framerate=60/1 \
  ! videoconvert ! autovideosink
```

### ffmpeg (record to file)

```bash
./aurora16k | ffmpeg -f rawvideo -pixel_format rgb24 \
  -video_size 320x180 -framerate 30 -i pipe:0 \
  -vf scale=1280:720 -c:v libx264 -crf 18 aurora16k.mp4
```

### Change resolution

Edit the two constants at the top of `aurora16k.rs`:

```rust
const W:  usize = 320;
const PH: usize = 180;
```

Then update the matching width/height in your pipeline string. Higher resolution reduces frame rate proportionally.

---

## Architecture

### No standard library, no libc

Both binaries use `#![no_std]` and `#![no_main]`. There is no dynamic linker, no PLT, no GOT, no C runtime. The entry point is a hand-written `_start` stub in `global_asm!` that calls `aurora_entry`, which calls `run()`, which calls `SYS_exit` when done.

### sys.rs — the system layer

All platform bindings live in `sys.rs`, which is `mod`-included by both main files. It provides:

| Symbol | What it does |
|---|---|
| `sys2`, `sys3` | Raw `syscall` wrappers (2- and 3-argument) via `core::arch::asm!` |
| `fast_floor` | Integer-truncation floor, avoids `floorf` PLT call |
| `fast_sin` | Range-reduction + 3-term Chebyshev minimax polynomial, max error < 2×10⁻⁴ |
| `fast_atan2` | Vigna's 2-term atan2, max error ≈ 0.004 rad |
| `F32Ext` | `sqrt` via `sqrtss` SSE instruction; `sin`/`cos`/`sin_cos`/`atan2` wrappers |
| `clock_monotonic` | `SYS_clock_gettime(CLOCK_MONOTONIC)` |
| `elapsed` | Wall-clock seconds since a `Timespec` |
| `sleep_ms` | `SYS_nanosleep` |
| `term_size` / `term_winsize` | `TIOCGWINSZ` ioctl, returns character grid and pixel dimensions |
| `write_raw` | `SYS_write(1, ptr, len)` via direct inline assembly (LTO-safe) |
| `Out` | Write-only byte buffer over a static BSS region |
| `Term` | RAII guard: enters/exits alternate screen buffer |

### Static BSS framebuffer — no mmap needed

A common pattern for no-std programs is to `mmap` anonymous memory for large buffers. We do not do this. Instead both variants declare their working memory as `static mut` arrays:

```rust
// sys.rs — ANSI output buffer for aurora8k (2 MiB, NOBITS in ELF)
static mut BUF: [u8; 1 << 21] = [0u8; 1 << 21];

// aurora16k.rs — RGB pixel framebuffer (320×180×3 = 169 KiB, NOBITS in ELF)
static mut FRAMEBUF: [u8; W * PH * 3] = [0u8; W * PH * 3];
```

Because these are zero-initialized statics, the linker places them in the `.bss` section, which is marked `NOBITS` in the ELF. The kernel maps the pages zeroed at load time. The arrays cost **zero bytes** in the ELF file. No `mmap` syscall. No `sys6`. No register-clobbering hazards.

### Linker scripts

Two custom linker scripts minimize ELF overhead:

- **`linker.ld`** — used for `aurora8k`. Merges ELF header, PHDRs, `.text`, and `.rodata` into one contiguous `RX` segment starting at file offset 0 with no page gaps. A second `PT_LOAD` covers `.bss` (NOBITS). Total header overhead: 176 bytes (two PHDRs).
- **`linker-upx.ld`** — used for both `*_standard` and `aurora16k`. Appends `/DISCARD/` rules to the default linker script, stripping `.eh_frame`, `.comment`, `.note`, `.got`, `.plt`, and dynamic sections. This produces a standard UPX-compatible ELF layout.

### Compiler flags

```
-C opt-level=z          # optimise for size, not speed
-C panic=abort          # no unwinding machinery
-C lto=fat              # whole-program LTO across all codegen units
-C codegen-units=1      # single CGU, allows full inlining
-C strip=symbols        # strip symbol table
-C relocation-model=static
-C link-arg=-nostdlib
-C link-arg=-Wl,--build-id=none
-C link-arg=-Wl,--no-eh-frame-hdr
```

After linking, `strip --strip-section-headers` removes the section header table (not needed at runtime) from the `aurora8k` binary.

### UPX compression

`aurora16k` is packed with UPX using the NRV2D algorithm at maximum compression:

```bash
upx --nrv2d -9 --force aurora16k
```

UPX prepends a decompression stub. At runtime the stub decompresses the payload into an anonymous mapping, fixes up entry-point offsets, and jumps in. The executable looks like a normal ELF to the kernel.

---

## Scene and rendering

Both scenes are raymarched using Signed Distance Functions. The rendering pipeline:

1. **`map(p)`** — evaluates all SDFs at point `p`, returns the nearest surface distance and a material ID.
2. **`march(ro, rd)`** — sphere-traces from ray origin `ro` along direction `rd` with an 0.80× safety multiplier (the scene uses approximate SDFs, so pure sphere-tracing would overshoot).
3. **`shade(ro, rd)`** — computes ambient occlusion, diffuse, specular (Blinn-Phong with `pow32`/`pow38`), Fresnel (clamped to [0,1]), rim lighting, three light sources with soft shadows, subsurface scattering, and material-specific effects.
4. **`tonemap(c, t)`** — ACES-inspired filmic curve, slight warm tint, gamma encode to 8-bit per channel.

### aurora8k scene elements

- Rolling terrain with `noise3`-based height field
- Hashed conifer forest (per-sector random seeds)
- Distant mountains
- Aurora ribbons in the sky
- Central crystal temple: `sd_octa` gem, torus rings, crystal shards, gateway arches, standing stones, rune ring ground glow
- Animated `smin`-blended energy beam
- Moon shaft and atmospheric haze
- Reflective water pool
- Animated firefly particles
- Camera choreography over four phases

### Math approximations

The demo uses no `libm`. All transcendentals are hand-coded:

| Function | Method | Max error |
|---|---|---|
| `sqrt` | SSE `sqrtss` instruction | Exact (hardware) |
| `sin` / `cos` | Range-reduce + Chebyshev minimax degree-5 odd polynomial | < 2×10⁻⁴ |
| `atan2` | Vigna two-term polynomial | ≈ 0.004 rad |
| `floor` | Integer truncation with negative-value correction | Exact for \|x\| < 2³¹ |

`fast_sin_4` (SSE2 4-wide SIMD sine) is available in `aurora16k.rs` for performance-critical loops.

### MapCtx — frame-level precomputation

Both variants precompute per-frame constants into a `MapCtx` struct before the pixel loop:

```rust
struct MapCtx {
    t:           f32,   // time in seconds
    cam_phase:   u32,   // which of the 4 camera choreography phases
    // precomputed noise samples, light positions, aurora state, etc.
}
```

This prevents redundant evaluation of expensive noise/trig across thousands of `map()` calls per frame.

---

## Size engineering

Getting a visually rich raymarcher under 8 KiB uncompressed required iterative measurement and trimming. Key techniques:

- **Single-file compilation** — `aurora8k.rs` `#[path]`-includes `sys.rs` as a module; one `rustc` invocation, one object file, LTO eliminates every unused function.
- **No generics on hot paths** — each generic monomorphisation adds size; all rendering math uses concrete `f32` or the `V` (vec3) struct.
- **`fmn` / `fmx` inlines** — hand-written min/max avoid the `f32::min` / `f32::max` PLT symbols.
- **Merged `sin_cos`** — one range-reduction feeds both sin and cos results.
- **UPX NRV2D** — repetitive ANSI escape sequences in `aurora8k` compress extremely well (repeated `\x1b[38;2;` patterns). The aurora16k RGB data is less compressible but the code section still benefits.
- **`strip --strip-section-headers`** — removes the 64-byte section header table and the section name string table from the uncompressed `aurora8k` binary.
- **Custom `linker.ld`** — eliminates the page-aligned gap between the ELF header/PHDRs and `.text` that the default linker script inserts (saves ~3.5 KiB).

---

## File layout

```
aurora8k/
├── aurora8k.rs       ANSI terminal demo (targets ≤8192 bytes uncompressed)
├── aurora16k.rs      RGB streaming demo (targets ≤16384 bytes UPX-packed)
├── sys.rs            System layer: syscalls, math, I/O, timing
├── linker.ld         Minimal ELF linker script (aurora8k, no page gap)
├── linker-upx.ld     UPX-compatible linker script (aurora16k + aurora8k_packed)
└── Makefile
```

---

## Platform requirements

- Linux x86-64 kernel ≥ 3.x (uses `SYS_clock_gettime`, `SYS_write`, `SYS_nanosleep`, `SYS_ioctl`)
- SSE2 capable CPU (any x86-64 CPU since 2003)
- `rustc` stable, edition 2021
- UPX 3.96+ for the packed targets
- GStreamer with `autovideosink` for `make run16`
- ffmpeg for `make record16`

No GPU. No display server. No X11. No Wayland. No sound.
