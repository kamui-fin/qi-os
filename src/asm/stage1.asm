;###############
; %%%% stage1 %%%%
;################

BITS 16
ORG 0x7C00

jmp 0x0000:start ; far jump to set CS to 0

start:
    ; zero out segment registers and init stack pointer temp
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    mov sp, 0x7C00
    sti

    mov [boot_drive], dl

    ; read strings proper direction
    cld
    
    mov si, bootup_message
    call print
    
    .load_disk_stage_2:
        mov si, STAGE2_DAP
        mov ah, 0x42
        int 0x13
        jc .disk_error

        jmp 0x0000:stage2_start

    .disk_error:
        mov si, msg_disk_error
        call print
        jmp spin

spin:
    hlt
    jmp spin

; si = address to string
print:
    mov ah, 0x0e
    .loop:
        lodsb

        test al, al
        jz .done; if al = 0, hit the end of str
        int 10h
        jmp .loop
    .done:
        ret


STAGE2_SECTORS equ (stage2_end - stage2_start) / 512
    
align 4
STAGE2_DAP:
    db 0x10
    db 0
    dw STAGE2_SECTORS
    dw stage2_start ; mem offset
    dw 0
    dq 1 ; stage2 starts at sector 2


boot_drive: db 0
bootup_message: db 'XiangQi OS is booting up', 13, 10, 0x0
msg_disk_error: db 'Disk read error!', 13, 10, 0x0

times 510-($-$$) db 0
dw 0xAA55 ; magic number
