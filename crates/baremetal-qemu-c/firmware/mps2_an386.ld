ENTRY(Reset_Handler)

MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 4M
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 4M
}

_estack    = ORIGIN(RAM) + LENGTH(RAM);            /* top of RAM (full-descending) */
_heap_end  = ORIGIN(RAM) + LENGTH(RAM) - 0x4000;   /* leave 16 KiB for the stack */

SECTIONS
{
  .isr_vector : { KEEP(*(.isr_vector)) } > FLASH

  /* .init/.fini must be laid out so that crti.o prologue immediately precedes
     any inserted body code and crtn.o epilogue follows — giving _init/_fini
     a proper return path.  Without this, __libc_init_array calls _init and
     never returns (falls into .fini with no pop/bx lr). */
  .init : {
    KEEP(*crti.o(.init))
    KEEP(*(.init))
    KEEP(*crtn.o(.init))
  } > FLASH

  .text : {
    *(.text*)
    *(.rodata*)
    . = ALIGN(4);
  } > FLASH

  .fini : {
    KEEP(*crti.o(.fini))
    KEEP(*(.fini))
    KEEP(*crtn.o(.fini))
    . = ALIGN(4);
  } > FLASH

  .init_array : {
    PROVIDE_HIDDEN(__preinit_array_start = .);
    KEEP(*(.preinit_array*))
    PROVIDE_HIDDEN(__preinit_array_end = .);
    PROVIDE_HIDDEN(__init_array_start = .);
    KEEP(*(SORT(.init_array.*)))
    KEEP(*(.init_array*))
    PROVIDE_HIDDEN(__init_array_end = .);
    . = ALIGN(4);
  } > FLASH

  _sidata = LOADADDR(.data);
  .data : {
    _sdata = .;
    *(.data*)
    . = ALIGN(4);
    _edata = .;
  } > RAM AT > FLASH

  .bss : {
    _sbss = .;
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    _ebss = .;
  } > RAM

  end = .;            /* newlib _sbrk heap start */
  PROVIDE(_end = .);
}
