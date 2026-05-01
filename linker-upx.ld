/* UPX-compatible discard script — augments the default linker script.
   Keeps the standard ELF PT_LOAD layout (LOAD at file offset 0) so UPX
   can recognise and pack the binary. All debug/dyn sections are discarded. */

SECTIONS {
  /DISCARD/ : {
    *(.eh_frame*)
    *(.comment)
    *(.interp)
    *(.note*)
    *(.gnu.hash*)
    *(.gnu.version*)
    *(.dynsym)
    *(.dynstr)
    *(.dynamic)
    *(.rela.*)
    *(.got*)
    *(.plt*)
  }
}
INSERT AFTER .text;
