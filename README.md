# Aurora16k: A 16k SDF Demo In GLSL + Rust

This is a 16k demo using GLSL and rust.

There are no meshes, no textures, no model files, no image assets, no audio in this snippet, and no CPU-side scene graph.

The GPU fragment shader is the scene.

---


# High-Level Architecture

The program is split into two conceptual parts:

```text
Rust no_std host
    |
    | creates EGL pbuffer context
    | creates OpenGL 3.3 Core context
    | compiles vertex shader
    | compiles fragment shader
    | draws fullscreen triangle
    | reads RGB pixels back with glReadPixels
    | writes raw RGB frames to stdout
    v

GLSL fragment shader
    |
    | for each pixel:
    |     build camera ray
    |     raymarch procedural SDF scene
    |     compute lighting/shadows/AO/fog/glow
    |     output final color
    v

ffplay / ffmpeg
    |
    | consumes raw RGB24 stream
    v

live preview or video file
````

The Rust side does not render the scene directly. It only sets up the environment and repeatedly invokes the shader.

The shader is responsible for:

* all geometry
* all animation
* all materials
* all lighting
* all sky rendering
* all atmosphere
* all camera motion
* all final image processing

---

# Render Pipeline Diagram

```svg
<svg width="920" height="520" viewBox="0 0 920 520" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <marker id="arrow" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto">
      <path d="M0,0 L0,6 L9,3 z" fill="#333"/>
    </marker>
    <style>
      .box { fill:#f7f7fb; stroke:#222; stroke-width:1.5; rx:10; }
      .gpu { fill:#eef8ff; stroke:#1b5d89; stroke-width:1.5; rx:10; }
      .cpu { fill:#fff7e8; stroke:#9b6b16; stroke-width:1.5; rx:10; }
      .out { fill:#eef9ee; stroke:#2d7d35; stroke-width:1.5; rx:10; }
      .txt { font-family:monospace; font-size:14px; fill:#111; }
      .small { font-family:monospace; font-size:12px; fill:#333; }
      .arrow { stroke:#333; stroke-width:1.5; marker-end:url(#arrow); fill:none; }
    </style>
  </defs>

  <rect x="30" y="40" width="220" height="100" class="cpu"/>
  <text x="50" y="70" class="txt">Rust no_std host</text>
  <text x="50" y="95" class="small">EGL pbuffer</text>
  <text x="50" y="115" class="small">OpenGL 3.3 Core</text>

  <rect x="350" y="40" width="230" height="100" class="gpu"/>
  <text x="370" y="70" class="txt">Shader program</text>
  <text x="370" y="95" class="small">fullscreen triangle</text>
  <text x="370" y="115" class="small">fragment shader per pixel</text>

  <rect x="670" y="40" width="210" height="100" class="gpu"/>
  <text x="690" y="70" class="txt">Framebuffer</text>
  <text x="690" y="95" class="small">640 x 360</text>
  <text x="690" y="115" class="small">RGB pixels</text>

  <rect x="350" y="220" width="230" height="120" class="gpu"/>
  <text x="370" y="250" class="txt">Fragment shader</text>
  <text x="370" y="275" class="small">camera ray</text>
  <text x="370" y="295" class="small">raymarch map_()</text>
  <text x="370" y="315" class="small">shade_()</text>

  <rect x="670" y="220" width="210" height="120" class="cpu"/>
  <text x="690" y="250" class="txt">glReadPixels</text>
  <text x="690" y="275" class="small">bottom-to-top rows</text>
  <text x="690" y="295" class="small">FRAMEBUF</text>
  <text x="690" y="315" class="small">then row flip</text>

  <rect x="350" y="400" width="230" height="80" class="cpu"/>
  <text x="370" y="430" class="txt">stdout raw RGB24</text>
  <text x="370" y="455" class="small">write_raw(FLIPBUF)</text>

  <rect x="670" y="400" width="210" height="80" class="out"/>
  <text x="690" y="430" class="txt">ffplay / ffmpeg</text>
  <text x="690" y="455" class="small">preview or encode</text>

  <path d="M250,90 L350,90" class="arrow"/>
  <path d="M580,90 L670,90" class="arrow"/>
  <path d="M465,140 L465,220" class="arrow"/>
  <path d="M580,280 L670,280" class="arrow"/>
  <path d="M775,340 L775,380 L580,440" class="arrow"/>
  <path d="M580,440 L670,440" class="arrow"/>
</svg>
```

---

# The Rust Host Program

The Rust side is intentionally minimal. It exists to:

1. Create a headless OpenGL context.
2. Compile the GLSL shaders.
3. Render a fullscreen triangle.
4. Read the pixels.
5. Stream them as raw RGB frames.

The host is `#![no_std]` and `#![no_main]`, so it does not use Rust's standard runtime or normal `main`.

```rust
#![no_std]
#![no_main]

mod sys;
use sys::{clock_monotonic, elapsed, write_raw, write_stderr};
```

The missing `sys` module presumably provides:

* raw Linux syscall entry point
* monotonic clock access
* stdout/stderr writing
* process startup and exit
* possibly panic handling

The Rust code itself avoids libc, but it links against EGL and GL dynamically:

```rust
#[link(name = "EGL")]
extern "C" { ... }

#[link(name = "GL")]
extern "C" { ... }
```

That means the final program depends on the system's OpenGL/EGL libraries being available at runtime.

---

## Resolution

```rust
const W:  usize = 640;
const PH: usize = 360;
```

The shader renders at fixed 640x360.

That is a good demo resolution:

* 16:9 aspect ratio
* low enough for real-time raymarching
* high enough to look like a real image
* raw RGB frame size is manageable

Frame size:

```text
640 * 360 * 3 = 691,200 bytes per frame
```

At 30 FPS, raw output bandwidth is:

```text
691,200 * 30 = 20,736,000 bytes/s
≈ 19.8 MiB/s
```

That is reasonable for a pipe to `ffplay` or `ffmpeg`.

---

## Frame Buffers

```rust
static mut FRAMEBUF: [u8; W * PH * 3] = [0u8; W * PH * 3];
static mut FLIPBUF:  [u8; W * PH * 3] = [0u8; W * PH * 3];
```

Two static buffers are used:

* `FRAMEBUF`: receives pixels from `glReadPixels`
* `FLIPBUF`: receives the vertically flipped image for stdout

These are in `.bss`, so they do not increase the executable size much. The executable stores only the fact that the memory must be zero-initialized.

OpenGL returns pixel rows from bottom to top. Raw video tools normally expect top-to-bottom rows. Therefore the program flips the rows manually before writing.

---

## EGL Setup

The host creates a headless EGL pbuffer surface:

```rust
let dpy = eglGetDisplay(core::ptr::null_mut());
eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut());
eglBindAPI(EGL_OPENGL_API);
```

This asks EGL for the default display and selects the OpenGL API.

Then it chooses a config:

```rust
let cfg_attrs: [i32; 7] = [
    EGL_SURFACE_TYPE,    EGL_PBUFFER_BIT,
    EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
    EGL_DEPTH_SIZE,      0,
    EGL_NONE,
];
```

The important request is:

```text
EGL_SURFACE_TYPE    = EGL_PBUFFER_BIT
EGL_RENDERABLE_TYPE = EGL_OPENGL_BIT
```

This means:

> Give me an offscreen pixel buffer surface that supports OpenGL rendering.

No window system is needed.

Then the pbuffer is created:

```rust
let pb_attrs: [i32; 5] = [
    EGL_WIDTH,  W  as i32,
    EGL_HEIGHT, PH as i32,
    EGL_NONE,
];
let surface = eglCreatePbufferSurface(dpy, cfg, pb_attrs.as_ptr());
```

This is the offscreen framebuffer target.

---

## OpenGL Context

```rust
let ctx_attrs: [i32; 7] = [
    EGL_CONTEXT_MAJOR_VERSION,       3,
    EGL_CONTEXT_MINOR_VERSION,       3,
    EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
    EGL_NONE,
];
let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
eglMakeCurrent(dpy, surface, surface, ctx);
```

The shader uses:

```glsl
#version 330 core
```

So the host requests OpenGL 3.3 Core.

The pbuffer is made current for both draw and read surfaces.

---

## Shader Compilation

```rust
unsafe fn compile(src: &[u8], kind: u32) -> u32 {
    let sh = glCreateShader(kind);
    let ptr = src.as_ptr();
    let len = src.len() as i32;
    glShaderSource(sh, 1, &ptr, &len);
    glCompileShader(sh);
    ...
    sh
}
```

The shader source is passed directly as bytes plus length.

This means the shader does not need to be null-terminated.

The shaders are embedded with:

```rust
const VERT: &[u8] = include_bytes!("aurora16k.vert");
const FRAG: &[u8] = include_bytes!("aurora16k.frag");
```

So even though the source files are separate during development, they become embedded in the final executable.

---

## Fullscreen Triangle

The program draws:

```rust
glDrawArrays(GL_TRIANGLES, 0, 3);
```

There is no vertex buffer. The vertex shader presumably uses `gl_VertexID` to generate a fullscreen triangle.

This is a common trick.

A fullscreen triangle is preferred over a fullscreen quad because:

* only 3 vertices
* no index buffer
* no VBO
* no diagonal seam
* less setup
* works naturally with `gl_VertexID`

Conceptually, the vertex shader produces something like:

```glsl
vec2 p;
if (gl_VertexID == 0) p = vec2(-1, -1);
if (gl_VertexID == 1) p = vec2( 3, -1);
if (gl_VertexID == 2) p = vec2(-1,  3);
```

That triangle covers the whole viewport.

OpenGL 3.3 Core requires a VAO even if no vertex attributes are used:

```rust
let mut vao: u32 = 0;
glGenVertexArrays(1, &mut vao);
glBindVertexArray(vao);
```

---

## Per-Frame Rendering

```rust
let t = if record { frame_t } else { elapsed(&start) };
if t > seconds { break; }

glUniform1f(t_loc, t);
glDrawArrays(GL_TRIANGLES, 0, 3);
```

The shader receives time as uniform `T`.

Resolution `R` is set once:

```rust
glUniform2f(r_loc, W as f32, PH as f32);
```

Every frame:

1. `T` is updated.
2. The fullscreen triangle is drawn.
3. The fragment shader runs once per pixel.
4. RGB pixels are read back.
5. Rows are flipped.
6. Raw RGB is written to stdout.

---

## Record Mode vs Live Mode

```rust
let t = if record { frame_t } else { elapsed(&start) };
```

In live mode, time comes from the monotonic clock.

In record mode, time is deterministic:

```rust
frame_t += 1.0 / 30.0;
```

This is good for encoding because every run generates identical frame times and exactly 30 FPS.

---

# The GLSL Shader

The GLSL fragment shader is the real demo.

At the top:

```glsl
#version 330 core
uniform float T;
uniform vec2 R;
out vec4 O;
```

* `T` is time in seconds.
* `R` is resolution.
* `O` is the final output color.

The shader is written in a sizecoding style:

* compact function names
* scalar constants
* minimal structs
* repeated procedural patterns
* many objects generated from polar repetition and hash functions

---

# Shader Execution Model

For each pixel, `main()` runs independently:

```glsl
void main() {
    Ctx cx = makeCtx();
    vec3 ro, f, r, u;
    cam_(ro, f, r, u);

    float asp = R.x / R.y;
    float px = (gl_FragCoord.x / R.x * 2. - 1.) * asp;
    float py = gl_FragCoord.y / R.y * 2. - 1.;

    vec3 rd = normalize(f * 1.35 + r * px + u * py);

    vec3 raw = shade_(ro, rd, cx) * vign;

    O = vec4(final_tonemapped_color, 1.);
}
```

Every pixel constructs a ray from the animated camera and calls `shade_()`.

---

# Main Shader Flow Diagram

```svg
<svg width="900" height="760" viewBox="0 0 900 760" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <marker id="arr2" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto">
      <path d="M0,0 L0,6 L9,3 z" fill="#222"/>
    </marker>
    <style>
      .box { fill:#f8f8ff; stroke:#222; stroke-width:1.5; rx:10; }
      .math { fill:#eef8ff; stroke:#1c638a; stroke-width:1.5; rx:10; }
      .scene { fill:#fff4e8; stroke:#9a6515; stroke-width:1.5; rx:10; }
      .light { fill:#f2fff0; stroke:#33813a; stroke-width:1.5; rx:10; }
      .txt { font-family:monospace; font-size:14px; fill:#111; }
      .small { font-family:monospace; font-size:12px; fill:#333; }
      .arrow { stroke:#222; stroke-width:1.5; marker-end:url(#arr2); fill:none; }
    </style>
  </defs>

  <rect x="330" y="20" width="240" height="70" class="box"/>
  <text x="355" y="50" class="txt">main()</text>
  <text x="355" y="72" class="small">per fragment / per pixel</text>

  <rect x="80" y="140" width="220" height="80" class="math"/>
  <text x="105" y="170" class="txt">makeCtx()</text>
  <text x="105" y="195" class="small">precompute sin/cos</text>

  <rect x="340" y="140" width="220" height="80" class="math"/>
  <text x="365" y="170" class="txt">cam_()</text>
  <text x="365" y="195" class="small">camera basis</text>

  <rect x="600" y="140" width="220" height="80" class="math"/>
  <text x="625" y="170" class="txt">screen ray</text>
  <text x="625" y="195" class="small">rd = f*1.35+r*px+u*py</text>

  <rect x="340" y="280" width="220" height="80" class="scene"/>
  <text x="365" y="310" class="txt">shade_(ro, rd)</text>
  <text x="365" y="335" class="small">raymarch and shade</text>

  <rect x="80" y="420" width="220" height="90" class="scene"/>
  <text x="105" y="450" class="txt">map_(p)</text>
  <text x="105" y="475" class="small">distance + material</text>
  <text x="105" y="495" class="small">terrain, temple, forest</text>

  <rect x="340" y="420" width="220" height="90" class="light"/>
  <text x="365" y="450" class="txt">norm_, shad_, ao_</text>
  <text x="365" y="475" class="small">normals, shadows</text>
  <text x="365" y="495" class="small">ambient occlusion</text>

  <rect x="600" y="420" width="220" height="90" class="light"/>
  <text x="625" y="450" class="txt">pal_, sky_</text>
  <text x="625" y="475" class="small">materials, sky</text>
  <text x="625" y="495" class="small">stars, moon, aurora</text>

  <rect x="340" y="600" width="220" height="80" class="box"/>
  <text x="365" y="630" class="txt">tone map</text>
  <text x="365" y="655" class="small">vignette, contrast, gamma</text>

  <path d="M450,90 L190,140" class="arrow"/>
  <path d="M450,90 L450,140" class="arrow"/>
  <path d="M450,90 L710,140" class="arrow"/>
  <path d="M450,220 L450,280" class="arrow"/>
  <path d="M710,220 L500,280" class="arrow"/>
  <path d="M340,320 L300,440" class="arrow"/>
  <path d="M450,360 L450,420" class="arrow"/>
  <path d="M560,320 L650,420" class="arrow"/>
  <path d="M450,510 L450,600" class="arrow"/>
</svg>
```

---

# Signed Distance Fields

The core geometric representation is a signed distance field.

An SDF function returns the distance from a point to the nearest surface.

```text
distance > 0    outside object
distance = 0    on surface
distance < 0    inside object
```

For example, a sphere centered at the origin with radius `r` is:

```glsl
length(p) - r
```

A raymarcher can safely advance along a ray by approximately the returned distance.

This is the core reason the shader can render complex procedural geometry without meshes.

---

# Basic Helper Functions

## `hash`

```glsl
float hash(float n) {
    return fract(sin(n) * 43758.5453);
}
```

This is a tiny deterministic pseudo-random function.

It is used for:

* star placement
* forest tree placement
* per-tree height variation
* rock/stone variation
* firefly placement
* material variation

It is not high-quality random noise, but it is small and good enough for visual randomness.

---

## `rep`

```glsl
float rep(float x, float c) {
    float s = x + .5 * c;
    return s - c * floor(s / c) - .5 * c;
}
```

This repeats a coordinate every `c` units and recenters it around zero.

It maps:

```text
... -1.5c, -0.5c, 0.5c, 1.5c ...
```

into the same local interval:

```text
[-0.5c, +0.5c]
```

This is used for:

* polar repetition of columns
* radial floor ornaments
* ring inlays
* repeated grid patterns

Repetition is a major sizecoding trick. One mathematical object becomes many visible objects.

---

## `noise3`

```glsl
float noise3(vec3 p) {
    return (sin(p.x * 1.7 + p.z * 2.3)
          + sin(p.x * 3.1 - p.y * 1.9 + p.z * .7) * .5
          + sin(p.x * 7.3 + p.y * 2.1 + p.z * 1.3) * .25) / 1.75;
}
```

This is not gradient noise or Perlin noise. It is a compact trigonometric pseudo-noise function.

It combines three sine waves at different frequencies and weights.

Purpose:

* material variation
* terrain color variation
* bark perturbation
* floor texture
* water ripple perturbation
* cheap natural-looking irregularity

The function is intentionally compact. Full value noise or simplex noise would be longer.

---

# Primitive SDFs

The shader defines several reusable distance primitives.

---

## Box

```glsl
float sd_box(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.)) + min(max(q.x, max(q.y, q.z)), 0.);
}
```

`b` is the half-size of the box.

This is used for:

* floor inlays
* radial markings
* rune stones
* gateway arches
* standing stones
* architectural blocks

---

## Octahedron

```glsl
float sd_octa(vec3 p, float s) {
    return (abs(p.x) + abs(p.y) + abs(p.z) - s) * .57735027;
}
```

This creates a diamond-like crystal shape.

The factor `.57735027` is approximately:

```text
1 / sqrt(3)
```

It improves the distance scaling.

Used for:

* main crystal gem
* secondary inner/outer gem
* magical shard/rune objects

---

## Torus

```glsl
float sd_torus(vec3 p, float r, float t) {
    return length(vec2(length(p.xz) - r, p.y)) - t;
}
```

A torus is a ring.

Parameters:

* `r`: major radius
* `t`: tube radius

Used for:

* rotating energy rings
* column caps
* circular ornaments

---

## Vertical Cylinder

```glsl
float sd_cyl_y(vec3 p, float r, float h) {
    float dx = length(p.xz) - r, dy = abs(p.y) - h;
    return min(max(dx, dy), 0.) + length(max(vec2(dx, dy), 0.));
}
```

Cylinder aligned along the Y axis.

Parameters:

* `r`: radius
* `h`: half-height

Used for:

* columns
* trunks
* distant standing stones
* altar cylinder

---

## Smooth Minimum

```glsl
float smin(float a, float b, float k) {
    float h = clamp(.5 + .5 * (b - a) / k, 0., 1.);
    return mix(b, a, h) - k * h * (1. - h);
}
```

This blends two SDFs smoothly instead of creating a hard union.

Hard union:

```glsl
min(a, b)
```

Smooth union:

```glsl
smin(a, b, k)
```

Used for the main crystal, where an octahedron and sphere are blended into a polished magical object.

---

# Raymarching

The main raymarch happens inside `shade_`.

Simplified:

```glsl
float depth = 0.;
for (int i = 0; i < 80; i++) {
    vec3 p = ro + rd * depth;
    vec2 h = map_(p, cx);

    if (h.x < .0015 * (1. + depth * .12)) {
        mat = h.y;
        break;
    }

    depth += clamp(h.x * .80, .006, .35);

    if (depth > 18. || i == 79)
        return sky_col + glow * .55;
}
```

The shader repeatedly:

1. Computes the current point on the ray.
2. Evaluates `map_()` at that point.
3. Receives nearest distance and material ID.
4. If distance is small enough, it hit a surface.
5. Otherwise it advances forward.

The step is:

```glsl
depth += clamp(h.x * .80, .006, .35);
```

The `.80` is a safety factor. A perfect SDF could step by exactly `h.x`, but this scene contains approximate distance fields:

* displaced terrain
* procedural waves
* unioned details
* repeated objects
* forest approximations

So stepping by 80% reduces overshoot artifacts.

The clamp:

```glsl
clamp(h.x * .80, .006, .35)
```

prevents:

* getting stuck with tiny steps
* taking dangerously huge steps through complex geometry

The hit threshold grows with distance:

```glsl
.0015 * (1. + depth * .12)
```

This reduces aliasing and missed hits at far distances.

---

# Raymarching Diagram

```svg
<svg width="900" height="380" viewBox="0 0 900 380" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <marker id="arr3" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto">
      <path d="M0,0 L0,6 L9,3 z" fill="#333"/>
    </marker>
    <style>
      .ray { stroke:#333; stroke-width:2; marker-end:url(#arr3); }
      .step { stroke:#1f77b4; stroke-width:2; stroke-dasharray:4 3; }
      .circle { fill:none; stroke:#1f77b4; stroke-width:1.5; opacity:.5; }
      .surface { fill:#ddd; stroke:#111; stroke-width:2; }
      .txt { font-family:monospace; font-size:13px; fill:#111; }
    </style>
  </defs>

  <path class="surface" d="M650,80 C770,120 790,260 660,310 C570,350 475,300 500,200 C520,120 580,65 650,80z"/>
  <line x1="80" y1="250" x2="810" y2="115" class="ray"/>

  <circle cx="120" cy="243" r="100" class="circle"/>
  <circle cx="315" cy="207" r="80" class="circle"/>
  <circle cx="470" cy="178" r="48" class="circle"/>
  <circle cx="565" cy="160" r="26" class="circle"/>
  <circle cx="615" cy="151" r="11" class="circle"/>

  <circle cx="120" cy="243" r="5" fill="#1f77b4"/>
  <circle cx="315" cy="207" r="5" fill="#1f77b4"/>
  <circle cx="470" cy="178" r="5" fill="#1f77b4"/>
  <circle cx="565" cy="160" r="5" fill="#1f77b4"/>
  <circle cx="615" cy="151" r="5" fill="#1f77b4"/>

  <text x="65" y="280" class="txt">ro</text>
  <text x="145" y="225" class="txt">step by SDF distance</text>
  <text x="510" y="140" class="txt">steps shrink near surface</text>
  <text x="640" y="150" class="txt">hit</text>
</svg>
```

---

# The `Ctx` Struct

```glsl
struct Ctx {
    float gem_sx, gem_cx, gem_sy, gem_cy,
          pulse_s,
          gm2_sx, gm2_cx, gm2_sy, gm2_cy,
          pulse2_s,
          r0_sx, r0_cx,
          r1_sz, r1_cz,
          r2_sy, r2_cy,
          r3_sx, r3_cx,
          floor_s,
          lp2_y;
};
```

`Ctx` stores precomputed animation values.

The shader calls trigonometric functions many times. Instead of recomputing the same `sin(T * factor)` and `cos(T * factor)` in every object, `makeCtx()` computes them once per pixel.

Strictly speaking, because this is fragment shader code, `makeCtx()` still runs per pixel. But within that pixel it avoids recomputing repeated terms inside `map_()`, `shade_()`, and lighting functions.

Examples:

```glsl
c.gem_sx = sin(T * .37);
c.gem_cx = cos(T * .37);
```

These are used to rotate the main gem.

```glsl
c.r0_sx = sin(T * .70);
c.r0_cx = cos(T * .70);
```

These are used both for the first rotating ring and the first moving light position.

```glsl
c.lp2_y = 1.3 + .8 * sin(T * 1.3);
```

This animates the second light vertically.

---

# Scene Construction

The scene function is:

```glsl
vec2 map_(vec3 p0, Ctx cx)
```

It returns:

```text
x = distance to nearest object
y = material ID
```

The helper:

```glsl
vec2 hitp(vec2 h, float d, float m) {
    return d < h.x ? vec2(d, m) : h;
}
```

keeps the closest object.

Conceptually:

```glsl
h = closest(h, terrain, material_4);
h = closest(h, crystal, material_1);
h = closest(h, rings, material_2);
h = closest(h, columns, material_3);
...
return h;
```

---

# Material ID Map

The shader uses these material IDs:

```text
1 = main crystal gem
2 = rotating cyan energy rings
3 = stone/gold architecture
4 = terrain / ground
5 = cyan floor inlays
6 = tree trunk / wood
7 = tree foliage
8 = water
9 = secondary purple crystal/rune objects
```

These IDs are consumed by:

* `pal_()` for base color
* `norm_()` for special terrain normals
* `ao_()` for special terrain occlusion
* `shade_()` for emission/specular/rim behavior

---

# Procedural Terrain

The terrain height function is:

```glsl
float terrain_h(float x, float z)
```

It returns the Y height of the terrain at horizontal coordinate `(x, z)`.

The terrain surface itself is inserted into the SDF world as:

```glsl
h = hitp(h, p0.y - terrain_h(p0.x, p0.z), 4.);
```

This is a heightfield distance approximation.

If:

```text
p0.y == terrain_h(p0.x, p0.z)
```

then the distance is zero.

If `p0.y` is above the terrain, distance is positive.

If `p0.y` is below the terrain, distance is negative.

---

## Terrain Flattening Around the Temple

```glsl
float flat_ = smoothstep(3., 7.5, length(vec2(x, z)));
```

This creates a radial mask.

Near the center:

```text
length(xz) < 3
flat_ ≈ 0
```

Farther away:

```text
length(xz) > 7.5
flat_ ≈ 1
```

The final terrain is:

```glsl
return -1.25 + waves * flat_;
```

That means the center is flat around `y = -1.25`, while the outer forest area becomes rolling terrain.

This is important compositionally:

* the temple needs a flat stage
* the forest needs natural ground variation

---

## Domain Warping

Before computing the main terrain waves, the code creates warp offsets:

```glsl
float wx = ...;
float wz = ...;
float nx = x + wx;
float nz = z + wz;
```

Then it uses `nx` and `nz` in the terrain formula.

This is called domain warping.

Instead of:

```glsl
height = f(x, z)
```

it does:

```glsl
height = f(x + warp_x, z + warp_z)
```

This breaks up obvious sine-wave regularity.

The result looks more natural than plain layered sine waves.

---

## Terrain Wave Stack

The final terrain wave is:

```glsl
sin(nx * .21 + nz * .18) * .40
+ sin(nx * .67 - nz * .43) * .22
+ sin((nx - nz) * 1.31) * .11
+ sin(nx * 2.3 + nz * 1.9) * .06
+ sin(nx * 4.7 - nz * 3.1) * .03
```

This is fractal-ish sine terrain:

* low frequencies define large hills
* higher frequencies add detail
* amplitudes decrease with frequency

---

# Terrain Diagram

```svg
<svg width="900" height="360" viewBox="0 0 900 360" xmlns="http://www.w3.org/2000/svg">
  <style>
    .ground { fill:none; stroke:#333; stroke-width:3; }
    .flat { fill:#eaf7ff; opacity:.7; stroke:#247; stroke-width:1; }
    .forest { fill:#eaffea; opacity:.7; stroke:#272; stroke-width:1; }
    .txt { font-family:monospace; font-size:13px; fill:#111; }
    .dash { stroke:#777; stroke-dasharray:5 4; }
  </style>

  <path class="ground" d="M40,230 C130,230 210,230 300,230 C350,220 390,210 440,225 C500,250 540,170 600,200 C670,240 720,150 860,185"/>
  <rect x="210" y="80" width="270" height="190" class="flat"/>
  <rect x="480" y="80" width="360" height="190" class="forest"/>
  <line x1="300" y1="70" x2="300" y2="285" class="dash"/>
  <line x1="480" y1="70" x2="480" y2="285" class="dash"/>

  <text x="250" y="55" class="txt">flat temple clearing</text>
  <text x="555" y="55" class="txt">rolling forest terrain</text>
  <text x="300" y="305" class="txt">smoothstep transition</text>
  <text x="40" y="255" class="txt">y = terrain_h(x,z)</text>
</svg>
```

---

# Main Scene Objects in `map_`

The `map_()` function is a sequence of object insertions.

---

## 1. Terrain

```glsl
h = hitp(h, p0.y - terrain_h(p0.x, p0.z), 4.);
```

The terrain is always present.

Material `4`.

---

## 2. Central Crystal Zone

```glsl
float a = atan(p0.z, p0.x), rr = length(p0.xz);

if (rr < 2.5) {
    ...
}
```

The central crystal and rotating rings are only evaluated when the point is near the center.

This is a performance optimization.

No need to evaluate central gem SDFs for points in the far forest.

---

## 3. Main Rotating Gem

The point is rotated around X and Y using precomputed sin/cos:

```glsl
vec3 gx =
    vec3(p0.x,
         cx.gem_cx * p0.y - cx.gem_sx * p0.z,
         cx.gem_sx * p0.y + cx.gem_cx * p0.z);

vec3 p =
    vec3(cx.gem_cy * gx.x + cx.gem_sy * gx.z,
         gx.y,
        -cx.gem_sy * gx.x + cx.gem_cy * gx.z);
```

This is an inverse transform applied to the sample point.

The gem itself is:

```glsl
smin(
    sd_octa(p, .95 + .08 * cx.pulse_s),
    length(p) - .72,
    .18
)
```

It consists of:

* an octahedron
* a sphere
* smooth blending
* animated pulse
* small trigonometric surface displacement

Material `1`.

The displacement is not a perfect SDF anymore, but the amplitude is small enough to work visually.

---

## 4. Secondary Crystal

```glsl
h = hitp(h, sd_octa(p2, .48 + .04 * cx.pulse2_s), 9.);
```

This is a smaller rotating octahedron/crystal using material `9`.

It gives an additional magical focal object.

---

## 5. Rotating Energy Rings

Four torus SDFs are created with different orientations:

```glsl
sd_torus(r0q, 1.18, .035)
sd_torus(r1q, 1.34, .026)
sd_torus(r2q, 1.52, .018)
sd_torus(r3q, 1.68, .013)
```

The radii increase and tube thickness decreases.

This makes nested energy rings around the gem.

Material `2`.

The fourth ring uses constants:

```glsl
const float C3Z = 0.71073, S3Z = 0.70360;
```

These are precomputed cosine/sine-like constants for a fixed rotation. They avoid calling `sin()`/`cos()` for that fixed tilt.

---

## 6. Eight Temple Columns

```glsl
if (rr > 1.8 && rr < 3.2) {
    float aa = rep(a, 2. * PI / 8.);
    vec3 q = vec3(rr - 2.35, p0.y + .15, aa * rr);
    ...
}
```

This is polar repetition.

Instead of placing eight columns individually, the shader folds angle `a` into one repeated sector.

The local coordinate:

```glsl
vec3 q = vec3(rr - 2.35, p0.y + .15, aa * rr);
```

means:

* `q.x`: radial offset from column ring radius
* `q.y`: vertical coordinate
* `q.z`: tangential coordinate

One cylinder plus two torus caps becomes eight columns.

```glsl
min(
    sd_cyl_y(q, .07, 1.1),
    min(
        sd_torus(q + vec3(0, -1.04, 0), .11, .025),
        sd_torus(q + vec3(0, 1.04, 0), .11, .025)
    )
)
```

Material `3`.

---

## 7. Repeating Floor Inlay Grid

```glsl
vec3 gv = vec3(
    rep(p0.x + .3 * cx.floor_s, .75),
    p0.y + 1.215,
    rep(p0.z, .75)
);

h = hitp(
    h,
    min(
        sd_box(gv, vec3(.20, .018, .018)),
        sd_box(gv, vec3(.018, .018, .20))
    ),
    5.
);
```

This creates a repeating cross pattern on the floor.

The X repetition drifts slightly with time:

```glsl
p0.x + .3 * cx.floor_s
```

Material `5`.

This reads as glowing cyan runic floor inlay.

---

## 8. Radial Ring Inlay

```glsl
h = hitp(
    h,
    sd_box(
        vec3(rr - 1.90,
             p0.y + 1.215,
             rep(a, 2. * PI / 16.) * rr),
        vec3(.022, .022, .095)
    ),
    5.
);
```

This creates 16 repeated radial inlay marks around the central ring.

Again, no array of 16 objects is needed.

Polar repetition handles it.

---

## 9. Inner Floating Rune Shards

```glsl
if (rr > 1.2 && rr < 2.3) {
    float s12 = 2. * PI / 12.;
    float sid = floor(a / s12 + .5);
    float aa12 = a - sid * s12;
    float hs = hash(sid * 7.);

    h = hitp(
        h,
        sd_box(
            rot_z(
                vec3(rr - 1.72,
                     p0.y - .28 * sin(hs * 6. + T * .5),
                     aa12 * rr),
                .3 + hs * .55
            ),
            vec3(.04, .33, .09)
        ),
        9.
    );
}
```

This creates 12 animated vertical shards or runes.

Each sector gets a hash `hs`, which controls:

* vertical bobbing phase
* rotation
* appearance variation

Material `9`.

---

## 10. Four Gateway Arches

```glsl
if (rr > 2.0 && rr < 5.0) {
    float a4 = rep(a, PI * .5);
    vec3 gp = vec3(rr - 3.45, p0.y + 1.25, a4 * rr * 1.85);

    h = hitp(
        h,
        min(
            min(
                sd_box(gp + vec3(0, 0, -.55), vec3(.10, 1., .10)),
                sd_box(gp + vec3(0, 0, .55), vec3(.10, 1., .10))
            ),
            sd_box(gp + vec3(0, 1.02, 0), vec3(.10, .13, .67))
        ),
        3.
    );
}
```

This creates four repeated gateway structures around the temple.

Each gateway consists of:

* left pillar
* right pillar
* top lintel

Material `3`.

---

## 11. Seven Standing Stones

```glsl
if (rr > 3.5 && rr < 5.8) {
    float s7 = 2. * PI / 7.;
    float sid7 = floor(a / s7 + .5);
    float a7 = a - sid7 * s7;
    float ss = sid7 * 9.3;

    h = hitp(
        h,
        sd_box(
            rot_z(
                vec3(rr - 4.75, p0.y + 1.25, a7 * rr * 2.),
                .08 * sin(ss)
            ),
            vec3(.10, .70 + hash(ss) * .30, .08)
        ),
        3.
    );
}
```

Seven irregular standing stones are placed in a ring.

Hash controls height variation.

A small rotation makes them less mechanical.

Material `3`.

---

## 12. Water Pool

```glsl
float pr = length(p0.xz) - 2.55;
float wave = .013 * (
        sin(p0.x * 4.2 + T * 2.1)
      + sin(p0.z * 3.7 - T * 1.8)
    )
    + .006 * noise3(vec3(p0.x * 2., T * .5, p0.z * 2.));

h = hitp(
    h,
    max(abs(p0.y + 1.228 + wave) - .006, abs(pr) - .55),
    8.
);
```

This creates a thin annular water surface.

The SDF is an intersection-like shape:

```glsl
max(
    abs(y - water_height_with_wave) - thickness,
    abs(radial_offset) - radial_width
)
```

Material `8`.

The water is thin and ring-shaped around radius `2.55`.

---

## 13. Water Basin / Pool Rim

```glsl
h = hitp(
    h,
    max(abs(p0.y + 1.10) - .12,
        abs(length(p0.xz) - 2.55) - .62),
    3.
);
```

This creates the basin or stone rim around the water.

Material `3`.

---

## 14. Radial Stone Platform Segments

```glsl
float ar = length(p0.xz);
h = hitp(
    h,
    sd_box(
        vec3(ar, p0.y + 1.21, rep(a, 2. * PI / 8.) * ar),
        vec3(.72, .035, .32)
    ),
    3.
);
```

This creates repeated stone platform slabs arranged radially.

Material `3`.

---

## 15. Central Altar Cylinder

```glsl
h = hitp(h, sd_cyl_y(vec3(p0.x, p0.y + 1.18, p0.z), .28, .07), 3.);
```

A small central cylinder, likely the altar or base under the crystal.

Material `3`.

---

## 16. Five Inner Monoliths / Crystal Posts

```glsl
if (rr > .8 && rr < 1.9) {
    float s5 = 2. * PI / 5.;
    float sid5 = floor(a / s5 + .5);
    float a5 = a - sid5 * s5;
    float m5s = sid5 * 11.7;

    h = hitp(
        h,
        sd_box(
            rot_z(
                vec3(rr - 1.25, p0.y + 1.25, a5 * rr * 2.5),
                .05 * sin(m5s)
            ),
            vec3(.06, .35 + hash(m5s) * .20, .05)
        ),
        9.
    );
}
```

Five inner objects are placed around the center.

Material `9`.

They look like magical purple posts or crystal runes.

---

## 17. Forest

```glsl
if (rr > 3. && rr < 14.5 && p0.y > -2.5 && p0.y < 3.2) {
    vec2 fh = forest_hit(p0);
    if (fh.x < h.x)
        h = fh;
}
```

The forest is only evaluated in a bounded annulus and vertical range.

This is critical for performance.

The forest itself is described in detail later.

---

## 18. Far Ring Stones

```glsl
if (rr > 9.5) {
    float s6 = 2. * PI / 6.;
    float sid6 = floor(a / s6 + .5);
    float a6 = a - sid6 * s6;
    float rs = sid6 * 17.1;
    float rh = .55 + hash(rs) * .65;

    vec3 rpp = vec3(
        rr - 11.5,
        p0.y - (
            terrain_h(p0.x * (11.5 / max(rr, .01)),
                      p0.z * (11.5 / max(rr, .01))) + rh
        ),
        a6 * rr
    );

    h = hitp(h, sd_cyl_y(rot_z(rpp, .3 * hash(rs + 1.) - .15), .12, rh), 3.);
}
```

This creates six distant standing stones or pillars around radius `11.5`.

It samples terrain height at the projected ring position so the stones sit approximately on the terrain.

Material `3`.

---

# Scene Layout Diagram

```svg
<svg width="900" height="760" viewBox="0 0 900 760" xmlns="http://www.w3.org/2000/svg">
  <style>
    .ring { fill:none; stroke:#333; stroke-width:1.5; }
    .dash { fill:none; stroke:#777; stroke-width:1; stroke-dasharray:6 4; }
    .crystal { fill:#d8b8ff; stroke:#6b2fb3; stroke-width:1.5; }
    .water { fill:#aee9ff; stroke:#187ca0; stroke-width:1.5; opacity:.75; }
    .forest { fill:#c9efc9; stroke:#267326; stroke-width:1.5; opacity:.55; }
    .stone { fill:#e8d5a5; stroke:#8a6821; stroke-width:1.5; }
    .txt { font-family:monospace; font-size:13px; fill:#111; }
  </style>

  <circle cx="450" cy="380" r="35" class="crystal"/>
  <text x="405" y="385" class="txt">crystal</text>

  <circle cx="450" cy="380" r="85" class="dash"/>
  <text x="520" y="320" class="txt">inner runes</text>

  <circle cx="450" cy="380" r="128" class="water"/>
  <circle cx="450" cy="380" r="96" fill="white" opacity=".6"/>
  <text x="545" y="390" class="txt">water ring</text>

  <circle cx="450" cy="380" r="170" class="ring"/>
  <text x="580" y="275" class="txt">8 columns</text>

  <circle cx="450" cy="380" r="245" class="ring"/>
  <text x="620" y="210" class="txt">4 gateways</text>

  <circle cx="450" cy="380" r="315" class="ring"/>
  <text x="645" y="145" class="txt">standing stones</text>

  <circle cx="450" cy="380" r="365" class="forest"/>
  <circle cx="450" cy="380" r="210" fill="white" opacity=".8"/>
  <text x="185" y="110" class="txt">forest annulus</text>

  <circle cx="450" cy="380" r="55" class="dash"/>
  <circle cx="450" cy="380" r="115" class="dash"/>
  <circle cx="450" cy="380" r="160" class="dash"/>
  <circle cx="450" cy="380" r="250" class="dash"/>

  <text x="390" y="720" class="txt">top-down conceptual layout</text>
</svg>
```

---

# The Forest Algorithm

The forest is implemented in:

```glsl
vec2 forest_hit(vec3 p0)
```

It is one of the most interesting parts of the shader because it creates many trees without storing any tree data.

---

## Forest Boundaries

```glsl
float r0 = length(p0.xz);
if (r0 < 3. || r0 > 14.5)
    return h;
```

Trees only exist in an annulus:

```text
inner radius ≈ 3
outer radius ≈ 14.5
```

This keeps the temple clearing open.

---

## Grid-Based Procedural Instancing

```glsl
float cell = 2.2;
float gx = floor(p0.x / cell);
float gz = floor(p0.z / cell);
```

World space is divided into square cells of size `2.2`.

For the current sample point, the shader only checks nearby cells:

```glsl
for (int di = -1; di <= 1; di++)
    for (int dj = -1; dj <= 1; dj++)
```

That is a `3x3` neighborhood.

This avoids checking all trees in the forest.

Each candidate cell may contain one tree.

---

## Early Distance Rejection

```glsl
vec2 dd = vec2(p0.x - cx_, p0.z - cz_);
if (dot(dd, dd) > 6.5)
    continue;
```

If the sample point is too far from the cell center, skip it.

This avoids unnecessary tree SDF evaluation.

Then radial rejection:

```glsl
float cr = length(vec2(cx_, cz_));
if (cr < 3.5 || cr > 14.2)
    continue;
```

Then occupancy rejection:

```glsl
float seed = cx_ * 13.71 + cz_ * 29.31;
float h0 = hash(seed);
if (h0 < .35)
    continue;
```

Only about 65% of candidate cells contain a tree.

---

## Jittered Tree Position

```glsl
float h1 = hash(seed + 1.);
float h2 = hash(seed + 2.);

float tx = cx_ + (h1 - .5) * cell * .75;
float tz = cz_ + (h2 - .5) * cell * .75;
```

The tree is not placed at the exact grid center.

It is randomly offset inside the cell.

This prevents a visible grid pattern.

---

## Tree Parameters

```glsl
float tree_ht = 1.6 + h0;
float tree_r = .44 + h1 * .28;
```

Each tree gets:

* height: `1.6` to `2.6`
* foliage radius: `.44` to `.72`

The tree base height is approximated by a cheaper terrain expression:

```glsl
float by_ = -1.25
          + .40 * sin(tx * .21 + tz * .18)
          + .22 * sin(tx * .67 - tz * .43);
```

Notice this does not call full `terrain_h()`. It uses the first two terrain waves only.

This is a performance/code-size tradeoff.

---

## Local Tree Coordinates

```glsl
vec3 tp = p0 - vec3(tx, by_, tz);
```

Now `tp` is the sample point relative to the tree base.

---

## Tree Bounding Tests

```glsl
if (length(tp.xz) > tree_r + .6)
    continue;

if (tp.y < -.3 || tp.y > tree_ht + .4)
    continue;
```

These skip points outside the plausible tree volume.

This is important because `forest_hit()` is called inside `map_()`, which is called many times per ray.

---

## Trunk

```glsl
h = hitp(
    h,
    sd_cyl_y(tp - vec3(0, tree_ht * .3, 0), .05, tree_ht * .3),
    6.
);
```

The trunk is a simple vertical cylinder.

It is centered at `tree_ht * .3` and has half-height `tree_ht * .3`, so it extends approximately from:

```text
y = 0
to
y = tree_ht * .6
```

Material `6`.

---

## Root / Trunk Base Blob

```glsl
h = hitp(h, length(tp) - tree_r * .30, 6.);
```

This adds a small spherical/rooty base around the tree origin.

It helps visually anchor the tree to the ground.

Material `6`.

---

## Foliage Layers

```glsl
float wind = .055 * sin(T * 1.15 + tx * .7 + tz * .5);

for (int i = 0; i < 5; i++) {
    float fi = float(i) * .20;
    float fy = tree_ht * (.38 + fi);
    float fr = tree_r * (1.05 - fi * 1.1);

    if (fr <= 0.)
        break;

    float sw = wind * fy * .25;
    vec3 fq = tp - vec3(sw, fy, sw * .5);

    h = hitp(
        h,
        length(vec3(fq.x, fq.y * .8, fq.z)) - fr,
        7.
    );
}
```

The foliage is a stack of five vertically arranged ellipsoid-like blobs.

The distance expression:

```glsl
length(vec3(fq.x, fq.y * .8, fq.z)) - fr
```

scales Y before measuring length. Because `fq.y` is multiplied by `0.8` (< 1), the blob is slightly taller than wide — a compact prolate shape that suits conifers. Crucially, a scale factor less than 1 keeps the SDF gradient magnitude below 1 in the y-direction, preserving the conservative distance bound required for correct sphere tracing.

The radius `fr` decreases for higher layers:

```glsl
fr = tree_r * (1.05 - fi * 1.1)
```

So the canopy tapers upward like a conifer.

Wind offsets the foliage:

```glsl
vec3 fq = tp - vec3(sw, fy, sw * .5);
```

Higher layers sway more because `sw` depends on `fy`.

Material `7`.

---

# Forest Diagram

```svg
<svg width="900" height="520" viewBox="0 0 900 520" xmlns="http://www.w3.org/2000/svg">
  <style>
    .grid { stroke:#ccc; stroke-width:1; }
    .cell { fill:#f8fff8; stroke:#aaa; stroke-width:1; }
    .tree { fill:#4c9a4c; stroke:#174d17; stroke-width:1.2; }
    .trunk { fill:#8a5a2b; stroke:#4a2b10; stroke-width:1; }
    .skip { fill:#eee; stroke:#999; stroke-width:1; opacity:.5; }
    .txt { font-family:monospace; font-size:13px; fill:#111; }
    .small { font-family:monospace; font-size:11px; fill:#333; }
  </style>

  <text x="50" y="40" class="txt">forest_hit(): check only 3x3 neighboring cells</text>

  <g transform="translate(60,80)">
    <rect x="0" y="0" width="90" height="90" class="cell"/>
    <rect x="90" y="0" width="90" height="90" class="cell"/>
    <rect x="180" y="0" width="90" height="90" class="cell"/>
    <rect x="0" y="90" width="90" height="90" class="cell"/>
    <rect x="90" y="90" width="90" height="90" fill="#fff2cc" stroke="#aa8" stroke-width="2"/>
    <rect x="180" y="90" width="90" height="90" class="cell"/>
    <rect x="0" y="180" width="90" height="90" class="cell"/>
    <rect x="90" y="180" width="90" height="90" class="cell"/>
    <rect x="180" y="180" width="90" height="90" class="cell"/>

    <circle cx="42" cy="55" r="12" class="tree"/>
    <circle cx="150" cy="35" r="12" class="skip"/>
    <circle cx="230" cy="72" r="12" class="tree"/>
    <circle cx="61" cy="145" r="12" class="tree"/>
    <circle cx="137" cy="132" r="5" fill="#d22"/>
    <circle cx="216" cy="145" r="12" class="tree"/>
    <circle cx="45" cy="220" r="12" class="skip"/>
    <circle cx="120" cy="246" r="12" class="tree"/>
    <circle cx="240" cy="225" r="12" class="tree"/>

    <text x="105" y="124" class="small">sample p</text>
    <text x="15" y="295" class="small">hash decides occupancy + jitter</text>
  </g>

  <g transform="translate(520,80)">
    <rect x="100" y="260" width="18" height="90" class="trunk"/>
    <ellipse cx="109" cy="250" rx="90" ry="34" class="tree"/>
    <ellipse cx="109" cy="205" rx="75" ry="30" class="tree"/>
    <ellipse cx="109" cy="165" rx="58" ry="26" class="tree"/>
    <ellipse cx="109" cy="130" rx="40" ry="22" class="tree"/>
    <ellipse cx="109" cy="100" rx="22" ry="18" class="tree"/>
    <text x="0" y="380" class="txt">one tree = trunk + stacked foliage SDF blobs</text>
    <text x="0" y="405" class="small">height, radius, sway, presence from hash(seed)</text>
  </g>
</svg>
```

---

# Normal Estimation

Normals are computed in:

```glsl
vec3 norm_(vec3 p, float m, Ctx cx)
```

Normals are required for:

* diffuse lighting
* specular highlights
* Fresnel rim lighting
* sky fill
* ambient occlusion direction
* water reflection

---

## Special Terrain Normal

For terrain material `4`, the shader uses the heightfield directly:

```glsl
if (m == 4.) {
    float e = .04;
    return normalize(vec3(
        terrain_h(p.x - e, p.z) - terrain_h(p.x + e, p.z),
        2. * e,
        terrain_h(p.x, p.z - e) - terrain_h(p.x, p.z + e)
    ));
}
```

This is more stable and cheaper than sampling the full scene SDF.

It estimates slope from neighboring terrain heights.

---

## General SDF Normal

For other materials:

```glsl
float e = .004;
vec3 k1 = vec3(1, -1, -1);
vec3 k2 = vec3(-1, -1, 1);
vec3 k3 = vec3(-1, 1, -1);
vec3 k4 = vec3(1, 1, 1);

return normalize(
    k1 * map_(p + k1 * e, cx).x +
    k2 * map_(p + k2 * e, cx).x +
    k3 * map_(p + k3 * e, cx).x +
    k4 * map_(p + k4 * e, cx).x
);
```

This is a tetrahedral normal approximation.

Instead of six samples along ±X, ±Y, ±Z, it uses four samples in tetrahedral directions.

That is cheaper than central differences and common in raymarching.

---

# Shadows

Soft shadows are computed in:

```glsl
float shad_(vec3 ro, vec3 rd, Ctx cx)
```

Simplified:

```glsl
float res = 1.;
float d = .035;

for (int i = 0; i < 36; i++) {
    float h = map_(ro + rd * d, cx).x;

    if (h < .002)
        return 0.;

    res = min(res, 9. * h / d);
    d += clamp(h, .025, .28);

    if (d > 8.)
        break;
}

return clamp(res, 0., 1.);
```

This raymarches from a surface point toward a light.

If the ray gets very close to geometry, the point is shadowed.

The expression:

```glsl
res = min(res, 9. * h / d);
```

creates a soft shadow approximation.

If the ray passes near geometry but does not hit it, `h / d` gets small and the light is partially attenuated.

---

# Ambient Occlusion

Ambient occlusion is computed in:

```glsl
float ao_(vec3 p, vec3 n, float m, Ctx cx)
```

It samples along the surface normal and checks whether geometry is nearby.

General case:

```glsl
float o = 0., sc = 1.;

for (int i = 1; i <= 9; i++) {
    float h = float(i) * .048;
    o += max(h - map_(p + n * h, cx).x, 0.) * sc;
    sc *= .57;
}

return clamp(1. - o * 1.5, 0., 1.);
```

If the point is open to space, then at distance `h` along the normal, the SDF should also be about `h`.

If the SDF is smaller than `h`, some geometry is nearby and the surface is occluded.

Terrain has a special cheaper AO path:

```glsl
if (m == 4.) {
    ...
    o += max(h - (sp.y - terrain_h(sp.x, sp.z)), 0.) * sc;
    ...
}
```

This avoids full `map_()` calls for terrain AO.

---

# Materials and Palette

The palette function:

```glsl
vec3 pal_(float m, vec3 p)
```

maps material IDs to base colors.

The material colors are procedural, not constant. Most use position and noise to avoid flat surfaces.

---

## Material 1: Main Crystal

```glsl
if (m == 1.) {
    float h = p.y * 6. + p.x * 3. + T;
    float h2 = p.z * 5. - p.y * 4. + T * 1.3;

    return vec3(
        .85 + .15 * abs(sin(h)),
        .30 + .30 * abs(sin(h * 1.5 + 1.2)),
        .88 + .12 * abs(sin(h2))
    );
}
```

Animated pink/purple crystalline color.

---

## Material 2: Energy Rings

```glsl
if (m == 2.) {
    float s = .8 + .2 * abs(sin(p.x * 11. + p.y * 13.));
    return vec3(.22 * s, .90 * s, s);
}
```

Cyan energy with small position variation.

---

## Material 3: Stone / Gold Architecture

```glsl
if (m == 3.) {
    float c = .80 + .20 * noise3(p * 2.5);
    float f = .90 + .10 * noise3(p * 6.);
    float v = smoothstep(.55, .45, abs(noise3(vec3(p.x * 4. + p.y * 3., p.y * 5., p.z * 4.))));
    float tx = c * f * (1. - v * .35);
    return vec3(.98 * tx, .70 * tx, .30 * tx + v * .05);
}
```

Warm stone/gold with vein-like variation.

The variable `v` acts like a vein mask.

---

## Material 4: Ground

```glsl
if (m == 4.) {
    float r = length(p.xz);
    float fb = smoothstep(3.5, 6.5, r);
    ...
    return mix(...);
}
```

The ground changes with radius.

Near the temple it is darker bluish stone.

Farther out it blends toward green forest ground.

---

## Material 5: Cyan Inlays

```glsl
if (m == 5.) {
    float gw = .7 + .3 * abs(sin(p.x * 8. + p.z * 6. + T * 1.2));
    return vec3(.10 * gw, .78 * gw, .90 * gw);
}
```

Glowing cyan with animation.

---

## Material 6: Wood

```glsl
if (m == 6.) {
    float g = .75 + .25 * noise3(vec3(p.y * 4., p.x * 3., p.z * 3.));
    return vec3(.30 * g, .17 * g, .09 * g);
}
```

Brown trunk material with procedural variation.

---

## Material 7: Leaves

```glsl
if (m == 7.) {
    float v = .70 + .30 * noise3(p * 5.);
    return vec3(.07 * v, .24 * v + .05 * abs(sin(p.y * 4. + p.x)), .09 * v);
}
```

Dark green foliage.

---

## Material 8: Water

```glsl
if (m == 8.)
    return vec3(.05, .14, .42);
```

Dark blue base. The reflective behavior is handled later in `shade_()`.

---

## Material 9: Purple Magic Objects

```glsl
if (m == 9.) {
    float s = .65 + .35 * abs(sin(p.y * 9. + T * 1.5 + p.x * 5.));
    return vec3(.70 * s + .25, .35 * s, .95 * s);
}
```

Bright violet/purple magical material.

---

# Sky, Stars, Moon, Mountains, Clouds, and Aurora

The sky function is:

```glsl
vec3 sky_(vec3 rd)
```

It takes only the ray direction.

There is no sky geometry. The sky is a procedural environment function.

---

## Base Gradient

```glsl
float y = clamp(rd.y * .5 + .5, 0., 1.);
vec3 c = vec3(.018, .014, .055) * (1. - y)
       + vec3(.014, .075, .175) * y;
```

This blends from dark purple near the horizon/downward directions to blue upward.

---

## Stars

First star layer:

```glsl
float u = atan(rd.z, rd.x) * 7.;
float v = (1. - clamp(rd.y, -1., 1.)) * (9. * PI * .5);
float id = abs(floor(u) * 37. + floor(v) * 113.);

c += vec3(.80, .92, 1.)
   * smoothstep(.995, 1., hash(id + floor(T * .03)))
   * (.5 + .5 * abs(sin(T * 3. + id)));
```

The sky direction is quantized into angular cells.

Each cell gets a hash.

Only cells with hash near 1 become stars.

The star brightness flickers with time.

Second star/cluster layer:

```glsl
float u2 = atan(rd.z, rd.x) * 19.;
float v2 = ...;
float cid = abs(floor(u2) * 53. + floor(v2) * 137.);
float cm = smoothstep(.82, .97, clamp(dot(rd, normalize(vec3(.6, .7, -.38))), 0., 1.));

c += vec3(.90, .85, 1.)
   * smoothstep(.984, .998, hash(cid + 17.3))
   * cm
   * (.4 + .6 * hash(cid));
```

This adds a more localized star cluster.

---

## Milky Way / Sky Bands

```glsl
float ma = atan(rd.z, rd.x);
c += vec3(.08, .10, .18)
   * (
       smoothstep(.18, .04, abs(ma * .6 + rd.y * .8) - .04)
       + smoothstep(.16, .02, abs(ma * .6 + rd.y * .8 + .3) - .02) * .5
     )
   * smoothstep(0., .25, rd.y);
```

This creates diagonal soft sky bands.

---

## Moon

```glsl
vec3 md = normalize(vec3(-.5, .65, -.6));
float mdd = clamp(dot(rd, md), 0., 1.);

c += vec3(.92, .94, 1.) * smoothstep(.9998, 1., mdd)
   + vec3(.12, .22, .40) * pow(smoothstep(.93, 1., mdd), 2.) * .50
   + vec3(.04, .08, .18) * smoothstep(.70, 1., mdd) * .15;
```

The moon is a very tight angular highlight in direction `md`.

It has:

* bright core
* bluish halo
* broader weak glow

---

## Mountain Silhouette

```glsl
float ang = atan(rd.z, rd.x);

float ridge = .065
            + .040 * sin(ang * 3.)
            + .022 * sin(ang * 9. + T * .04)
            + .012 * sin(ang * 23.)
            + .006 * sin(ang * 51. + T * .10);

float mtn = smoothstep(ridge + .018, ridge - .010, rd.y);

c = c * (1. - mtn * .87) + vec3(.010, .018, .038) * mtn;
```

This creates a mountain silhouette in the sky based on ray direction.

The ridge line is a sum of sine waves over angle.

The mountain darkens the sky where `rd.y` is below the ridge.

---

## Horizon Glow

```glsl
c += vec3(.25, .30, .55)
   * smoothstep(.15, -.05, rd.y)
   * pow(clamp(dot(rd, normalize(vec3(md.x, 0, md.z))), 0., 1.), 2.)
   * .18;
```

Adds bluish glow near the horizon in the moon's horizontal direction.

---

## Clouds

```glsl
float ch = smoothstep(.08, .26, rd.y) * (1. - smoothstep(.28, .52, rd.y));

c += vec3(.07, .09, .16)
   * pow(
        (.5 + .5 * sin(ang * 7. + T * .06))
      * (.5 + .5 * sin(ang * 13. - rd.y * 9. + T * .04)),
        2.
     )
   * ch;
```

Clouds are angular sine bands constrained to a vertical region.

Not physically volumetric, but visually cheap.

---

## Aurora

The aurora is several animated colored bands:

```glsl
float hm = smoothstep(.04, .42, rd.y)
         * (1. - smoothstep(.48, .88, rd.y));
```

This limits aurora to a vertical range above the horizon.

Then four color layers are added:

```glsl
green
purple
blue
pink
```

Each uses:

* angle-dependent sine
* time animation
* high-frequency wave modulation
* vertical mask

Example green layer:

```glsl
c += vec3(.04, .65, .38)
   * (.5 + .5 * sin(ang * 3.2 + T * .17))
   * hm
   * pow(.5 + .5 * sin(ang * 17. + rd.y * 23. + T * 1.20), 2.)
   * .28;
```

This creates shimmering curtains without any textures.

---

# Sky Composition Diagram

```svg
<svg width="900" height="520" viewBox="0 0 900 520" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="skygrad" x1="0" x2="0" y1="1" y2="0">
      <stop offset="0%" stop-color="#08051f"/>
      <stop offset="55%" stop-color="#07184a"/>
      <stop offset="100%" stop-color="#0b346a"/>
    </linearGradient>
  </defs>
  <style>
    .txt { font-family:monospace; font-size:13px; fill:#fff; }
    .darktxt { font-family:monospace; font-size:13px; fill:#111; }
    .mount { fill:#050914; }
    .aurora1 { fill:none; stroke:#3cff9a; stroke-width:18; opacity:.45; }
    .aurora2 { fill:none; stroke:#a64cff; stroke-width:12; opacity:.35; }
    .cloud { fill:none; stroke:#aab3d8; stroke-width:16; opacity:.22; }
    .moon { fill:#f2f5ff; stroke:#dfe7ff; stroke-width:8; }
  </style>

  <rect x="0" y="0" width="900" height="520" fill="url(#skygrad)"/>
  <path class="aurora1" d="M80,190 C190,120 310,250 430,170 C540,100 650,220 820,130"/>
  <path class="aurora2" d="M40,240 C180,170 310,270 460,210 C600,160 700,280 860,200"/>
  <path class="cloud" d="M100,300 C230,270 310,330 440,295 C560,260 690,335 810,300"/>
  <circle cx="680" cy="105" r="24" class="moon"/>

  <circle cx="160" cy="95" r="2" fill="#fff"/>
  <circle cx="220" cy="130" r="1.5" fill="#e8f2ff"/>
  <circle cx="310" cy="70" r="2" fill="#fff"/>
  <circle cx="510" cy="130" r="1.5" fill="#fff"/>
  <circle cx="760" cy="180" r="2" fill="#dcecff"/>

  <path class="mount" d="M0,390 L70,340 L130,370 L210,320 L300,380 L400,335 L500,365 L600,310 L700,370 L790,330 L900,380 L900,520 L0,520z"/>

  <text x="45" y="45" class="txt">procedural stars</text>
  <text x="475" y="100" class="txt">moon + halo</text>
  <text x="120" y="175" class="txt">aurora sine bands</text>
  <text x="110" y="305" class="txt">cloud band</text>
  <text x="350" y="430" class="txt">mountain silhouette from angular ridge</text>
</svg>
```

---

# Camera Choreography

The camera function is:

```glsl
void cam_(out vec3 ro, out vec3 f, out vec3 r, out vec3 u)
```

It outputs:

* `ro`: ray origin / camera position
* `f`: forward vector
* `r`: right vector
* `u`: up vector

The animation is divided into 18-second segments:

```glsl
int seg = int(T * (1. / 18.));
int act = seg % 5;
float loc = T - float(seg) * 18.;
float k = smoothstep(0., 5., loc);
```

There are five camera acts.

Each act changes:

* distance from center
* height
* orbit speed

---

## Act 0: Approach

```glsl
dist = 7. - k * 3.;
ht = -.2 + k * 1.7;
spd = .07;
```

Starts far and low, then moves closer and higher.

---

## Act 1: Medium Orbit

```glsl
dist = 4.2;
ht = 1.2 + .3 * sin(T * .35);
spd = .14;
```

Stable orbit around the temple.

---

## Act 2: Close Dynamic Shot

```glsl
dist = 2.3 + .3 * sin(T * .8);
ht = .2 + .5 * sin(T * .6);
spd = .40;
```

Closer and faster.

This emphasizes the crystal and rings.

---

## Act 3: Pullback

```glsl
dist = 4. + k * 4.5;
ht = 1.5 + k * 2.4;
spd = .09;
```

Moves back and up, showing more of the forest and sky.

---

## Act 4: High Wide Shot

```glsl
dist = 6.5;
ht = 3. + k * 1.8;
spd = .06;
```

High, slow, wide view.

---

## Camera Basis

```glsl
float tt = T * spd;
ro = vec3(dist * cos(tt), ht, dist * sin(tt));

float ly = (act == 4) ? -.28 + .12 * sin(T * .25)
                      : -.08 + .18 * sin(T * .28);

f = normalize(vec3(0, ly, 0) - ro);
r = normalize(cross(f, vec3(0, 1, 0)));
u = cross(r, f);
```

The camera always looks near the origin, with a slightly animated target height.

The basis is:

```text
f = forward
r = right
u = up
```

Then `main()` constructs rays:

```glsl
vec3 rd = normalize(f * 1.35 + r * px + u * py);
```

The value `1.35` is the focal length. Larger means narrower FOV.

---

# Lighting Model

Lighting is computed after raymarching hits a surface.

```glsl
vec3 p = ro + rd * depth;
vec3 n = norm_(p, mat, cx);
vec3 base = pal_(mat, p);
```

Then the material may emit light:

```glsl
vec3 emit = (mat == 1.)   ? base * .20
          : (mat == 2.)   ? base * .32
          : (mat == 5.)   ? base * .35
          : (mat == 9.)   ? base * .25
                          : vec3(0.);
```

Emissive materials:

* crystal
* energy rings
* floor inlays
* purple magical objects

---

## Specular Strength by Material

```glsl
float ss = (mat == 1. || mat == 9.) ? 2.2
         : (mat == 2.)              ? 1.8
         : (mat == 8.)              ? 3.5
         : (mat == 6.)              ? .18
         : (mat == 7.)              ? .07
         : (mat == 5.)              ? .8
                                    : 1.;
```

Water has the highest specular strength.

Leaves and wood have low specular.

Crystal and magic objects are shiny.

---

## Bump-Like Normal Perturbation

For terrain:

```glsl
if (mat == 4.) {
    float e = .08;
    float bx = noise3(vec3(p.x + e, p.y, p.z)) - noise3(vec3(p.x - e, p.y, p.z));
    float bz = noise3(vec3(p.x, p.y, p.z + e)) - noise3(vec3(p.x, p.y, p.z - e));
    bn = normalize(n + vec3(bx, 0, bz) * .35);
}
```

For wood:

```glsl
else if (mat == 6.) {
    float e = .06;
    float bx = noise3(vec3(p.y * 2. + e, p.z, p.x)) - noise3(vec3(p.y * 2. - e, p.z, p.x));
    float by_ = noise3(vec3(p.x, p.y * 2. + e, p.z)) - noise3(vec3(p.x, p.y * 2. - e, p.z));
    bn = normalize(n + vec3(bx, by_, 0) * .30);
}
```

This creates cheap material detail without changing the actual geometry.

---

## Lights

Two point lights are used:

```glsl
vec3 lp1 = vec3(2.6 * cx.r0_cx, 2.1, 2.6 * cx.r0_sx);
vec3 lp2 = vec3(-2., cx.lp2_y, -2.5);
```

`lp1` orbits the scene.

`lp2` bobs vertically.

For each light:

```glsl
vec3 l = normalize(lps[li] - p);
float dif = max(dot(n, l), 0.) * shad_(p + n * .01, l, cx);

vec3 rv = normalize(n * (2. * dot(n, l)) - l);
float spec = pow(max(dot(rv, -rd), 0.), 38.);

col += base * dif
     + vec3(1., .95, .8) * spec * ss * (.55 + .45 * dif);
```

Lighting includes:

* diffuse Lambert term
* soft shadow multiplier
* Blinn/Phong-like specular reflection
* material-specific specular strength

---

## Sky Fill

```glsl
vec3 sky_fill = n.y > 0. ? sky_(n) * (0.18 * n.y) : vec3(0.);
vec3 col = base * .06 + emit + base * max(n.y, 0.) * .05 + sky_fill * base;
```

Upward-facing surfaces receive some sky color.

This gives a more natural outdoor look.

---

# Fresnel and Rim Lighting

```glsl
float fr = clamp(1. + dot(n, rd), 0., 1.);
float fres = fr * fr * fr;
```

When the view angle grazes a surface, `fres` becomes stronger.

Then:

```glsl
col = col * ao_v + vec3(.25, .85, 1.) * fres;
```

A cyan base rim is added.

Then a material-dependent rim is added:

```glsl
vec3 rim = (mat == 1. || mat == 9.) ? vec3(.50, .20, 1.) * 1.8
         : (mat == 2.)              ? vec3(.10, .80, 1.) * 1.5
         : (mat == 7.)              ? vec3(.05, .50, .15) * 1.2
         : (mat == 8.)              ? vec3(.10, .40, .90) * 2.
                                    : vec3(.22, .75, 1.);

col += rim * fres * ao_v;
```

This is a stylized fantasy lighting trick. It makes silhouettes glow.

---

# Leaf Translucency

For foliage:

```glsl
if (mat == 7.) {
    float bl = max(-dot(n, normalize(lp1 - p)), 0.);
    col += vec3(.08, .35, .10) * bl * bl * .55 * smoothstep(8., 3., depth);
}
```

This adds backlighting when leaves face away from the light.

It makes tree canopies feel more organic.

---

# Water and Reflection

Water material is `8`.

In `shade_()`:

```glsl
if (mat == 8.) {
    float wf = min(fres + .4, 1.);
    col = col * (1. - wf) + sky_(rd - n * (2. * dot(rd, n))) * wf;
    col *= (.3 + .7 * abs(sin(p.x * 8. + p.z * 7. + T * 2.5)));
}
```

The reflection direction is:

```glsl
rd - n * (2. * dot(rd, n))
```

That is the vector reflection formula.

The water color is blended with reflected sky based on Fresnel:

```glsl
wf = min(fres + .4, 1.)
```

The sine multiplier adds animated surface shimmer.

This is not full scene reflection. It reflects only the procedural sky, which is cheap and effective.

---

# Volumetric Glow and Fireflies

During raymarching, before any hit is found, the shader accumulates glow:

```glsl
glow += vec3(.7, .2, 1.) * .0015 / (.012 + abs(h.x))
      + vec3(.1, .8, 1.) * .0008 / (.020 + abs(h.x + .05));
```

This makes rays glow when they pass near surfaces.

It is a fake volumetric effect.

The closer the ray is to geometry, the more glow accumulates.

---

## Central Beam Glow

```glsl
float xz2 = dot(p.xz, p.xz);

glow += vec3(.25, .65, 1.) * .003 / (.018 + xz2)
      * clamp(p.y + 1.2, 0., 1.)
      * clamp(2.8 - p.y, 0., 1.);

glow += vec3(.60, .65, .90) * .0005 / (.06 + xz2 * .15)
      * clamp(p.y + .5, 0., 1.)
      * clamp(4. - p.y, 0., 1.) * .3;
```

This creates a vertical energy glow near the center.

Because `xz2` is small near the Y axis, the glow is strongest near the center line.

The vertical `clamp` terms restrict it to a height range.

---

## Fireflies

```glsl
vec3 fp = p + vec3(.5 * sin(T * .17), T * .20, .4 * cos(T * .14));

float cell = 2.;
float ix = floor(fp.x / cell);
float iy = floor(fp.y / cell);
float iz = floor(fp.z / cell);

float seed = ix * 17. + iy * 57. + iz * 113.;

vec3 pp = vec3(
    ix * cell + hash(seed + 1.) * cell,
    iy * cell + hash(seed + 2.) * cell,
    iz * cell + hash(seed + 3.) * cell
);

float fd = length(fp - pp);

glow += vec3(.40, .95, .55)
      * .0008 / (.018 + fd * fd)
      * smoothstep(9., 2.5, length(p.xz));
```

The shader creates one potential glowing point per 3D grid cell.

The grid scrolls upward over time because:

```glsl
fp.y += T * .20
```

The fireflies are strongest near the center because of:

```glsl
smoothstep(9., 2.5, length(p.xz))
```

This fades them out toward the outer forest.

---

# Mist and Fog

Final fog:

```glsl
float fog = smoothstep(3., 16., depth);
float mist = smoothstep(.5, -.9, p.y) * .42;

return col * (1. - fog) * (1. - mist)
     + sky_col * fog * (1. - mist)
     + vec3(.038, .065, .155) * mist
     + glow * .6;
```

There are two atmospheric effects:

## Distance fog

```glsl
fog = smoothstep(3., 16., depth)
```

Far surfaces blend into the sky.

## Low mist

```glsl
mist = smoothstep(.5, -.9, p.y) * .42
```

Low areas near the ground get blue mist.

Together they:

* hide raymarching distance limits
* add depth
* integrate forest and mountains
* improve mood

---

# Final Color Grading

At the end of `main()`:

```glsl
vec3 raw = shade_(ro, rd, cx) * vign;
```

The scene color is multiplied by a vignette.

---

## Vignette

```glsl
float vign = clamp(
    1. - .38 * min(px * px + py * py, 1.)
       - .12 * max(max(abs(px), abs(py)) - .75, 0.),
    0.,
    1.
);
```

This darkens:

* radial corners
* extreme frame edges

It focuses the eye on the center.

---

## Progress-Based Warmth

```glsl
float prog = clamp(T / 90., 0., 1.);
vec3 warm = vec3(1. + prog * .07, 1., 1. - prog * .06);
```

Over the 90-second runtime, the image becomes slightly warmer:

* red increases
* blue decreases

This gives subtle temporal color progression.

---

## Lift

```glsl
vec3 lift = vec3(.012, .012, .022);
```

Adds a small dark-level lift before tone mapping.

This keeps shadows from crushing completely black.

---

## Contrast

```glsl
float lum = dot(raw, vec3(.299, .587, .114));
float contrast = 1. + .12 * (lum - .45);
```

A simple luminance-dependent contrast adjustment.

Bright pixels get slightly more contrast.

Dark pixels get slightly less.

---

## Tone Mapping and Gamma

```glsl
vec3 gc = (raw + lift) * warm * max(contrast, .3);

O = vec4(
    sqrt(clamp(gc / (1. + gc), vec3(0.), vec3(1.))),
    1.
);
```

The expression:

```glsl
gc / (1. + gc)
```

is Reinhard-like tone mapping.

It compresses bright values smoothly.

The final `sqrt()` approximates gamma correction.

`sqrt(x)` is roughly `pow(x, 0.5)`, close to display gamma correction.

---

# Full Per-Pixel Diagram

```svg
<svg width="900" height="900" viewBox="0 0 900 900" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <marker id="arr4" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto">
      <path d="M0,0 L0,6 L9,3 z" fill="#222"/>
    </marker>
    <style>
      .box { fill:#f9f9ff; stroke:#222; stroke-width:1.4; rx:10; }
      .scene { fill:#fff6e8; stroke:#916412; stroke-width:1.4; rx:10; }
      .light { fill:#efffed; stroke:#227a2d; stroke-width:1.4; rx:10; }
      .post { fill:#f0f6ff; stroke:#275c9a; stroke-width:1.4; rx:10; }
      .txt { font-family:monospace; font-size:13px; fill:#111; }
      .small { font-family:monospace; font-size:11px; fill:#333; }
      .arrow { stroke:#222; stroke-width:1.4; marker-end:url(#arr4); fill:none; }
    </style>
  </defs>

  <rect x="330" y="20" width="240" height="60" class="box"/>
  <text x="355" y="55" class="txt">pixel gl_FragCoord</text>

  <rect x="330" y="120" width="240" height="70" class="box"/>
  <text x="355" y="150" class="txt">screen coords px, py</text>
  <text x="355" y="170" class="small">aspect corrected</text>

  <rect x="330" y="230" width="240" height="80" class="box"/>
  <text x="355" y="260" class="txt">camera ray</text>
  <text x="355" y="282" class="small">rd = normalize(f*1.35+r*px+u*py)</text>

  <rect x="330" y="360" width="240" height="80" class="scene"/>
  <text x="355" y="390" class="txt">raymarch shade_()</text>
  <text x="355" y="412" class="small">loop up to 80 steps</text>

  <rect x="60" y="500" width="220" height="80" class="scene"/>
  <text x="85" y="530" class="txt">map_()</text>
  <text x="85" y="552" class="small">nearest distance + material</text>

  <rect x="340" y="500" width="220" height="80" class="light"/>
  <text x="365" y="530" class="txt">surface shading</text>
  <text x="365" y="552" class="small">normal, AO, shadow, lights</text>

  <rect x="620" y="500" width="220" height="80" class="light"/>
  <text x="645" y="530" class="txt">environment</text>
  <text x="645" y="552" class="small">sky, fog, mist, glow</text>

  <rect x="330" y="650" width="240" height="80" class="post"/>
  <text x="355" y="680" class="txt">post color</text>
  <text x="355" y="702" class="small">vignette, warm, contrast</text>

  <rect x="330" y="780" width="240" height="60" class="post"/>
  <text x="355" y="815" class="txt">O = vec4(color, 1)</text>

  <path d="M450,80 L450,120" class="arrow"/>
  <path d="M450,190 L450,230" class="arrow"/>
  <path d="M450,310 L450,360" class="arrow"/>
  <path d="M390,440 L190,500" class="arrow"/>
  <path d="M450,440 L450,500" class="arrow"/>
  <path d="M510,440 L730,500" class="arrow"/>
  <path d="M450,580 L450,650" class="arrow"/>
  <path d="M450,730 L450,780" class="arrow"/>
</svg>
```

---

# Performance Characteristics

The cost is dominated by calls to `map_()`.

Each pixel may do:

* up to 80 raymarch steps
* each raymarch step calls `map_()`
* hit shading calls `norm_()`
* `norm_()` calls `map_()` four more times
* each light calls `shad_()`
* `shad_()` calls `map_()` up to 36 times
* `ao_()` calls `map_()` up to 9 times

Worst-case hit pixel estimate:

```text
main raymarch:       up to 80 map_ calls
normal:              4 map_ calls
two shadows:         2 * 36 = 72 map_ calls
AO:                  9 map_ calls
--------------------------------------
total:               up to 165 map_ calls per pixel
```

At 640x360:

```text
230,400 pixels/frame
```

Worst-case theoretical upper bound:

```text
230,400 * 165 ≈ 38 million map_ calls/frame
```

Actual cost is lower because:

* many rays miss before expensive shading
* shadow rays break early
* forest is bounded
* central objects are bounded by radius tests
* many objects have early radial conditions
* far terrain/sky returns before surface shading

Still, this is a heavy shader. The code uses many cheap approximations to keep it viable.

---

# Important Optimization Patterns

## 1. Radial Bounding

Many objects are only evaluated in certain radius ranges:

```glsl
if (rr < 2.5) { central crystal and rings }
if (rr > 1.8 && rr < 3.2) { columns }
if (rr > 3. && rr < 14.5) { forest }
```

This prevents evaluating all objects everywhere.

---

## 2. Polar Repetition

Objects are repeated angularly without arrays:

```glsl
rep(a, 2. * PI / 8.)
```

This creates:

* 8 columns
* 16 inlays
* 12 shards
* 4 gateways
* 7 stones
* 5 inner posts
* 6 far stones

The executable stores one formula, not many objects.

---

## 3. Hash-Based Variation

Instead of storing random parameters, the shader derives them:

```glsl
hash(seed)
hash(seed + 1.)
hash(seed + 2.)
```

Used for:

* object placement
* height
* rotation
* tree size
* star placement

---

## 4. Approximate Noise

The shader avoids full noise implementations.

Instead it uses:

```glsl
sin(...) + sin(...) * .5 + sin(...) * .25
```

This is small and fast enough.

---

## 5. Special-Case Terrain Normal and AO

Terrain has cheaper normal and AO paths.

That avoids full scene SDF sampling for the most common surface.

---

## 6. Pbuffer Rendering

The Rust host does not create a window.

It renders offscreen and streams raw RGB.

This makes the program suitable for:

* live preview via ffplay
* deterministic recording via ffmpeg
* headless rendering
* simple capture pipelines

---

# What the Demo Is Doing Artistically

The scene is a layered procedural composition:

```text
center:
    glowing crystal altar

near ring:
    water pool, inlays, runes, columns

middle ring:
    gateways and standing stones

outer ring:
    forest and far stones

background:
    mountains, moon, stars, clouds, aurora

atmosphere:
    mist, glow, fog, fireflies
```

The shader uses several tricks to make this feel larger than it is:

* repeated geometry
* animated rings
* emissive materials
* sky complexity
* distance fog
* forest density
* moving camera acts
* procedural variation everywhere

The result is not a physically accurate renderer. It is a compact procedural fantasy scene optimized for visual richness per line of code and per byte.

---

# Subsystem Summary

## Rust host

```text
Creates EGL pbuffer
Creates OpenGL 3.3 context
Compiles shaders
Draws fullscreen triangle
Reads RGB pixels
Flips rows
Writes raw RGB24 frames to stdout
```

## GLSL shader

```text
Builds animated camera ray
Raymarches signed-distance scene
Generates terrain, temple, forest, water, rings, crystals
Computes normals, shadows, AO
Applies material palette
Adds sky, stars, moon, mountains, aurora
Adds glow, mist, fog, fireflies
Tone maps and outputs final color
```

---

# Final Mental Model

The entire demo is built around one question:

```glsl
vec2 map_(vec3 p, Ctx cx)
```

For any 3D point `p`, the shader asks:

```text
What is the distance to the closest surface?
What material is that surface?
```

Everything else follows from that:

```text
raymarching:
    use distance to find surfaces

normals:
    sample distance nearby

shadows:
    march distance toward lights

ambient occlusion:
    sample distance along normal

materials:
    use material ID

fog:
    use ray depth

glow:
    accumulate near surfaces

water:
    reflect sky using normal

camera:
    choose ray origin/direction over time
```

The program is therefore not a mesh renderer.

It is a mathematical world evaluator.

The geometry is not loaded.

The geometry is not stored.

The geometry is computed on demand, for every pixel, every frame.

```
```
