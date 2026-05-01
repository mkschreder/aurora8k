UPX     ?= /tmp/upx-5.1.1-amd64_linux/upx
RUSTC   ?= rustc
EDITION  = 2021

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

SRCS8  = aurora8k.rs sys.rs
SRCS16 = aurora16k.rs sys.rs

.PHONY: all clean pack run run16

# ── Default: smallest uncompressed binary (~8.1 KB, custom minimal ELF) ────────
all: aurora8k

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

clean:
	rm -f aurora8k aurora8k_standard aurora8k_packed \
	      aurora16k aurora16k_standard \
	      librust_out.rmeta *.rcgu.o
