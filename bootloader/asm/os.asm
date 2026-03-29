stage1_start:
    %include "stage1.asm"
    align 512
stage1_end:

stage2_start:
    %include "stage2.asm"
    align 512
stage2_end: