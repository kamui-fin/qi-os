use alloc::vec;
use core::{arch::asm, ptr::from_ref};

use alloc::{boxed::Box, collections::vec_deque::VecDeque, vec::Vec};
use x86_64::{
    instructions::interrupts::without_interrupts,
    structures::paging::{FrameAllocator, Mapper, Page, PageSize, PageTableFlags, Size4KiB},
    VirtAddr,
};

use crate::serial_println;

#[derive(Debug, PartialEq, Eq)]
pub enum BlockReason {
    Paused,
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked(BlockReason),
}

type ThreadId = &'static str;

#[repr(C)]
#[derive(Debug)]
pub struct ThreadControlBlock {
    rsp: *const usize,
    rsp0: *const usize, // kernel stack pointer to use when entering kernel
    cr3: *const usize,
    next_task: Option<*const ThreadControlBlock>,
    state: ThreadState,
    id: ThreadId,
    stack: Option<Box<[usize]>>,
    // parent_process_id: ProcessId
}

const KERNEL_STACK_SIZE: usize = 256 * Size4KiB::SIZE as usize;

impl ThreadControlBlock {
    // New kernel task
    pub fn new(name: &'static str, return_address: *const ()) -> Self {
        let max_stack_len = KERNEL_STACK_SIZE / core::mem::size_of::<usize>();
        let mut stack: Box<[usize]> = vec![0usize; max_stack_len].into_boxed_slice();

        stack[max_stack_len - 8] = 0; // rbx
        stack[max_stack_len - 7] = 0; // rbp
        stack[max_stack_len - 6] = 0; // r12
        stack[max_stack_len - 5] = 0; // r13
        stack[max_stack_len - 4] = 0; // r14
        stack[max_stack_len - 3] = 0; // r15
        stack[max_stack_len - 2] = return_address as usize;

        let rsp = from_ref(&stack[max_stack_len - 6]);
        let rsp0 = from_ref(&stack[max_stack_len - 1]);

        // TODO: currently assume all threads are in kernel space
        let cr3: *const usize;
        unsafe {
            asm!(r#"
                mov {}, cr3
            "#, out(reg) cr3)
        }

        Self {
            stack: Some(stack),
            rsp,
            rsp0,
            cr3,
            state: ThreadState::Ready,
            id: name,
            next_task: None,
        }
    }

    // Constructor for existing main kernel thread
    pub fn kmain() -> Self {
        let rsp: *const usize;
        let cr3: *const usize;

        unsafe {
            asm!(r#"
                mov {}, rsp
                mov {}, cr3
            "#, out(reg) rsp, out(reg) cr3)
        }

        Self {
            stack: None,
            rsp: rsp,
            rsp0: rsp,
            cr3,
            next_task: None,
            state: ThreadState::Running,
            id: "KMAIN",
        }
    }
}

/*
 Note that you can have a "task start up function" that is executed when a new task first gets CPU time and
 does a few initialisation things and then passes control to the task's normal code.
 In this case the new kernel stack will include a "return EIP" that contains the address
 of the "task start up function", plus an extra "return EIP"
 (for when the "task start up function" returns) that contains the address of the task itself
 (taken from an input parameter of the "create_kernel_task()" function).
*/
// fn task_startup_hook();

extern "C" {
    pub fn switch_to_task(next_thread: *const ThreadControlBlock);
}

struct IrqLock {
    counter: usize,
}

impl IrqLock {
    fn new() -> Self {
        Self { counter: 0 }
    }

    fn lock(&mut self) {
        x86_64::instructions::interrupts::disable();
        self.counter += 1;
    }

    fn unlock(&mut self) {
        self.counter -= 1;
        if self.counter == 0 {
            x86_64::instructions::interrupts::enable();
        }
    }
}

pub struct Scheduler {
    threads: Vec<Box<ThreadControlBlock>>,
    irq_lock: IrqLock,
}

#[no_mangle]
pub static mut CURR_THREAD_PTR: *mut ThreadControlBlock = core::ptr::null_mut();
pub static mut MAIN_THREAD: *mut ThreadControlBlock = core::ptr::null_mut();

impl Scheduler {
    pub fn new() -> Self {
        Self {
            threads: Vec::with_capacity(5),
            irq_lock: IrqLock::new(),
        }
    }

    pub fn lock(&mut self) {
        self.irq_lock.lock();
    }

    pub fn unlock(&mut self) {
        self.irq_lock.unlock();
    }

    pub fn schedule(&mut self) {
        let next_thread = self
            .threads
            .iter_mut()
            .find(|t| t.state == ThreadState::Ready);
        if let Some(next_thread) = next_thread {
            next_thread.state = ThreadState::Running;
            unsafe {
                switch_to_task(&**next_thread as *const ThreadControlBlock);
            }
        }
    }

    pub fn spawn(&mut self, id: &'static str, return_addr: *const ()) {
        let new_thread = Box::new(ThreadControlBlock::new(id, return_addr));
        self.threads.push(new_thread);
    }

    pub fn block_task(&mut self, reason: BlockReason) {
        self.lock();
        let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
        curr_thread.state = ThreadState::Blocked(reason);
        self.schedule();
        self.unlock();
    }

    pub fn unblock_task(&mut self, thread: *mut ThreadControlBlock) {
        self.lock();

        let thread = unsafe { &mut *thread };
        thread.state = ThreadState::Ready;

        // TODO: perhaps if there's only one thread running, we can pre-empt immediately?
        self.schedule();

        self.unlock();
    }
}

// enter back into scheduler
// TODO: figure out a way without all these globals
pub fn sched() {
    unsafe {
        let curr_thread = &mut *CURR_THREAD_PTR;
        curr_thread.state = ThreadState::Ready;
        switch_to_task(MAIN_THREAD);
    }
}

/*
   for(;;) {
        lock_scheduler();
        schedule();
        unlock_scheduler();
    }
}

void block_task(int reason) {
    lock_scheduler();
    current_task_TCB->state = reason;
    schedule();
    unlock_scheduler();
}

void unblock_task(thread_control_block * task) {
    lock_scheduler();
    if(first_ready_to_run_task == NULL) {

        // Only one task was running before, so pre-empt

        switch_to_task(task);
    } else {
        // There's at least one task on the "ready to run" queue already, so don't pre-empt

        last_ready_to_run_task->next = task;
        last_ready_to_run_task = task;
    }
    unlock_scheduler();
}
*/
