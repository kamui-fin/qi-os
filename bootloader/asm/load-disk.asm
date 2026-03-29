load_all_components:
    ; === 1. UNREAL MODE SETUP (Done once) ===
    cli
    lgdt [gdt_descriptor_unreal]
    mov eax, cr0
    or al, 1
    mov cr0, eax            ; Protected Mode ON
    
    mov bx, 0x08            ; Your 4GB data segment index
    mov fs, bx              
    
    and al, 0xFE
    mov cr0, eax            ; Protected Mode OFF
    jmp 0:.flush            
.flush:
    xor ax, ax
    mov ds, ax
    mov es, ax
    sti

    ; === 2. LOAD KERNEL (ELF) ===
    ; Destination: 0x1000000 (16MB High Memory)
    mov edi, 0x1000000
    mov edx, KERNEL_LBA
    mov ecx, KERNEL_COUNT
    call load_sectors

    ; === 3. LOAD STAGE 3 (Rust) ===
    ; Destination: 0x10000 (Low Memory)
    mov edi, 0x10000       
    mov edx, STAGE3_LBA
    mov ecx, STAGE3_COUNT
    call load_sectors

    mov si, msg_kernel_loaded
    call print
    ret

; --- Reusable Loading Routine ---
; Input: EDI = Dest, EDX = Start LBA, ECX = Sector Count
load_sectors:
    test ecx, ecx
    jz .done
    
    push ecx                ; Save total remaining
    
    ; Calculate chunk (max 64 sectors)
    mov ebx, 64
    cmp ecx, ebx
    jae .size_fixed
    mov ebx, ecx
.size_fixed:
    
    mov [COMMON_DAP + 2], bx
    mov [COMMON_DAP + 8], edx

    push edx                ; Save current LBA
    push ebx                ; Save current chunk size

    ; BIOS READ
    mov dl, [boot_drive]
    mov si, COMMON_DAP
    mov ah, 0x42
    int 0x13
    jc stage2_entrypoint.kernel_disk_error   ; Defined elsewhere in your file

    ; THE UNREAL MOVE
    pop ebx                 ; Chunk size
    mov ecx, ebx
    shl ecx, 7              ; sectors to dwords
    
    push ds
    mov ax, 0x1000          ; Temporary BIOS buffer (0x10000)
    mov ds, ax
    xor esi, esi
    
.copy_loop:
    a32 mov eax, [ds:esi]
    a32 mov [fs:edi], eax
    add esi, 4
    add edi, 4
    loop .copy_loop
    
    pop ds
    pop edx                 ; Current LBA
    pop ecx                 ; Total remaining
    
    add edx, ebx            ; Advance LBA
    sub ecx, ebx            ; Decrease remaining
    jnz load_sectors        ; Loop if more sectors
.done:
    ret

align 8
COMMON_DAP:
    db 0x10, 0
    dw 0                    ; Count
    dw 0                    ; Offset
    dw 0x1000               ; Segment (0x10000)
    dq 0                    ; LBA

; --- Constants for the loader ---
STAGE3_COUNT equ (STAGE3_BYTES + 511) / 512
KERNEL_COUNT equ (KERNEL_BYTES + 511) / 512

STAGE3_LBA   equ (1 + STAGE2_SECTORS)
KERNEL_LBA   equ (STAGE3_LBA + STAGE3_COUNT)