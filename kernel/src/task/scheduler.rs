/*
 Note that you can have a "task start up function" that is executed when a new task first gets CPU time and
 does a few initialisation things and then passes control to the task's normal code.
 In this case the new kernel stack will include a "return EIP" that contains the address
 of the "task start up function", plus an extra "return EIP"
 (for when the "task start up function" returns) that contains the address of the task itself
 (taken from an input parameter of the "create_kernel_task()" function).
*/
fn task_startup_hook() {
    SCHEDULER.force_release();
}

use core::sync::atomic::Ordering;

use alloc::{collections::vec_deque::VecDeque, sync::Arc, vec::Vec};
use lazy_static::lazy_static;
use x86_64::instructions::interrupts;

use crate::{
    interrupts::{ELAPSED, TIME_SLICE},
    lapic::mycpu,
    serial_println,
    spinlock::{Spinlock, SpinlockGuard},
    task::thread::{BlockReason, ThreadControlBlock, ThreadId, ThreadState},
};

static MAX_TASKS: usize = 15;

extern "C" {
    pub fn switch_to_task(next_thread: *const ThreadControlBlock);
}

lazy_static! {
    pub static ref SCHEDULER: Spinlock<Scheduler> = Spinlock::new(Scheduler::new());
}

pub struct Scheduler {
    pub threads: Vec<ThreadControlBlock>,
    pub ready_queue: VecDeque<ThreadId>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            threads: Vec::with_capacity(MAX_TASKS),
            ready_queue: VecDeque::with_capacity(MAX_TASKS),
        }
    }

    pub fn pick_next_thread(&mut self) -> *mut ThreadControlBlock {
        if let Some(next_id) = self.ready_queue.pop_front() {
            let next_thread = self
                .threads
                .iter_mut()
                .find(|t| t.id == next_id)
                .expect("thread not found");
            next_thread.state = ThreadState::Running;
            next_thread as *mut ThreadControlBlock
        } else {
            let cpu = mycpu();
            let idle_thread = unsafe { &mut *cpu.main_sched_thread.load(Ordering::Relaxed) };
            idle_thread.state = ThreadState::Running;
            idle_thread
        }
    }

    pub fn spawn(&mut self, id: ThreadId, return_addr: *const ()) {
        let new_thread = ThreadControlBlock::new(id, return_addr, None, None, None);
        self.threads.push(new_thread);

        if id > 2 {
            self.ready_queue.push_back(id);
        }
    }

    pub fn unblock_task(&mut self, id: ThreadId) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            if let ThreadState::Blocked(_) = thread.state {
                thread.state = ThreadState::Ready;
                self.ready_queue.push_back(id);

                let cpu = mycpu();
                cpu.needs_resched.store(true, Ordering::SeqCst);
            }
        }
    }
}

pub fn scheduler_loop() {
    let cpu = mycpu();

    loop {
        let irq_count = cpu.irq_disable_depth.load(Ordering::Relaxed);
        assert!(irq_count > 0, "BUG: scheduling while atomic!");

        interrupts::enable();

        let mut scheduler = SCHEDULER.lock();

        cpu.needs_resched.store(false, Ordering::SeqCst);

        let curr_thread = unsafe { &mut *cpu.curr_thread.load(Ordering::Relaxed) };
        if curr_thread.state == ThreadState::Running && curr_thread.id != 1 {
            curr_thread.state = ThreadState::Ready;
            curr_thread.time_slice_remaining = TIME_SLICE;

            scheduler.ready_queue.push_back(curr_thread.id);
        }

        let next_thread = scheduler.pick_next_thread();

        cpu.curr_thread.store(next_thread, Ordering::Relaxed);

        unsafe {
            switch_to_task(next_thread);
        }
    }
}

fn switch_to_scheduler() {
    // assert!(is_holding(SCHEDULER))
    // assert!(mycpu().ncli == 1)
    // assert!(curr_thread.state != RUNNING)
    // assert!(interrupts_enabled == false)

    let cpu = mycpu();
    let int_enabled = cpu.int_enabled.load(Ordering::SeqCst);
    let sched_thread = cpu.main_sched_thread.load(Ordering::SeqCst);
    unsafe {
        switch_to_task(sched_thread);
    }
    cpu.int_enabled.store(int_enabled, Ordering::SeqCst)
}

pub fn yield_sched() {
    let mut scheduler = SCHEDULER.lock();

    let cpu = mycpu();
    let curr_thread = unsafe { &mut *cpu.curr_thread.load(Ordering::Relaxed) };
    curr_thread.state = ThreadState::Ready;
    curr_thread.time_slice_remaining = TIME_SLICE;
    scheduler.ready_queue.push_back(curr_thread.id);

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

pub fn terminate_task(status: u8) {
    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.unblock_task(2); // 2 is cleaner task
    }

    block_task(BlockReason::Terminated(status));
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
