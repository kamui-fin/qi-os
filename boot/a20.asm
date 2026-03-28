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