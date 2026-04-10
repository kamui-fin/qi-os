use alloc::vec;
use core::{
    arch::{asm, naked_asm},
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    ptr::from_ref,
    sync::atomic::AtomicUsize,
};

use crate::{
    interrupts::{ELAPSED, TIME_SLICE},
    lock::{SimpleIrqLock, IRQ_DISABLE_COUNTER, NEEDS_RESCHEDULE},
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
    pub time_slice_remaining: usize, // resets to 100 ms upon context switch
}

const KERNEL_STACK_SIZE: usize = 1 * Size4KiB::SIZE as usize;

#[unsafe(naked)]
pub unsafe extern "C" fn task_startup_hook() {
    naked_asm!(
        "sti",
        "call r12",
        /* "call {terminate}",
        terminate = sym crate::thread::terminate_task, */
    );
}

impl ThreadControlBlock {
    // New kernel task
    pub fn new(id: u64, return_address: *const (), cr3: Option<*const usize>) -> Self {
        let max_stack_len = KERNEL_STACK_SIZE / core::mem::size_of::<usize>();
        let mut stack: Box<[usize]> = vec![0usize; max_stack_len].into_boxed_slice();

        stack[max_stack_len - 8] = 0; // r15
        stack[max_stack_len - 7] = 0; // r14
        stack[max_stack_len - 6] = 0; // r13
        stack[max_stack_len - 5] = return_address as usize; // r12
        stack[max_stack_len - 4] = 0; // rbp
        stack[max_stack_len - 3] = 0; // rbx
        stack[max_stack_len - 2] = (task_startup_hook as *const ()).addr(); // actual return addr

        let rsp = from_ref(&stack[max_stack_len - 8]);
        let rsp0 = from_ref(&stack[max_stack_len - 1]);

        // TODO: currently assume all threads are in kernel space
        let cr3 = cr3.unwrap_or_else(|| {
            let cr3: *const usize;
            unsafe {
                asm!(r#"
                mov {}, cr3
            "#, out(reg) cr3)
            }
            cr3
        });

        Self {
            stack: Some(stack),
            rsp,
            rsp0,
            cr3,
            state: ThreadState::Ready,
            time_slice_remaining: TIME_SLICE,
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
            time_slice_remaining: TIME_SLICE,
            id: 1,
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
    pub ready_queue: VecDeque<ThreadId>,
}

#[no_mangle]
pub static mut CURR_THREAD_PTR: *mut ThreadControlBlock = core::ptr::null_mut();
pub static mut MAIN_THREAD: *mut ThreadControlBlock = core::ptr::null_mut();

static MAX_TASKS: usize = 15;

impl Scheduler {
    pub fn new() -> Self {
        Self {
            threads: Vec::with_capacity(MAX_TASKS),
            ready_queue: VecDeque::with_capacity(MAX_TASKS),
        }
    }

    pub fn pick_next_thread(&mut self) -> Option<*mut ThreadControlBlock> {
        if let Some(next_id) = self.ready_queue.pop_front() {
            let next_thread = self
                .threads
                .iter_mut()
                .find(|t| t.id == next_id)
                .expect("thread not found");
            next_thread.state = ThreadState::Running;
            Some(&mut **next_thread as *mut ThreadControlBlock)
        } else {
            let idle_thread = unsafe { &mut *MAIN_THREAD };
            idle_thread.state = ThreadState::Running;
            Some(idle_thread)
        }
    }

    pub fn spawn(&mut self, id: ThreadId, return_addr: *const ()) {
        let new_thread = Box::new(ThreadControlBlock::new(id, return_addr, None));
        self.threads.push(new_thread);

        if id > 2 {
            self.ready_queue.push_back(id);
        }
    }

    pub fn unblock_task(&mut self, id: ThreadId) {
        let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
        let num_threads = self.threads.len();
        let thread = self.threads.iter_mut().find(|t| t.id == id);
        if let Some(thread) = thread {
            thread.state = ThreadState::Ready;
            self.ready_queue.push_back(id);

            // If we're currently running idle task OR there's literally no other threads
            if num_threads == 0 || curr_thread.id == 1 {
                NEEDS_RESCHEDULE.store(true, core::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

pub fn switch_if_needed() {
    let needs_schedule = NEEDS_RESCHEDULE.load(core::sync::atomic::Ordering::SeqCst);

    if !needs_schedule {
        return;
    }

    // this should never happen!!
    let irq_count = IRQ_DISABLE_COUNTER.load(core::sync::atomic::Ordering::Relaxed);
    if irq_count > 0 {
        panic!("BUG: scheduling while atomic!");
    }

    // clear flag
    NEEDS_RESCHEDULE.store(false, core::sync::atomic::Ordering::SeqCst);
    let next_thread = {
        let mut scheduler = SCHEDULER.lock();
        scheduler.pick_next_thread()
    };

    if let Some(next_thread) = next_thread {
        unsafe {
            switch_to_task(next_thread);
        }
    }
}

pub fn block_task(reason: BlockReason) {
    {
        let _guard = SCHEDULER.lock();

        let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
        curr_thread.state = ThreadState::Blocked(reason);
        curr_thread.time_slice_remaining = TIME_SLICE;

        NEEDS_RESCHEDULE.store(true, core::sync::atomic::Ordering::SeqCst);
    }

    switch_if_needed();
}

pub fn get_time_since_boot() -> u64 {
    ELAPSED.load(core::sync::atomic::Ordering::Relaxed) * 1_000_000
}

pub fn nano_sleep(nano_sec: u64) {
    nano_sleep_until(get_time_since_boot() + nano_sec);
}

fn nano_sleep_until(abs_time: u64) {
    if abs_time <= get_time_since_boot() {
        return;
    }
    block_task(BlockReason::Sleep(abs_time));
}

pub fn terminate_task() {
    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.unblock_task(2); // 2 is cleaner task
    }

    block_task(BlockReason::Terminated);
}

pub fn yield_sched() {
    {
        let mut scheduler = SCHEDULER.lock();

        let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
        curr_thread.state = ThreadState::Ready;
        curr_thread.time_slice_remaining = TIME_SLICE;

        scheduler.ready_queue.push_back(curr_thread.id);

        NEEDS_RESCHEDULE.store(true, core::sync::atomic::Ordering::SeqCst);
    }

    switch_if_needed();
}
