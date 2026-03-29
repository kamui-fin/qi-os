;###############
; %%%% stage2 %%%%
;################

BITS 16

stage2_entrypoint:
    mov si, msg_loaded
    call print

    call check_cpuid
    cmp eax, 0
    jz .error_no_cpuid
    mov si, msg_cpuid_ok
    call print

    call long_mode_is_supported
    cmp eax, 0
    je .error_no_long_mode
    mov si, msg_lm_ok
    call print

    call enable_a20 
    test ax, ax
    jz .error_a20
    mov si, msg_a20_ok
    call print

    call get_memory_map

    call load_all_components

    mov ax, 1280
    mov bx, 1024
    mov cl, 16
    call vbe_set_mode

    ; mov si, msg_switching_pm
    ; call print
    jmp switch_protected_mode

    ; ==========================================
    ; 16-BIT ERROR HANDLERS
    ; ==========================================
    .kernel_disk_error:
        mov si, msg_kernel_disk_error
        call print
        
        mov si, msg_newline
        jmp .halt

    .error_no_cpuid:
        mov si, err_cpuid
        jmp .halt
    .error_no_long_mode:
        mov si, err_lm
        jmp .halt
    .error_a20:
        mov si, err_a20
        jmp .halt

    .halt:
        call print
        cli
    .spin:
        hlt
        jmp .spin

%include "cpu-checks.asm"
%include "a20.asm"
%include "load-disk.asm"
%include "hwinfo.asm"
%include "graphics.asm"

switch_protected_mode:
    cli

    ;;; point to GDT
    lgdt [gdt_descriptor_32]

    ;;; PE bit
    mov eax, cr0
    or eax, 1
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

%include "paging.asm"

enable_longmode:
    ; [ ] Enable PAE (Physical Address Extension): Set bit 5 in CR4.
    mov eax, cr4
    or eax, 1 << 5
    mov cr4, eax
    ; [ ] Load CR3: Point the CPU to the address of your PML4 table.
    mov eax, PML4_ADDR
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
    or eax, (1 << 31) | 1
    mov cr0, eax
    ; [ ] Load 64-bit GDT: (Often combined with step 5, but must be active now).
    lgdt [gdt_descriptor] 
    
    ; [ ] The Final Far Jump: jmp 0x08:stage3_start. This officially puts you in 64-bit Long Mode.
    mov edi, screen
    jmp CODE_SEG:0x10000


%include "gdt.asm"

msg_stage2:       db 'Stage 2 Init...', 13, 10, 0
msg_cpuid_ok:     db 'CPUID OK.', 13, 10, 0
msg_lm_ok:        db 'Long Mode OK.', 13, 10, 0
msg_a20_ok:       db 'A20 Enabled.', 13, 10, 0
msg_switching_pm: db 'Entering 32-bit PM...', 13, 10, 0
msg_newline:        db 13, 10, 0
msg_kernel_disk_error: db 'ERR: Unable to load the rust kernel.', 0
msg_loaded: db 'Stage 2 has loaded!!', 13, 10, 0x0
msg_kernel_loaded: db 'Successfully loaded ENTIRE kernel from disk!', 13, 10, 0x0
err_cpuid:        db 'ERR: No CPUID.', 13, 10, 0
err_lm:           db 'ERR: No Long Mode.', 13, 10, 0
err_a20:          db 'ERR: A20 Failed.', 13, 10, 0

times 512 - ($ - $$) % 512 db 0
