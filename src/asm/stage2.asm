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

    mov si, msg_switching_pm
    call print
    jmp switch_protected_mode

    ; ==========================================
    ; 16-BIT ERROR HANDLERS
    ; ==========================================
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


; TODO: load all useful BIOS info and provide to kernel thru memory
; query_bios:

; The cpuid instruction was introduced with the Pentium. To see if it's there, we try to flip Bit 21 (the ID bit) in the EFLAGS register. If we can flip it and it stays flipped, the CPU supports cpuid.
check_cpuid:
    pushfd                  ; Push EFLAGS to stack
    pop eax                 ; Pop into EAX
    mov ecx, eax            ; Save original for later
    xor eax, 1 << 21        ; Flip bit 21
    push eax                ; Push modified value
    popfd                   ; Pop into EFLAGS

    pushfd                  ; Push EFLAGS back to stack
    pop eax                 ; Pop it back into EAX
    
    push ecx                ; Restore original EFLAGS
    popfd
    
    xor eax, ecx            ; Compare original vs modified
    jz .no_cpuid            ; If bit didn't change, CPUID isn't supported
    mov eax, 1              ; SUCCESS: Set eax to 1
    ret

    .no_cpuid:
        xor eax, eax            ; FAILURE: Clear eax to 0 (technically already 0, but this is explicit)
        ret

long_mode_is_supported:
    ;********************************************************************;
    ; Check if Long mode is supported                                    ;
    ;--------------------------------------------------------------------;
    ; Returns: eax = 0 if Long mode is NOT supported, else non-zero.     ;
    ;********************************************************************;
    mov eax, 0x80000000 ; Test if extended processor info in available.  
    cpuid                
    cmp eax, 0x80000001 
    jb .not_supported     
    mov eax, 0x80000001 ; After calling CPUID with EAX = 0x80000001, 
    cpuid               ; all AMD64 compliant processors have the longmode-capable-bit
    test edx, (1 << 29) ; (bit 29) turned on in the EDX (extended feature flags).
    jz .not_supported   ; If it's not set, there is no long mode.
    ret

    .not_supported:
        xor eax, eax
        ret

