/* QEMU `virt` (RISC-V VirtIO board) memory map. Unlike the ARM mps2-an386
   board's separate FLASH/RAM split (memory.x), `-kernel` on `virt` loads a
   bare-metal ELF straight into RAM at 0x8000_0000 with no separate flash
   region, so every riscv-rt REGION_ alias below maps onto the same RAM
   block. 16 MiB is ample for this smoke test's code + heap + stack. */
MEMORY
{
  RAM : ORIGIN = 0x80000000, LENGTH = 16M
}

REGION_ALIAS("REGION_TEXT", RAM);
REGION_ALIAS("REGION_RODATA", RAM);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);
