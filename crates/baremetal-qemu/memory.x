/* QEMU mps2-an386 (Cortex-M4) memory map. QEMU models generous regions;
   these origins match the AN386 image layout (code at 0x0, SRAM at 0x20000000). */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 4M
  RAM   : ORIGIN = 0x20000000, LENGTH = 4M
}
