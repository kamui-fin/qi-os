ENTRY(_start)

KERNEL_OFFSET = 0xFFFFFFFF80000000;
KERNEL_START = 0x100000;

SECTIONS {
    . = KERNEL_OFFSET + KERNEL_START;

    .text : ALIGN(4K) {
        KEEP(*(.text._start))
        *(.text*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata*)
    }

    .data : ALIGN(4K) {
        *(.data*)
    }

    .bss : ALIGN(4K) {
        *(.bss*)
    }
}