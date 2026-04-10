ENTRY(_start)

SECTIONS {
    . = 0x400000; /* following Linux x86-64 ELF standard */

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
