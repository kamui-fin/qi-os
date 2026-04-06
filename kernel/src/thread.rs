use alloc::vec;
use core::{
    arch::asm,
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    ptr::from_ref,
    sync::atomic::AtomicUsize,
};

use crate::{
    interrupts::ELAPSED,
    lock::{PreemptIrqLock, SimpleIrqLock, NEEDS_RESCHEDULE, PREEMPT_COUNT},
    serial_println,
};
use alloc::{boxed::Box, collections::vec_deque::VecDeque, vec::Vec};
use lazy_static::lazy_static;
use x86_64::{
    instructions::interrupts::without_interrupts,
    structures::paging::{FrameAllocator, Mapper, Page, PageSize, PageTableFlags, Size4KiB},
    VirtAddr,
};

lazy_static! {
    pub static ref SCHEDULER: SimpleIrqLock<Scheduler> = SimpleIrqLock::new(Scheduler::new());
    pub static ref PREEMPT_DISABLE: PreemptIrqLock<()> = PreemptIrqLock::new(());
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlockReason {
    Paused,
    Sleep(u64), // sleep expiry
    Terminated,
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked(BlockReason),
}

pub type ThreadId = u64;

#[repr(C)]
#[derive(Debug)]
pub struct ThreadControlBlock {
    rsp: *const usize,
    rsp0: *const usize, // kernel stack pointer to use when entering kernel
    cr3: *const usize,
    pub state: ThreadState,
    pub id: ThreadId,
    stack: Option<Box<[usize]>>,
    // parent_process_id: ProcessId
}

const KERNEL_STACK_SIZE: usize = 256 * Size4KiB::SIZE as usize;

impl ThreadControlBlock {
    // New kernel task
    pub fn new(id: u64, return_address: *const ()) -> Self {
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
            id,
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
            state: ThreadState::Running,
            id: 0,
        }
    }
}

unsafe impl Send for ThreadControlBlock {}
unsafe impl Sync for ThreadControlBlock {}

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

pub struct Scheduler {
    pub threads: Vec<Box<ThreadControlBlock>>,
}

#[no_mangle]
pub static mut CURR_THREAD_PTR: *mut ThreadControlBlock = core::ptr::null_mut();
pub static mut MAIN_THREAD: *mut ThreadControlBlock = core::ptr::null_mut();

impl Scheduler {
    pub fn new() -> Self {
        Self {
            threads: Vec::with_capacity(5),
        }
    }

    pub fn schedule(&mut self) {
        // TODO: for safety, create a wrapper around switch_to_task and add this too
        // TODO: idle task must have LOWEST priority
        if PREEMPT_COUNT.load(core::sync::atomic::Ordering::Relaxed) != 0 {
            NEEDS_RESCHEDULE.store(true, core::sync::atomic::Ordering::Relaxed);
            return;
        }
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

    pub fn spawn(&mut self, id: u64, return_addr: *const ()) {
        let new_thread = Box::new(ThreadControlBlock::new(id, return_addr));
        self.threads.push(new_thread);
    }

    pub fn unblock_task(&mut self, id: u64) {
        let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
        let num_threads = self.threads.len();
        let thread = self.threads.iter_mut().find(|t| t.id == id);
        let mut should_preempt = false;
        if let Some(thread) = thread {
            thread.state = ThreadState::Ready;

            // If we're currently running idle task OR there's literally no other threads
            if num_threads == 0 || curr_thread.id == 1 {
                should_preempt = true;
            }
        }

        if should_preempt {
            self.schedule();
        }
    }
}

pub fn block_task(reason: BlockReason) {
    let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
    curr_thread.state = ThreadState::Blocked(reason);

    let mut scheduler = SCHEDULER.lock();
    scheduler.schedule();
}

fn get_time_since_boot() -> u64 {
    ELAPSED.load(core::sync::atomic::Ordering::Relaxed) * 1_000_000
}

fn nano_sleep(nano_sec: u64) {
    nano_sleep_until(get_time_since_boot() + nano_sec);
}

fn nano_sleep_until(abs_time: u64) {
    // just get a guard to prevent a sudden context switch after if statement which might
    // invalidate time contract
    let _guard = PREEMPT_DISABLE.lock();
    if abs_time <= get_time_since_boot() {
        return;
    }
    block_task(BlockReason::Sleep(abs_time));
}

fn terminate_task() {
    let _guard = PREEMPT_DISABLE.lock();

    block_task(BlockReason::Terminated);

    let mut scheduler = SCHEDULER.lock();
    scheduler.unblock_task(2); // 2 is cleaner task
}

/* thread_control_block *terminated_task_list = NULL;

void terminate_task(void) {

    // Note: Can do any harmless stuff here (close files, free memory in user-space, ...) but there's none of that yet

    lock_stuff();

    // Put this task on the terminated task list

    lock_scheduler();
    current_task_TCB->next = terminated_task_list;
    terminated_task_list = current_task_TCB;
    unlock_scheduler();

    // Block this task (note: task switch will be postponed until scheduler lock is released)

    block_task(TERMINATED);

    // Make sure the cleaner task isn't paused

    unblock_task(cleaner_task);

    // Unlock the scheduler's lock

    unlock_stuff();
}
 */

pub fn yield_sched() {
    unsafe {
        let curr_thread = &mut *CURR_THREAD_PTR;
        curr_thread.state = ThreadState::Ready;

        let mut scheduler = SCHEDULER.lock();
        scheduler.schedule();
    }
}
