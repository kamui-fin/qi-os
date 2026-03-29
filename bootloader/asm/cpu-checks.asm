; The cpuid instruction was introduced with the Pentium. To see if it's there, we try to flip Bit 21 (the ID bit) in the EFLAGS register. If we can flip it and it stays flipped, the CPU supports cpuid.
check_cpuid:
    pushfd                  ; Push EFLAGS to stack
    pop eax                 ; Pop into EAX
    mov ecx, eax            ; Save original for later
    xor eax, 1 << 21        ; Flip bit 21
    push eax                ; Push modified value
    popfd                   ; Pop into EFLAGS

    pushfd                  ; Push EFLAGS back to stack
    pop eax                 ; Pop it back into EAX
    
    push ecx                ; Restore original EFLAGS
    popfd
    
    xor eax, ecx            ; Compare original vs modified
    jz .no_cpuid            ; If bit didn't change, CPUID isn't supported
    mov eax, 1              ; SUCCESS: Set eax to 1
    ret

    .no_cpuid:
        xor eax, eax            ; FAILURE: Clear eax to 0 (technically already 0, but this is explicit)
        ret

long_mode_is_supported:
    ;********************************************************************;
    ; Check if Long mode is supported                                    ;
    ;--------------------------------------------------------------------;
    ; Returns: eax = 0 if Long mode is NOT supported, else non-zero.     ;
    ;********************************************************************;
    mov eax, 0x80000000 ; Test if extended processor info in available.  
    cpuid                
    cmp eax, 0x80000001 
    jb .not_supported     
    mov eax, 0x80000001 ; After calling CPUID with EAX = 0x80000001, 
    cpuid               ; all AMD64 compliant processors have the longmode-capable-bit
    test edx, (1 << 29) ; (bit 29) turned on in the EDX (extended feature flags).
    jz .not_supported   ; If it's not set, there is no long mode.
    ret

    .not_supported:
        xor eax, eax
        ret
