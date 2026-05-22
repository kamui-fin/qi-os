section .text

global trampoline_start
global trampoline_end

CODE32_SEG equ 0x08
DATA32_SEG equ 0x10
CODE64_SEG equ 0x18
DATA64_SEG equ 0x20

TRAMPOLINE_DATA equ 0x8F00 
CR3_OFFSET      equ TRAMPOLINE_DATA + 0
STACK_OFFSET    equ TRAMPOLINE_DATA + 8
ENTRY_OFFSET    equ TRAMPOLINE_DATA + 16
READY_OFFSET    equ TRAMPOLINE_DATA + 24
APIC_ID_OFFSET    equ TRAMPOLINE_DATA + 32
CPU_PTR_OFFSET    equ TRAMPOLINE_DATA + 40

trampoline_start:

[bits 16]
ap_trampoline:
    cli
    cld
    jmp 0:(0x8000 + (setup_32bit - trampoline_start))

align 64
setup_32bit:
    xor ax, ax
    mov ds, ax
    lgdt [0x8000 + (gdt_descriptor - trampoline_start)]
    ; PE bit
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    jmp CODE32_SEG:(0x8000 + (setup_kernel - trampoline_start))

[bits 32]
setup_kernel:
    ; data seg
    mov ax, DATA32_SEG
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    ; enable PAE
    mov eax, cr4
    or eax, 1 << 5
    mov cr4, eax
    ; load cr3
    mov eax, [CR3_OFFSET]
    mov cr3, eax
    ; enable long mode
    EFER_MSR equ 0xC0000080
    EFER_LM_ENABLE equ 1 << 8
    mov ecx, EFER_MSR
    rdmsr
    or eax, EFER_LM_ENABLE
    wrmsr
    ; enable paging
    mov eax, cr0
    or eax, (1 << 31) | 1
    mov cr0, eax

    jmp CODE64_SEG:(0x8000 + (wait_for_bsp - trampoline_start))


[bits 64]
wait_for_bsp:
    mov ax, DATA64_SEG
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    mov rsp, [STACK_OFFSET]

    mov rax, [ENTRY_OFFSET]
    mov rdi, [APIC_ID_OFFSET]
    mov rsi, [CPU_PTR_OFFSET]

    mov byte [READY_OFFSET], 1

    jmp rax

align 16
gdt_start:
    dq 0x0                        ; 0x00: Null
    dq 0x00cf9a000000ffff         ; 0x08: 32-bit Code
    dq 0x00cf92000000ffff         ; 0x10: 32-bit Data
    dq 0x00209A0000000000         ; 0x18: 64-bit Code
    dq 0x0000920000000000         ; 0x20: 64-bit Data
gdt_end:

gdt_descriptor:
    dw gdt_end - gdt_start - 1
    dd (0x8000 + (gdt_start - trampoline_start))

trampoline_end:
