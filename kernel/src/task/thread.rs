use crate::interrupts::TIME_SLICE;
use crate::spinlock::Spinlock;
use crate::task::proc::ProcessControlBlock;
use crate::task::scheduler::terminate_task;
use alloc::boxed::Box;
use alloc::{sync::Arc, vec};
use core::{
    arch::{asm, naked_asm},
    ptr::from_ref,
};
use x86_64::structures::paging::{PageSize, Size2MiB};

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub enum BlockReason {
    Paused,
    Sleep(u64), // sleep expiry
    Terminated(u8),
    CompositorWait,
    WaitPipeRead(u64),
    WaitPipeWrite(u64),
    WaitThread(u64),
    WaitStdin(u8),
    TtyRenderWait,
    AsyncExecutorWait,
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
pub struct ThreadControlBlock {
    pub rsp: *const usize,
    pub rsp0: *const usize, // kernel stack pointer to use when entering kernel
    pub cr3: *const usize,
    pub state: ThreadState,
    pub id: ThreadId,
    pub stack: Box<[usize]>,
    pub time_slice_remaining: usize, // resets to 100 ms upon context switch

    pub pcb: Option<Arc<Spinlock<ProcessControlBlock>>>,
}

pub const KERNEL_STACK_SIZE: usize = 1 * Size2MiB::SIZE as usize;

#[unsafe(naked)]
pub unsafe extern "C" fn task_startup_hook() {
    naked_asm!(
        "call {release_sched}",
        "sti",
        "call r12",
        "call {terminate}",
        release_sched = sym crate::task::scheduler::release_scheduler_hook,
        terminate = sym crate::task::scheduler::terminate_task,
    );
}

impl ThreadControlBlock {
    // New kernel task
    pub fn new(
        id: u64,
        return_address: *const (),
        cr3: Option<*const usize>,
        rip: Option<u64>,
        rsp: Option<u64>,
    ) -> Self {
        let max_stack_len = KERNEL_STACK_SIZE / core::mem::size_of::<usize>();
        // TODO: guard page
        let mut stack: Box<[usize]> = vec![0usize; max_stack_len].into_boxed_slice();

        stack[max_stack_len - 8] = 0; // r15
        stack[max_stack_len - 7] = rsp.unwrap_or_default() as usize; // r14
        stack[max_stack_len - 6] = rip.unwrap_or_default() as usize; // r13
        stack[max_stack_len - 5] = return_address as usize; // r12
        stack[max_stack_len - 4] = 0; // rbp
        stack[max_stack_len - 3] = 0; // rbx
        stack[max_stack_len - 2] = (task_startup_hook as *const ()).addr(); // actual return addr

        let rsp = from_ref(&stack[max_stack_len - 8]);
        let rsp0 = from_ref(&stack[max_stack_len - 1]);

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
            stack,
            rsp,
            rsp0,
            cr3,
            state: ThreadState::Ready,
            time_slice_remaining: TIME_SLICE,
            id,
            pcb: None,
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
            stack: Box::new([]),
            pcb: None,
            rsp,
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
