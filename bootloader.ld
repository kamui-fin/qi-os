ENTRY(_start)

SECTIONS
{
    /* Stage 3 is loaded at 64KB */
    . = 0x10000;

    .text :
    {
        /* Ensure the entry point is at the very beginning of the binary */
        KEEP(*(.text._start))
        *(.text .text.*)
    }

    .rodata :
    {
        *(.rodata .rodata.*)
    }

    .data :
    {
        *(.data .data.*)
    }

    .bss :
    {
        __bss_start = .;
        *(.bss .bss.*)
        __bss_end = .;
    }

    /* Remove unnecessary ELF sections that might bloat the binary */
    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame)
        *(.rel.plt)
    }
}