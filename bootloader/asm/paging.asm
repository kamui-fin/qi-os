setup_paging:
    ; zero out 20kib region of page entries
    mov edi, PML4_ADDR
    push edi
    xor eax, eax
    mov ecx, 5*0x1000/4
    cld
    rep stosd
    pop edi

    ; virt mem for first 2 MB
    ; fill out PML4_ADDR
    mov DWORD [edi], PDPT_LOW & PT_ADDR_MASK | PT_PRESENT | PT_WRITE
    mov edi, PDPT_LOW
    mov DWORD [edi], PD_LOW & PT_ADDR_MASK | PT_PRESENT | PT_WRITE
    mov edi, PD_LOW
    mov DWORD [edi], 0 & PT_ADDR_MASK | PT_PRESENT | PT_WRITE | PT_LARGE_SIZE

    ; high half kernel mapping
    ; alloc 1 gib
    mov edi, PML4_ADDR+PML4_INDEX*8
    mov DWORD [edi], PDPT_HIGH & PT_ADDR_MASK | PT_PRESENT | PT_WRITE
    mov edi, PDPT_HIGH+PDPT_INDEX*8
    mov DWORD [edi], PD_HIGH & PT_ADDR_MASK | PT_PRESENT | PT_WRITE

    mov edi, PD_HIGH
    mov eax, PT_PRESENT | PT_WRITE | PT_LARGE_SIZE
    mov ecx, 512 
    .kernel_map_loop:
        mov [edi], eax                   ; Store low 32 bits (entry is 8 bytes) 
        mov dword [edi + 4], 0           ; Store high 32 bits 
        add edi, 8                       ; Advance to next PD_HIGH entry 
        add eax, 0x200000               ; Increment address by 2MiB
        loop .kernel_map_loop


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