enable_a20:
    ; --- 1. Check if it's already enabled ---
    call check_a20
    cmp ax, 1
    je .done

    ; --- 2. Try BIOS Method ---
    ; Most modern BIOSes support this interrupt
    mov ax, 0x2401
    int 0x15
    
    call check_a20
    cmp ax, 1
    je .done

    ; --- 3. Try "Fast A20" (Port 92) ---
    ; This works on almost all modern hardware/emulators
    in al, 0x92
    or al, 2           ; Set bit 1 (A20 Enable)
    and al, 0xFE       ; Ensure bit 0 is 0 (prevents accidental reset)
    out 0x92, al

    call check_a20
    cmp ax, 1
    je .done

    ; --- 4. If all else fails, ret ax = 0 ---
    ; (In a real OS, you'd print an error message here)
    mov ax, 0
    ret

    .done:
        mov ax, 1
        ret

; A simplified check: Does memory at 1MB+16 wrap to 0?
check_a20:
    push ds
    push es
    cli
    
    xor ax, ax
    mov es, ax         ; ES = 0x0000
    not ax
    mov ds, ax         ; DS = 0xFFFF
    
    ; Compare 0000:0500 with FFFF:0510 (which is 1MB + 0x0500)
    mov di, 0x0500
    mov si, 0x0510
    
    mov al, [es:di]    ; Original value at 0x0500
    push ax
    
    mov byte [es:di], 0x00
    mov byte [ds:si], 0xFF
    
    cmp byte [es:di], 0xFF
    
    pop ax
    mov [es:di], al    ; Restore original value
    
    mov ax, 0          ; Assume disabled
    je .disabled       ; If the 0xFF we wrote to DS:SI showed up at ES:DI
    mov ax, 1          ; Otherwise, it's enabled!

    .disabled:
        sti
        pop es
        pop ds
        ret

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


; setup_paging:
;     ; zero out 20kib region of page entries
;     mov edi, PML4_ADDR
;     push edi
;     xor eax, eax
;     mov ecx, 5*0x1000/4
;     cld
;     rep stosd
;     pop edi

;     ; virt mem for first 2 MB
;     ; fill out PML4_ADDR
;     mov DWORD [edi], PDPT_LOW & PT_ADDR_MASK | PT_PRESENT | PT_WRITE
;     mov edi, PDPT_LOW
;     mov DWORD [edi], PD_LOW & PT_ADDR_MASK | PT_PRESENT | PT_WRITE
;     mov edi, PD_LOW
;     mov DWORD [edi], 0 & PT_ADDR_MASK | PT_PRESENT | PT_WRITE | PT_LARGE_SIZE

;     ; high half kernel mapping
;     ; alloc 1 gib
;     mov edi, PML4_ADDR+PML4_INDEX*8
;     mov DWORD [edi], PDPT_HIGH & PT_ADDR_MASK | PT_PRESENT | PT_WRITE
;     mov edi, PDPT_HIGH+PDPT_INDEX*8
;     mov DWORD [edi], PD_HIGH & PT_ADDR_MASK | PT_PRESENT | PT_WRITE

;     mov edi, PD_HIGH
;     mov eax, PT_PRESENT | PT_WRITE | PT_LARGE_SIZE
;     mov ecx, 512 
;     .kernel_map_loop:
;         mov [edi], eax                   ; Store low 32 bits (entry is 8 bytes) 
;         mov dword [edi + 4], 0           ; Store high 32 bits 
;         add edi, 8                       ; Advance to next PD_HIGH entry 
;         add eax, 0x200000               ; Increment address by 2MiB
;         loop .kernel_map_loop


; enable_longmode:
;     ; [ ] Enable PAE (Physical Address Extension): Set bit 5 in CR4.
;     mov eax, cr4
;     or eax, 1 << 5
;     mov cr4, eax
;     ; [ ] Load CR3: Point the CPU to the address of your PML4 table.
;     mov eax, PML4_ADDR
;     mov cr3, eax
;     ; [ ] Enable Long Mode: Set the LME (Long Mode Enable) bit in the EFER MSR (Model Specific Register).
;     EFER_MSR equ 0xC0000080
;     EFER_LM_ENABLE equ 1 << 8

;     mov ecx, EFER_MSR
;     rdmsr
;     or eax, EFER_LM_ENABLE
;     wrmsr
;     ; [ ] Enable Paging: Set bit 31 in CR0. Now the CPU is in "Compatibility Mode."
;     mov eax, cr0
;     or eax, (1 << 31) | 1
;     mov cr0, eax
;     ; [ ] Load 64-bit GDT: (Often combined with step 5, but must be active now).
;     lgdt [gdt_descriptor] 
;     ; [ ] The Final Far Jump: jmp 0x08:kernel_entry. This officially puts you in 64-bit Long Mode.
;     jmp CODE_SEG:kernel_entry

; ---- 32-bit GDT (The Stepping Stone) ----
gdt_start_32:
    gdt_null_32: 
        dq 0x0

    gdt_code_32: 
        dw 0xffff    ; Limit (bits 0-15) - 0xFFFF for 4GB (with granularity)
        dw 0x0       ; Base (bits 0-15)
        db 0x0       ; Base (bits 16-23)
        db 10011010b ; Access: Present, Ring 0, Exec/Read
        db 11001111b ; Flags: Granularity (4KB), 32-bit size, Limit (16-19)
        db 0x0       ; Base (bits 24-31)

    gdt_data_32: 
        dw 0xffff    ; Limit (bits 0-15)
        dw 0x0       ; Base (bits 0-15)
        db 0x0       ; Base (bits 16-23)
        db 10010010b ; Access: Present, Ring 0, Read/Write
        db 11001111b ; Flags: Granularity (4KB), 32-bit size, Limit (16-19)
        db 0x0       ; Base (bits 24-31)
gdt_end_32:

gdt_descriptor_32:
    dw gdt_end_32 - gdt_start_32 - 1
    dd gdt_start_32 ; Use dd (32-bit) for the address

; ---- 64 bit version -----
gdt_start:
    gdt_null: 
        dq 0x0
        
    gdt_code:
        ; base and limit are ignored in 64-bit mode, 
        dw 0         ; Limit (low 16 bits)     = 2 bytes
        dw 0         ; Base (low 16 bits)      = 2 bytes
        db 0         ; Base (middle 8 bits)    = 1 byte
        db 10011010b ; Access Byte             = 1 byte (Present, Ring 0, Code, Exec/Read)
        db 00100000b ; Flags                   = 1 byte (Bit 5 is Long Mode)
        db 0         ; Base (high 8 bits)      = 1 byte
    gdt_data:
        dw 0         ; Limit                   = 2 bytes
        dw 0         ; Base                    = 2 bytes
        db 0         ; Base                    = 1 byte
        db 10010010b ; Access Byte             = 1 byte (Present, Ring 0, Data, R/W)
        db 00000000b ; Flags                   = 1 byte
        db 0         ; Base                    = 1 byte
gdt_end:

gdt_descriptor:
    dw gdt_end - gdt_start - 1
    dd gdt_start

PML4_ADDR equ 0xB000
PDPT_LOW equ (PML4_ADDR + 0x1000)
PD_LOW equ (PML4_ADDR + 0x2000)
PDPT_HIGH equ (PML4_ADDR + 0x3000)
PD_HIGH equ (PML4_ADDR + 0x4000)

KERNEL_VIRT_BASE equ 0xFFFFFFFF80000000

PML4_INDEX   equ ((KERNEL_VIRT_BASE >> 39) & 0x1FF)  ; Bits 47-39
PDPT_INDEX   equ ((KERNEL_VIRT_BASE >> 30) & 0x1FF)  ; Bits 38-30
PD_INDEX     equ ((KERNEL_VIRT_BASE >> 21) & 0x1FF)  ; Bits 29-21
PT_INDEX     equ ((KERNEL_VIRT_BASE >> 12) & 0x1FF)  ; Bits 20-12

; the page table only uses certain parts of the actual address
PT_ADDR_MASK equ 0xFFFFF000
PT_PRESENT equ 1                 ; marks the entry as in use
PT_WRITE equ (1 << 1)                ; marks the entry as r/w
PT_LARGE_SIZE equ (1 << 7)               ; Bit 7 => 2MB page size, ignore PT

CODE_SEG equ 0x8
DATA_SEG equ 0x10

; --- Strings ---
msg_stage2:       db 'Stage 2 Init...', 13, 10, 0
msg_cpuid_ok:     db 'CPUID OK.', 13, 10, 0
msg_lm_ok:        db 'Long Mode OK.', 13, 10, 0
msg_a20_ok:       db 'A20 Enabled.', 13, 10, 0
msg_switching_pm: db 'Entering 32-bit PM...', 13, 10, 0

err_cpuid:        db 'ERR: No CPUID.', 13, 10, 0
err_lm:           db 'ERR: No Long Mode.', 13, 10, 0
err_a20:          db 'ERR: A20 Failed.', 13, 10, 0
msg_loaded: db 'Stage 2 has loaded!!', 13, 10, 0x0

times 512 - ($ - $$) % 512 db 0
