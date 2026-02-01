/* Custom rodata section ordering from esp-hal 1.0 to fix App Descriptor placement */
/* This overrides esp-hal 0.23's rodata.x to place .flash.appdesc FIRST */

SECTIONS {
  /* For ESP App Description, must be placed first in image (PR #4745 fix) */
  .flash.appdesc : ALIGN(4)
  {
      KEEP(*(.flash.appdesc));
      KEEP(*(.flash.appdesc.*));
  } > RODATA

  /* Merge section to ensure proper alignment */
  .rodata_merge : ALIGN (4) {
    . = ALIGN(ALIGNOF(.rodata));
  } > RODATA

  .rodata : ALIGN(4)
  {
    . = ALIGN (4);
    _rodata_start = ABSOLUTE(.);
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
    . = ALIGN(4);
    _rodata_end = ABSOLUTE(.);
  } > RODATA

  .rodata.wifi : ALIGN(4)
  {
    . = ALIGN(4);
    *( .rodata_wlog_*.* )
    . = ALIGN(4);
  } > RODATA
}
