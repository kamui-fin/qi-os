use core::sync::atomic::{AtomicPtr, Ordering};

use alloc::{collections::vec_deque::VecDeque, sync::Arc, vec::Vec};
use lazy_static::lazy_static;
use x86_64::{
    instructions::{hlt, interrupts},
    VirtAddr,
};

use crate::{
    interrupts::{ELAPSED, TIME_SLICE},
    lapic::{mycpu, Cpu},
    serial_println,
    spinlock::{Spinlock, SpinlockGuard},
    task::{
        proc::ProcessControlBlock,
        thread::{BlockReason, ThreadControlBlock, ThreadId, ThreadState},
    },
};

// currently every core takes the same lock on every tick and wake, ping pongs between cores

// threads will build up working cashe and then be swapped

// outline:
// have a per core run list
// teh ready queues will be thread safe and lock free
// use work stealing so that a core with nothing to do will look in other cores
//      run lists for work to take
// have each cores scheduler judge how 'full' it is and try to move tasks to other cores
// mark sleeping threads as runnable AND dead, the scheduler will try to run them eventually and notice

pub static MAX_TASKS: usize = 15;

#[no_mangle]
pub extern "C" fn release_scheduler_hook() {
    SCHEDULER.force_release();
}

#[inline]
fn switch_to_task_wrapper(
    old_thread: *const ThreadControlBlock,
    next_thread: *mut ThreadControlBlock,
) {
    // Interrupts MUST be disabled when calling this function
    assert!(!x86_64::instructions::interrupts::are_enabled());
    let cpu = mycpu();
    serial_println!("cli depth: {:#?}", cpu.irq_disable_depth);
    cpu.curr_thread.store(next_thread, Ordering::Relaxed);

    unsafe {
        if let Some(pcb) = (*next_thread).pcb.clone() {
            let pcb_ptr = Arc::as_ptr(&pcb) as *mut Spinlock<ProcessControlBlock>;
            cpu.proc.store(pcb_ptr, Ordering::Relaxed);
        }
    }

    unsafe {
        (*cpu.tss).privilege_stack_table[0] = VirtAddr::new((*next_thread).rsp0 as u64);
    }

    unsafe {
        switch_to_task(old_thread, next_thread);
    }
}

extern "C" {
    pub fn switch_to_task(
        old_thread: *const ThreadControlBlock,
        next_thread: *const ThreadControlBlock,
    );
}

lazy_static! {
    pub static ref SCHEDULER: Spinlock<Scheduler> = Spinlock::new(Scheduler::new());
}

pub struct Scheduler {
    pub threads: Vec<ThreadControlBlock>,
    //pub ready_queue: VecDeque<ThreadId>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            threads: Vec::with_capacity(MAX_TASKS),
            //ready_queue: VecDeque::with_capacity(MAX_TASKS),
        }
    }

    pub fn pick_next_thread(&mut self) -> Option<*mut ThreadControlBlock> {
        let cpu = mycpu();
        //serial_println!("{:#?}", cpu.ready_queue);
        serial_println!(
            "{:#?}",
            self.threads.iter().map(|t| t.id).collect::<Vec<u64>>()
        );
        if let Some(next_id) = cpu.ready_queue.lock().pop_front() {
            let next_thread = self
                .threads
                .iter_mut()
                .find(|t| t.id == next_id)
                .expect("thread not found");
            next_thread.state = ThreadState::Running;
            Some(next_thread as *mut ThreadControlBlock)
        } else {
            None
        }
    }

    pub fn spawn(&mut self, id: ThreadId, return_addr: *const ()) {
        let cpu = mycpu();
        let new_thread = ThreadControlBlock::new(id, return_addr, None, None, None);
        self.threads.push(new_thread);

        if id > 2 {
            cpu.ready_queue.lock().push_back(id);
        }
    }

    pub fn unblock_task(&mut self, id: ThreadId) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            if let ThreadState::Blocked(_) = thread.state {
                let cpu = mycpu();
                thread.state = ThreadState::Ready;
                cpu.ready_queue.lock().push_back(id);

                cpu.needs_resched.store(true, Ordering::SeqCst);
            }
        }
    }
}

