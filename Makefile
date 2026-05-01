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

SRCS = aurora64k.rs sys.rs

.PHONY: all clean pack run

# ── Default: smallest uncompressed binary (~9.3 KB, custom minimal ELF) ───────
all: aurora64k

aurora64k: $(SRCS) linker.ld
	$(RUSTC) aurora64k.rs --edition $(EDITION) $(RUSTFLAGS_COMMON) \
		-C link-arg=-Wl,-T,linker.ld \
		-o $@
	strip --strip-section-headers $@

# ── UPX-packed variant (~8.4 KB, self-extracting) ─────────────────────────────
pack: aurora64k_packed

aurora64k_packed: aurora64k_standard
	cp aurora64k_standard $@
	$(UPX) --nrv2d -9 --force -q $@

# Standard ELF layout required by UPX (INSERT discard script)
aurora64k_standard: $(SRCS) linker-upx.ld
	$(RUSTC) aurora64k.rs --edition $(EDITION) $(RUSTFLAGS_COMMON) \
		-C link-arg=-Wl,-T,linker-upx.ld \
		-o $@

# ── Helpers ───────────────────────────────────────────────────────────────────
run: aurora64k
	./aurora64k

clean:
	rm -f aurora64k aurora64k_standard aurora64k_packed \
	      librust_out.rmeta *.rcgu.o
