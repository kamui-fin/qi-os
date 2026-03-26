# Minimal Bootloader

## Procedure

- real -> protected mode
- setup valid GDT 
- flip Protection Enable bit in CR0
- Far jump to clear CPU pre-fetch queue of 16-bit instructions
- setup stack
- init segment registers to 0 always
- setup page table