pub fn scheduler_loop() -> ! {
    let cpu = mycpu();
    // need to enable interrupts first, handle and then enter lock
    loop {
        interrupts::enable();

        if cpu.ready_queue.lock().is_empty() {
            hlt();
            continue;
        }

        cpu.needs_resched.store(false, Ordering::SeqCst);

        let curr_thread = unsafe { &mut *cpu.curr_thread.load(Ordering::Relaxed) };

        if curr_thread.state == ThreadState::Running && curr_thread.id != 1 {
            curr_thread.state = ThreadState::Ready;
            curr_thread.time_slice_remaining = TIME_SLICE;
            cpu.ready_queue.lock().push_back(curr_thread.id);
        }

        let mut scheduler = SCHEDULER.lock();
        let next_thread = scheduler.pick_next_thread().unwrap();

        switch_to_task_wrapper(curr_thread, next_thread);
        drop(scheduler);
    }
}

pub fn terminate_task(status: u8) {
    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.unblock_task(2); // 2 is cleaner task
    }

    block_task(BlockReason::Terminated(status));
}

fn switch_to_scheduler() {
    // assert!(is_holding(SCHEDULER))
    // assert!(mycpu().ncli == 1)
    // assert!(curr_thread.state != RUNNING)
    // assert!(interrupts_enabled == false)
    let cpu = mycpu();
    serial_println!(" CPU {:#?} swapped to scheduler", cpu.apic_id);
    let int_enabled = cpu.int_enabled.load(Ordering::SeqCst);
    let sched_thread = cpu.main_sched_thread.load(Ordering::SeqCst);

    switch_to_task_wrapper(cpu.curr_thread.load(Ordering::SeqCst), sched_thread);

    cpu.int_enabled.store(int_enabled, Ordering::SeqCst)
}

pub fn switch_if_needed() {
    let needs_resched = x86_64::instructions::interrupts::without_interrupts(|| {
        mycpu().needs_resched.load(Ordering::Relaxed)
    });
    if needs_resched {
        yield_sched()
    }
}

pub fn yield_sched() {
    let mut scheduler = SCHEDULER.lock();
    let cpu = mycpu();
    let curr_thread = unsafe { &mut *cpu.curr_thread.load(Ordering::Relaxed) };
    if curr_thread.id == 1 {
        return;
    }

    curr_thread.state = ThreadState::Ready;
    curr_thread.time_slice_remaining = TIME_SLICE;

    cpu.ready_queue.lock().push_back(curr_thread.id);
    cpu.ready_queue.force_release();

    switch_to_scheduler();
}

pub fn block_task(reason: BlockReason) {
    let _guard = SCHEDULER.lock();

    let cpu = mycpu();
    let curr_thread = unsafe { &mut *cpu.curr_thread.load(Ordering::Relaxed) };
    serial_println!("Blocking {} for {:#?}", curr_thread.id, reason);
    curr_thread.state = ThreadState::Blocked(reason);
    curr_thread.time_slice_remaining = TIME_SLICE;

    cpu.needs_resched.store(true, Ordering::SeqCst);

    switch_to_scheduler();
}

pub fn block_task_with_lock<'a, T>(
    reason: BlockReason,
    old_guard: SpinlockGuard<'a, T>,
    lock: &'a Spinlock<T>,
) -> SpinlockGuard<'a, T> {
    let cpu = mycpu();
    let irq_count = cpu.irq_disable_depth.load(Ordering::Relaxed);
    assert!(irq_count > 1, "only allowed to hold 1 spinlock");

    let _sched_guard = SCHEDULER.lock();

    old_guard.into_raw();
    lock.force_release();

    let curr_thread = unsafe { &mut *cpu.curr_thread.load(Ordering::Relaxed) };
    curr_thread.state = ThreadState::Blocked(reason);
    curr_thread.time_slice_remaining = TIME_SLICE;

    cpu.needs_resched.store(true, Ordering::SeqCst);

    switch_to_scheduler();

    drop(_sched_guard); // follow strict lock ordering
    lock.lock()
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

pub fn get_time_since_boot() -> u64 {
    ELAPSED.load(Ordering::Relaxed) * 1_000_000
}
