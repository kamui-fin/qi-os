stage1_start:
    %include "stage1.asm"
    align 512
stage1_end:

stage2_start:
    %include "stage2.asm"
    align 512
stage2_end:

; TODO: enable graphics mode
kernel_blob_start:
    incbin "../kernel.bin"
kernel_blob_end: