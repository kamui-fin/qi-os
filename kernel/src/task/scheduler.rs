/*
 Note that you can have a "task start up function" that is executed when a new task first gets CPU time and
 does a few initialisation things and then passes control to the task's normal code.
 In this case the new kernel stack will include a "return EIP" that contains the address
 of the "task start up function", plus an extra "return EIP"
 (for when the "task start up function" returns) that contains the address of the task itself
 (taken from an input parameter of the "create_kernel_task()" function).
*/
// fn task_startup_hook();

use alloc::{collections::vec_deque::VecDeque, sync::Arc, vec::Vec};
use lazy_static::lazy_static;
use x86_64::instructions::interrupts;

use crate::{
    interrupts::{ELAPSED, TIME_SLICE},
    lapic::mycpu,
    serial_println,
    spinlock::Spinlock,
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
    pub threads: Vec<Arc<Spinlock<ThreadControlBlock>>>,
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
            let mut next_thread = self
                .threads
                .iter_mut()
                .find(|t| t.lock().id == next_id)
                .expect("thread not found")
                .lock();
            next_thread.state = ThreadState::Running;
            &mut *next_thread as *mut ThreadControlBlock
        } else {
            let cpu = mycpu();
            let idle_thread = unsafe {
                &mut *cpu
                    .main_sched_thread
                    .load(core::sync::atomic::Ordering::Relaxed)
            };
            idle_thread.state = ThreadState::Running;
            idle_thread
        }
    }

    pub fn spawn(&mut self, id: ThreadId, return_addr: *const ()) {
        let new_thread = Arc::new(Spinlock::new(ThreadControlBlock::new(
            id,
            return_addr,
            None,
            None,
            None,
        )));
        self.threads.push(new_thread);

        if id > 2 {
            self.ready_queue.push_back(id);
        }
    }

    pub fn unblock_task(&mut self, id: ThreadId) {
        if let Some(thread_arc) = self.threads.iter().find(|t| t.lock().id == id) {
            let mut thread = thread_arc.lock();
            if let ThreadState::Blocked(_) = thread.state {
                thread.state = ThreadState::Ready;
                self.ready_queue.push_back(id);

                let cpu = mycpu();
                cpu.needs_resched
                    .store(true, core::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

pub fn scheduler_loop() {
    let cpu = mycpu();

    loop {
        interrupts::enable();

        let mut guard = SCHEDULER.lock();
        let next_thread = guard.pick_next_thread();
        switch_if_needed();

        // ptable lock automatically drops
    }
}

pub fn switch_if_needed() {
    let cpu = mycpu();
    let needs_schedule = cpu.needs_resched.load(core::sync::atomic::Ordering::SeqCst);

    if !needs_schedule {
        return;
    }

    // this should never happen!!
    let irq_count = cpu
        .irq_disable_depth
        .load(core::sync::atomic::Ordering::Relaxed);
    if irq_count > 0 {
        panic!("BUG: scheduling while atomic!");
    }

    // clear flag
    cpu.needs_resched
        .store(false, core::sync::atomic::Ordering::SeqCst);
    let next_thread = {
        let mut scheduler = SCHEDULER.lock();

        let curr_thread =
            unsafe { &mut *cpu.curr_thread.load(core::sync::atomic::Ordering::Relaxed) };
        if curr_thread.state == ThreadState::Running && curr_thread.id != 1 {
            curr_thread.state = ThreadState::Ready;
            curr_thread.time_slice_remaining = TIME_SLICE;

            scheduler.ready_queue.push_back(curr_thread.id);
        }

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

        let cpu = mycpu();
        let curr_thread =
            unsafe { &mut *cpu.curr_thread.load(core::sync::atomic::Ordering::Relaxed) };
        serial_println!("Blocking {} for {:#?}", curr_thread.id, reason);
        curr_thread.state = ThreadState::Blocked(reason);
        curr_thread.time_slice_remaining = TIME_SLICE;

        cpu.needs_resched
            .store(true, core::sync::atomic::Ordering::SeqCst);
    }

    switch_if_needed();
}

pub fn block_task_drop_lock<L>(reason: BlockReason, lock: L) {
    {
        let _guard = SCHEDULER.lock();

        let cpu = mycpu();
        let curr_thread =
            unsafe { &mut *cpu.curr_thread.load(core::sync::atomic::Ordering::Relaxed) };
        curr_thread.state = ThreadState::Blocked(reason);
        curr_thread.time_slice_remaining = TIME_SLICE;

        cpu.needs_resched
            .store(true, core::sync::atomic::Ordering::SeqCst);

        drop(lock);
    }

    switch_if_needed();
}

pub fn terminate_task(status: u8) {
    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.unblock_task(2); // 2 is cleaner task
    }

    block_task(BlockReason::Terminated(status));
}

pub fn yield_sched() {
    {
        let mut scheduler = SCHEDULER.lock();

        let cpu = mycpu();
        let curr_thread =
            unsafe { &mut *cpu.curr_thread.load(core::sync::atomic::Ordering::Relaxed) };
        curr_thread.state = ThreadState::Ready;
        curr_thread.time_slice_remaining = TIME_SLICE;

        scheduler.ready_queue.push_back(curr_thread.id);

        cpu.needs_resched
            .store(true, core::sync::atomic::Ordering::SeqCst);
    }

    switch_if_needed();
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
    ELAPSED.load(core::sync::atomic::Ordering::Relaxed) * 1_000_000
}
