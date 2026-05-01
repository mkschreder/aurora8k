# aurora8k

A CPU-only SDF ray-marcher that renders a procedural 3D scene into a
truecolor terminal. No GPU, no assets, no crates, no libc, no libm.
Pure math + raw Linux syscalls. Written in `no_std` Rust.

## What it renders

An animated scene with:
- a central gem (octahedron + sphere CSG with surface displacement)
- three orbital torus rings
- eight temple columns via polar repetition
- an animated reflective floor with a tiled inlay
- procedural star field sky

Full physically-based shading: soft shadows, ambient occlusion, Fresnel,
fog, and volumetric glow — all running on the CPU at ~60 ms/frame on a
modern terminal (100×36 cells).

## Build

```sh
make          # smallest uncompressed binary (~9.3 KB)
make pack     # UPX-packed self-extracting binary (~8.4 KB)
make run      # build and run
make clean
```

Requires: `rustc` (any recent stable), GNU `ld`, GNU `strip`.  
Optional for `make pack`: UPX (defaults to `/tmp/upx-5.1.1-amd64_linux/upx`).

```sh
make pack UPX=upx   # if UPX is on PATH
```

## Run

```sh
./aurora8k          # runs for 90 seconds then exits
```

## Size analysis

| Binary | Size | Notes |
|---|---|---|
| `aurora8k` | ~9.3 KB | custom minimal ELF, 2 PHDRs, no section headers |
| `aurora8k_packed` | ~8.4 KB | UPX `--nrv2d -9 --force` self-extractor |

The theoretical minimum with perfect LZMA compression is ~5.1 KB
(4.7 KB compressed code + ~400 B decompressor stub). 4 KB is not
achievable: the scene code alone compresses to 4.7 KB which already
exceeds 4 096 bytes.

## Architecture

| File | Purpose |
|---|---|
| `aurora8k.rs` | scene geometry, lighting, camera, main loop |
| `sys.rs` | no_std Linux runtime: syscalls, inline math, I/O, timing |
| `linker.ld` | minimal ELF layout (2 PHDRs, no gaps) |
| `linker-upx.ld` | standard ELF layout required by UPX |

### sys.rs highlights

- **Raw syscalls** via `global_asm!` / inline `asm!` — no libc at all
- **`fast_sin`** — 4-term Horner polynomial, error < 2×10⁻⁵
- **`fast_atan2`** — Vigna 2-term approximation, error ≈ 0.004 rad
- **`fast_floor`** — integer cast trick, avoids `floorf` PLT call
- **`sqrtss`** — single SSE instruction via inline asm, no libm
- **`mmap`** — 1 MiB frame buffer allocated at startup (no BSS bloat)
- **Volatile writes** in `Out::push_str` prevent LLVM from emitting `memcpy`

### aurora8k.rs highlights

- **`fmn` / `fmx`** — `if`-based min/max, compiles to `minss`/`maxss`
  not `fminf`/`fmaxf` PLT calls
- **`pow38`** — x³⁸ via 5 multiplications instead of `powf(38.0)`
- **Gamma** approximated as sqrt (γ ≈ 2.0) instead of `powf(0.4545)`
- **`acos`** in star UV replaced with a linear approximation

## Linux x86-64 only
