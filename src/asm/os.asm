stage1_start:
    %include "stage1.asm"
    align 512
stage1_end:

stage2_start:
    %include "stage2.asm"
    align 512
stage2_end:

; TODO: enable graphics mode
;       load kernel into ram (parse ELF)
;       far jump to kernel entry.
; For now we will use this dummy kernel

kernel_entry:
    hlt

; BITS 64
; kernel_entry:
;     mov rsp, 0x90000                
;     mov ax, DATA_SEG
;     mov ds, ax
;     mov es, ax
;     mov fs, ax
;     mov gs, ax
;     mov ss, ax

;     ; Print a '64' to the screen to celebrate (VGA 0xB8000)
;     mov rdi, 0xB8000                
;     mov dword [rdi], 0x1F341F36

;     hlt

