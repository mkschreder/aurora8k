UPX        ?= /tmp/upx-5.1.1-amd64_linux/upx
RUSTC      ?= rustc
EDITION     = 2021
FLAMEGRAPH ?= $(HOME)/.cargo/bin/flamegraph

RUSTFLAGS_COMMON = \
	-C opt-level=z \
	-C panic=abort \
	-C lto=fat \
	-C codegen-units=1 \
	-C strip=symbols \
	-C relocation-model=static \
	-C link-arg=-nostdlib \
	-C link-arg=-Wl,--build-id=none \
	-C link-arg=-Wl,--no-eh-frame-hdr

# GL build: dynamically links libEGL + libGL; no relocation-model=static,
# no -nostdlib (dynamic linker needs standard ELF layout).
# -nostartfiles keeps our own _start while still emitting a dynamic section.
RUSTFLAGS_GL = \
	-C opt-level=z \
	-C panic=abort \
	-C lto=fat \
	-C codegen-units=1 \
	-C strip=symbols \
	-C link-arg=-nostartfiles \
	-C link-arg=-Wl,--build-id=none \
	-C link-arg=-Wl,--no-eh-frame-hdr

# Profile build: same opts but with debug symbols and build-id for flamegraph
RUSTFLAGS_PROF = \
	-C opt-level=z \
	-C panic=abort \
	-C lto=fat \
	-C codegen-units=1 \
	-g \
	-C relocation-model=static \
	-C link-arg=-nostdlib \
	-C link-arg=-Wl,--build-id=sha1 \
	-C link-arg=-Wl,--no-eh-frame-hdr

SRCS8  = aurora8k.rs sys.rs
SRCS16 = aurora16k.rs sys.rs

# Must match W, PH in aurora16k.rs.
AURORA16_W := 640
AURORA16_H := 360

.PHONY: all clean pack run run16 record16 profile8 profile16

# ── Default: aurora8k (8 KB uncompressed) + aurora16k GL (GPU-accelerated) ───
all: aurora8k aurora8k_packed aurora16k

aurora8k: $(SRCS8) linker.ld
	$(RUSTC) aurora8k.rs --edition $(EDITION) $(RUSTFLAGS_COMMON) \
		-C link-arg=-Wl,-T,linker.ld \
		-o $@
	strip --strip-section-headers $@

# ── aurora8k UPX-packed variant ──────────────────────────────────────────────
pack: aurora8k_packed

aurora8k_packed: aurora8k_standard
	cp aurora8k_standard $@
	$(UPX) --nrv2d -9 --force -q $@

aurora8k_standard: $(SRCS8) linker-upx.ld
	$(RUSTC) aurora8k.rs --edition $(EDITION) $(RUSTFLAGS_COMMON) \
		-C link-arg=-Wl,-T,linker-upx.ld \
		-o $@

# ── aurora16k: GPU-accelerated via EGL + OpenGL 3.3 fragment shader ──────────
# Raymarcher runs entirely on the GPU; CPU only sets uniforms + reads pixels.
# Streams raw RGB24 to stdout — pipe to ffplay (run16) or ffmpeg (record16).
aurora16k_standard: $(SRCS16) linker-gl.ld
	$(RUSTC) aurora16k.rs --edition $(EDITION) $(RUSTFLAGS_GL) \
		-C link-arg=-Wl,-T,linker-gl.ld \
		-o $@

aurora16k: aurora16k_standard
	@ls -lh $<

# ── Helpers ───────────────────────────────────────────────────────────────────
run: aurora8k
	./aurora8k

# Stream GPU-rendered aurora16k to a window via ffplay.
# GPU renders each 320×180 frame in milliseconds; ffplay updates in real time.
# GStreamer alternative (if gst-launch-1.0 is installed):
#   ./aurora16k_standard | gst-launch-1.0 fdsrc fd=0 do-timestamp=true \
#     ! rawvideoparse width=$(AURORA16_W) height=$(AURORA16_H) format=rgb \
#     ! videoconvert ! autovideosink sync=false
run16: aurora16k_standard
	./aurora16k_standard | ffplay -f rawvideo -pixel_format rgb24 \
	  -video_size $(AURORA16_W)x$(AURORA16_H) -i pipe:0

# Record the full 90 s animation to MP4 (requires ffmpeg).
# Passes "record" as argv[1] so aurora16k uses fixed 1/30 s steps: every
# animation frame is rendered and captured in order, no matter how long each
# frame takes.  ffmpeg receives exactly 30 frames per animation-second.
record16: aurora16k_standard
	./aurora16k_standard record | ffmpeg -f rawvideo -pixel_format rgb24 \
	  -video_size $(AURORA16_W)x$(AURORA16_H) -framerate 30 -i pipe:0 \
	  -c:v libx264 -crf 18 aurora16k.mp4

# ── Flamegraph profiling (requires ~/.cargo/bin/flamegraph + perf) ─────────────
# Build with debug symbols (no strip) then capture a flamegraph SVG.
# Usage: make profile8   → produces flamegraph8k.svg
#        make profile16  → produces flamegraph16k.svg
aurora8k_prof: $(SRCS8) linker-upx.ld
	$(RUSTC) aurora8k.rs --edition $(EDITION) $(RUSTFLAGS_PROF) \
		-C link-arg=-Wl,-T,linker-upx.ld \
		-o $@

aurora16k_prof: $(SRCS16) linker-gl.ld
	$(RUSTC) aurora16k.rs --edition $(EDITION) $(RUSTFLAGS_GL) \
		-g \
		-C link-arg=-Wl,-T,linker-gl.ld \
		-o $@

profile8: aurora8k_prof
	$(FLAMEGRAPH) --no-inline -o flamegraph8k.svg -- ./aurora8k_prof > /dev/null

profile16: aurora16k_prof
	$(FLAMEGRAPH) --no-inline -o flamegraph16k.svg -- ./aurora16k_prof > /dev/null

clean:
	rm -f aurora8k aurora8k_standard aurora8k_packed \
	      aurora16k aurora16k_standard \
	      aurora8k_prof aurora16k_prof \
	      flamegraph8k.svg flamegraph16k.svg perf.data \
	      aurora16k.mp4 \
	      librust_out.rmeta *.rcgu.o
