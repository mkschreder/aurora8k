/* linker-gl.ld — for the dynamically-linked GL build of aurora16k.
 * Discards debug/note bloat but keeps PLT, GOT, and the dynamic section
 * intact so EGL/GL shared libraries load correctly.                       */
SECTIONS {
  /DISCARD/ : {
    *(.eh_frame*)
    *(.comment)
    *(.note*)
    *(.gnu.build*)
    *(.debug*)
  }
}
INSERT AFTER .text;
