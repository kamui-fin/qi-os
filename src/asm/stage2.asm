; check if long mode is supported
; enable A20 gate
; load all useful BIOS info
; paging
; switch to protected mode
; - disable interrupt, set GDT, set PE bit of cr0

; The steps for enabling long mode are:
; Disable paging
; Set the PAE enable bit in CR4
; Load CR3 with the physical address of the PML4 (Level 4 Page Map)
; Enable long mode by setting the LME flag (bit 8) in MSR 0xC0000080 (aka EFER)
; Enable paging
; Now the CPU will be in compatibility mode, and instructions are still 32-bit. To enter long mode, the D/B bit (bit 22, 2nd 32-bit value) of the GDT code segment must be clear (as it would be for a 16-bit code segment), and the L bit (bit 21, 2nd 32-bit value) of the GDT code segment must be set. Once that is done, the CPU is in 64-bit long mode. 

; TODO: enable graphics mode
; load kernel into ram (parse ELF)
; far jump to kernel entry

;###############
; %%%% stage2 %%%%
;################

BITS 16

mov si, msg_loaded
call print

query_bios:

long_mode_is_supported:

enable_a20:

switch_protected_mode:
    cli

    ;;; point to GDT
    lgdt [gdt_descriptor_32]

    ;;; PE bit
    mov eax, cr0
    or al, 1
    mov cr0, eax

    ;;;; FLUSH PIPELINE
    jmp CODE_SEG:begin_protected_mode

    ;;; YAY we're in 32 bit mode now
    BITS 32
    begin_protected_mode:
        mov ax, DATA_SEG
        mov ds, ax
        mov ss, ax
        mov es, ax
        mov fs, ax
        mov gs, ax

        mov ebp, 0x90000
        mov esp, ebp


setup_paging:


enable_longmode:

; [ ] Enable PAE (Physical Address Extension): Set bit 5 in CR4.
mov eax, cr4
or eax, 1 << 5
mov cr4, eax
; [ ] Load CR3: Point the CPU to the address of your PML4 table.
mov eax, PML4_TABLE
mov cr3, eax
; [ ] Enable Long Mode: Set the LME (Long Mode Enable) bit in the EFER MSR (Model Specific Register).
EFER_MSR equ 0xC0000080
EFER_LM_ENABLE equ 1 << 8

mov ecx, EFER_MSR
rdmsr
or eax, EFER_LM_ENABLE
wrmsr
; [ ] Enable Paging: Set bit 31 in CR0. Now the CPU is in "Compatibility Mode."
mov eax, cr0
or eax, 1 << 31 
mov cr0, eax
; [ ] Load 64-bit GDT: (Often combined with step 5, but must be active now).
lgdt [rel gdt_descriptor_64] ;; TODO: DO WE REALLY NEED rel??
; [ ] The Final Far Jump: jmp 0x08:kernel_entry. This officially puts you in 64-bit Long Mode.
jmp CODE_SEG:kernel_entry

BITS 64
kernel_entry:
    mov ax, DATA_SEG
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; Print a '64' to the screen to celebrate (VGA 0xB8000)
    mov rdi, 0xb8000
    mov rax, 0x1F341F36 ; '6' and '4' with blue background
    mov [rdi], rax

    hlt

gdt_start_32:
    gdt_null_32: 
        dq 0x0
    gdt_code_32: 
        dw 0xffff, 0x0, 0x0
        db 10011010b
        db 11001111b ; Bit 6 (D) is 1, Bit 5 (L) is 0
        db 0x0

    ; --- 32-Bit Data Segment (Index 2: 0x10) ---
    gdt_data_32: 
        dw 0xffff, 0x0, 0x0
        db 10010010b
        db 11001111b
        db 0x0
gdt_end_32:
gdt_descriptor_32:
    dw gdt_end_32 - gdt_start_32 - 1 ; Size of our GDT , always less one of the true size
    dd gdt_start_32 ; Start address of our GDT

gdt_start_64:
    gdt_null_64: 
        dq 0x0
    gdt_code_64:
        dw 0, 0, 0   ; Limit/Base ignored in 64-bit
        db 10011010b ; Present, Ring 0, Code, Exec/Read
        db 00100000b ; Bit 5 (L) is 1 (Long Mode!), Bit 6 (D) is 0
        db 0x0
    gdt_data_64:
        dw 0, 0, 0
        db 10010010b ; Present, Ring 0, Data, R/W
        db 00000000b
        db 0x0
gdt_end_64:
gdt_descriptor_64:
    dw gdt_end_64 - gdt_start_64 - 1
    dd gdt_start_64



CODE_SEG equ 0x8
DATA_SEG equ 0x10

msg_loaded: db 'Stage 2 has loaded!!', 13, 10, 0x0

PML4_TABLE equ 0xB000

times 512 - ($ - $$) % 512 db 0
