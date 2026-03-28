load_kernel:
    ; === 1. UNREAL MODE SETUP (Done once) ===
    cli
    lgdt [gdt_descriptor_unreal]
    mov eax, cr0
    or al, 1
    mov cr0, eax            ; Protected Mode ON
    
    mov bx, 0x08
    mov fs, bx              ; FS is now our "Big" 4GB segment
    
    and al, 0xFE
    mov cr0, eax            ; Protected Mode OFF
    jmp 0:.flush            ; Clear the pipeline
.flush:
    xor ax, ax
    mov ds, ax              ; Standard DS for data
    mov es, ax              ; Standard ES for BIOS
    sti

    ; === 2. INITIALIZE VARIABLES ===
    mov edi, 0x100000       ; Destination: 1MB
    mov edx, STAGE2_SECTORS + 1 
    mov ecx, KERNEL_COUNT   

.load_loop:
    test ecx, ecx
    jz .done_loading
    
    ; Calculate chunk (max 64 sectors)
    mov ebx, 64
    cmp ecx, ebx
    jae .size_fixed
    mov ebx, ecx
.size_fixed:
    
    mov [KERNEL_DAP + 2], bx
    mov [KERNEL_DAP + 8], edx

    push ecx                ; Save total remaining
    push ebx                ; Save current chunk size

    ; === 3. BIOS READ ===
    mov dl, [boot_drive]
    mov si, KERNEL_DAP
    mov ah, 0x42
    int 0x13
    jc stage2_entrypoint.kernel_disk_error

    ; === 4. THE CLEAN MOVE (Manual Loop) ===
    pop ebx                 ; Get chunk size back
    mov ecx, ebx
    shl ecx, 7              ; Convert sectors to dwords (512 / 4 = 128)
    
    push ds
    mov ax, 0x1000          ; Source Buffer
    mov ds, ax
    xor esi, esi
    
.copy_chunk:
    a32 mov eax, [ds:esi]   ; Read 4 bytes from BIOS buffer
    a32 mov [fs:edi], eax   ; Write 4 bytes to 1MB+ using FS (4GB limit!)
    add esi, 4
    add edi, 4
    loop .copy_chunk
    
    pop ds                  ; Restore DS=0

    ; === 5. INCREMENT ===
    add edx, ebx            ; Advance LBA
    pop ecx                 ; Restore total remaining
    sub ecx, ebx
    jnz .load_loop


.done_loading:
    mov si, msg_kernel_loaded
    call print
    ret

align 8
KERNEL_DAP:
    db 0x10    ; size
    db 0       ; reserved
    dw 0       ; sector count (Loop fills this)
    dw 0       ; offset (0x0000)
    dw 0x1000  ; segment (0x1000:0x0000 = 0x10000)
    dq 0       ; LBA (Loop fills this)

KERNEL_COUNT equ (kernel_blob_end - kernel_blob_start + 511) / 512