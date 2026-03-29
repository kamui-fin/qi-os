gdt_start_unreal:
    dq 0x0                  ; Null Descriptor
    ; Data Segment: Base=0, Limit=0xFFFFF, G=1 (4GB), Read/Write
    dw 0xFFFF, 0x0000, 0x9200, 0x00CF
gdt_end_unreal:

gdt_descriptor_unreal:
    dw gdt_end_unreal - gdt_start_unreal - 1
    dd gdt_start_unreal

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

CODE_SEG equ 0x8
DATA_SEG equ 0x10