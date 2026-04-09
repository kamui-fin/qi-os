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

    # save old task's rsp
    mov rax, [rip+CURR_THREAD_PTR]
    mov [rax+0], rsp

    # point CURR_THREAD_PTR to new task
    mov [rip+CURR_THREAD_PTR], rdi

    
    # switch stacks
    mov rsp, [rdi+0]      
    mov rbx, [rdi+(1*8)] 
    mov rax, [rdi+(2*8)]

    mov rdx, [rip+TSS_POINTER]
    mov [rdx+4], rbx           

    mov rcx, cr3              
    cmp rax, rcx             
    je .doneVAS             
    mov cr3, rax           


.doneVAS:
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx

    ret                           # Load next task's RIP from its kernel stack
