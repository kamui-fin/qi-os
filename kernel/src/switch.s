.intel_syntax noprefix
.global switch_to_task

.extern CURR_THREAD_PTR 
.extern TSS_POINTER

# fn switch_to_task(
#       next_thread: *const ThreadControlBlock,
# );
#
# WARNING: Caller is expected to disable IRQs before calling, and enable IRQs again after function returns


# SYSTEM V ABI
# ARGS: rdi, rsi, rdx, rcx, r8, r9
# calle saved: rbx, rbp, r12, r13, 14, 15

switch_to_task:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15

    mov rax, [rip+CURR_THREAD_PTR]    # edi = address of the previous task's "thread control block"
    mov [rax+0], rsp        # Save rsp for previous task's kernel stack in the thread's TCB

    # Load next task's state

    mov rsi, [rsp+8]         # rsi = address of the next task's "thread control block" (parameter passed on stack)
    mov [rip+CURR_THREAD_PTR], rsi    # Current task's TCB is the next task TCB

    mov rsp, [rsi+0]         # Load rsp for next task's kernel stack from the thread's TCB
    mov rbx, [rsi+(1*8)]        # ebx = address for the top of the next task's kernel stack
    mov rax, [rsi+(2*8)]         # eax = address of page directory for next task

    mov rdx, [rip+TSS_POINTER]
    mov [rdx+4], rbx            # Adjust the rsp0 field in the TSS (used by CPU for for CPL=3 -> CPL=0 privilege level changes)

    mov rcx, cr3                   # ecx = previous task's virtual address space
    cmp rax, rcx                   # Does the virtual address space need to being changed?
    je .doneVAS                   # no, virtual address space is the same, so don't reload it and cause TLB flushes
    mov cr3, rax                   # yes, load the next task's virtual address space


.doneVAS:
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx

    ret                           # Load next task's EIP from its kernel stack
