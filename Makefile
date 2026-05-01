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

.PHONY: all clean pack run run16 profile8 profile16

# ── Default: smallest uncompressed binary (~8.1 KB, custom minimal ELF) ────────
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

# ── aurora16k: expanded 16 KB UPX-compressed variant ────────────────────────
# Uncompressed standard ELF (measure with: wc -c aurora16k_standard)
aurora16k_standard: $(SRCS16) linker-upx.ld
	$(RUSTC) aurora16k.rs --edition $(EDITION) $(RUSTFLAGS_COMMON) \
		-C link-arg=-Wl,-T,linker-upx.ld \
		-o $@

# UPX-packed target (goal: ≤16 384 bytes)
aurora16k: aurora16k_standard
	cp aurora16k_standard $@
	$(UPX) --nrv2d -9 --force -q $@

# ── Helpers ───────────────────────────────────────────────────────────────────
run: aurora8k
	./aurora8k

run16: aurora16k_standard
	./aurora16k_standard

# ── Flamegraph profiling (requires ~/.cargo/bin/flamegraph + perf) ─────────────
# Build with debug symbols (no strip) then capture a flamegraph SVG.
# Usage: make profile8   → produces flamegraph8k.svg
#        make profile16  → produces flamegraph16k.svg
aurora8k_prof: $(SRCS8) linker-upx.ld
	$(RUSTC) aurora8k.rs --edition $(EDITION) $(RUSTFLAGS_PROF) \
		-C link-arg=-Wl,-T,linker-upx.ld \
		-o $@

aurora16k_prof: $(SRCS16) linker-upx.ld
	$(RUSTC) aurora16k.rs --edition $(EDITION) $(RUSTFLAGS_PROF) \
		-C link-arg=-Wl,-T,linker-upx.ld \
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
	      librust_out.rmeta *.rcgu.o
