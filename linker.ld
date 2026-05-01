/* Minimal linker script for aurora64k — produces the smallest possible ELF.
   Merges ELF header, PHDRs, .text and .rodata into one RX segment at
   file offset 0 — no gaps, no page-sized holes.
   2 PHDRs total: LOAD(RX) + LOAD(RW-NOBITS) = 176 bytes of header overhead. */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS {
  code  PT_LOAD  FLAGS(5);          /* R+X: ELF hdr, PHDRs, code, rodata */
  bss   PT_LOAD  FLAGS(6);          /* R+W: small BSS (BUF_PTR pointer)   */
}

SECTIONS {
  /* Start sections right after ELF header (64) + 2 PHDRs (112) = 176 bytes
     so they land at the correct file offset with no page-gap. */
  . = 0x400000 + SIZEOF_HEADERS;

  .text : {
    *(.text*)
    *(.rodata*)
  } :code

  . = ALIGN(0x1000);

  .bss (NOLOAD) : {
    *(.bss*)
  } :bss

  /DISCARD/ : {
    *(.eh_frame*)  *(.comment)   *(.interp)
    *(.note*)      *(.gnu.*)     *(.dynsym)
    *(.dynstr)     *(.dynamic)   *(.rela.*)
    *(.got*)       *(.plt*)
  }
}
