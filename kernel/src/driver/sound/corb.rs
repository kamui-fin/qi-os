use core::{pin::Pin, task::Poll};

use alloc::boxed::Box;
use conquer_once::spin::OnceCell;
use core::arch::x86_64::_mm_clflush;
use crossbeam_queue::ArrayQueue;
use futures_util::task::Context;
use futures_util::{task::AtomicWaker, Stream};
use spin::Mutex;

pub static CORB: OnceCell<Mutex<CorbBuffer>> = OnceCell::uninit();
pub static RIRB: OnceCell<Mutex<RirbBuffer>> = OnceCell::uninit();

pub static RESP_WAKER: AtomicWaker = AtomicWaker::new();
pub static HDA_CMD_RESP_QUEUE: OnceCell<CmdResponsePairBuffer> = OnceCell::uninit();

// TODO: make sure allocator respects Layout::align()
#[repr(align(128))]
pub struct AlignedRingBuffer<T> {
    buffer: [T; 256],
}

pub struct CorbBuffer {
    pub buffer: Box<AlignedRingBuffer<u32>>,
    pub wp: usize,
}

pub struct CmdResponsePairBuffer {
    pub awaiting_req: ArrayQueue<u32>,
    pub ready_resp: ArrayQueue<RirbResponseEntry>,
}

pub struct CorbRespStream {
    _private: (),
}

impl CorbRespStream {
    pub fn new() -> Self {
        CorbRespStream { _private: () }
    }
}

impl Stream for CorbRespStream {
    type Item = RirbResponseEntry;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let queue = HDA_CMD_RESP_QUEUE.try_get().expect("not initialized");
        // fast path
        if let Some(resp) = queue.ready_resp.pop() {
            return Poll::Ready(Some(resp));
        }

        RESP_WAKER.register(&cx.waker());
        match queue.ready_resp.pop() {
            Some(resp) => {
                RESP_WAKER.take();
                Poll::Ready(Some(resp))
            }
            None => Poll::Pending,
        }
    }
}

impl CorbBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Box::new(AlignedRingBuffer {
                buffer: [0u32; 256],
            }),
            wp: 0,
        }
    }

    pub fn push(&mut self, val: u32) {
        self.wp = (self.wp + 1) % 256;
        unsafe {
            core::ptr::write_volatile(&mut self.buffer.buffer[self.wp], val);
            core::arch::x86_64::_mm_clflush(&self.buffer.buffer[self.wp] as *const _ as *const u8);
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RirbResponseEntry {
    pub raw_response: u32,
    // Bits 35–32 (Codec ID / CID): The hardware index of the codec that sent the message (e.g., 0000 means Codec 0).
    // Bit 36 (Solicited vs. Unsolicited / U)
    pub metadata: u32,
}

#[repr(align(128))]
pub struct RirbBuffer {
    pub buffer: Box<AlignedRingBuffer<RirbResponseEntry>>,
    pub rp: u8,
}

impl RirbBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Box::new(AlignedRingBuffer {
                buffer: [RirbResponseEntry {
                    raw_response: 0,
                    metadata: 0,
                }; 256],
            }),
            rp: 0,
        }
    }
    pub fn pop(&mut self) -> RirbResponseEntry {
        self.rp = self.rp.wrapping_add(1);
        let ptr = &self.buffer.buffer[self.rp as usize];
        let val = unsafe {
            _mm_clflush(ptr as *const _ as *const u8);
            core::ptr::read_volatile(ptr)
        };
        val
    }
}
