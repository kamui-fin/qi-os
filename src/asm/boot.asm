BITS 16
ORG 0x7C00

jmp 0x0000:s1_entry

s1_entry:

    push dl                        ;;; temporarily saving the drive number 
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax                     ;;; Setting segment registers to 0 

    mov sp, 0x7C00
   
    mov si, word1
    call print
    jmp $


print: 
    cld
    
.loop:
    lodsb   
    cmp al, 0                       ;;; comparing if al = 0 then we reached the end of the string 
    je .done                        ;;; 'jump if equal' end loop when we reached the end of the string 
    mov ah, 0x0E                    
    int 0x10
    jmp .loop

.done:
    ret

word1: db "Hello World", 0          ;;; string with null terminator

times 510-($-$$) db 0               ;;; padding the unused bytes 

dw 0xAA55                           ;;; required boot signature at last 2 bytes

    