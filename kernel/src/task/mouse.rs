use crate::driver::mouse::Ps2Flags;
use crate::fs::vfs::FileOps;
use crate::{
    driver::mouse::GenericPs2Packet,
    fs::fat::BlockDevice,
    println, serial_println,
    task::{
        self,
        proc::thread_for_proc,
        thread::{block_task, BlockReason, ThreadState, SCHEDULER},
    },
    PROC, SCREEN,
};
use alloc::{sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use core::{
    pin::Pin,
    task::{Context, Poll},
};
use crossbeam_queue::ArrayQueue;
use embedded_graphics::{
    pixelcolor::{BinaryColor, Rgb565},
    prelude::{DrawTarget, Primitive, RgbColor, Transform},
    primitives::{Circle, PrimitiveStyle},
    Drawable,
};
use futures_util::stream::Stream;
use futures_util::stream::StreamExt;
use futures_util::task::AtomicWaker;

static WAKER: AtomicWaker = AtomicWaker::new();
static PACKET_QUEUE: OnceCell<ArrayQueue<GenericPs2Packet>> = OnceCell::uninit();

pub fn init_packet_queue() {
    PACKET_QUEUE
        .try_init_once(|| ArrayQueue::new(100))
        .expect("Packet queue already initialized");
}

pub struct MouseDeviceHandle {}
impl FileOps for MouseDeviceHandle {
    fn read(&self, _: &crate::fs::vfs::File, buffer: &mut [u8]) -> usize {
        let queue = PACKET_QUEUE.try_get().expect("not initialized");
        let mut bytes_written = 0;

        loop {
            if buffer.len() - bytes_written < 3 {
                if bytes_written > 0 {
                    return bytes_written;
                }
                return 0; // buffer too small
            }

            if let Some(packet) = queue.pop() {
                buffer[bytes_written] = packet.status.bits();
                buffer[bytes_written + 1] = packet.x_mov;
                buffer[bytes_written + 2] = packet.y_mov;
                bytes_written += 3;
            } else {
                if bytes_written > 0 {
                    return bytes_written;
                }
                block_task(BlockReason::WaitMouse(0));
            }
        }
    }
    fn write(&self, _: &crate::fs::vfs::File, _buffer: &[u8]) -> usize {
        return 0;
    }
}

/// Called by the mouse interrupt handler
///
/// Must not block or allocate.
pub(crate) fn add_packet(packet: GenericPs2Packet) {
    if let Ok(queue) = PACKET_QUEUE.try_get() {
        queue.force_push(packet);
        WAKER.wake();
        wake_mouse_sleepers(0, Direction::Read);
    } else {
        serial_println!("WARNING: packet queue uninitialized");
    }
}

pub struct Ps2PacketStream {
    _private: (),
}

impl Ps2PacketStream {
    pub fn new() -> Self {
        PACKET_QUEUE
            .try_init_once(|| ArrayQueue::new(100))
            .expect("Ps2PacketStream::new should only be called once");
        Ps2PacketStream { _private: () }
    }
}

impl Stream for Ps2PacketStream {
    type Item = GenericPs2Packet;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<GenericPs2Packet>> {
        let queue = PACKET_QUEUE.try_get().expect("not initialized");
        // fast path
        if let Some(packet) = queue.pop() {
            return Poll::Ready(Some(packet));
        }

        WAKER.register(&cx.waker());
        match queue.pop() {
            Some(packet) => {
                WAKER.take();
                Poll::Ready(Some(packet))
            }
            None => Poll::Pending,
        }
    }
}

/*pub async fn print_mouse_movement() {
    use embedded_graphics::prelude::Point;

    let mut packets = Ps2PacketStream::new();
    let mut x = 0 as i32;
    let mut y = 0 as i32;
    while let Some(packet) = packets.next().await {
        // draw cursor
        x += packet.get_x() as i32;
        y -= packet.get_y() as i32;

    {

        let screen = SCREEN.get().unwrap().lock();
        x = x.clamp(0, screen.width as i32);
        y = y.clamp(0, screen.height as i32);
        }

        let point = Point::new(x as i32, y as i32);
        serial_println!("{:#?}", point);

        // screen.clear(Rgb565::BLACK).unwrap();
        /* Circle::new(point, 15)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::RED))
        .draw(&mut *screen)
        .unwrap(); */
    }
}*/

enum Direction {
    Read,
    Write,
}

fn wake_mouse_sleepers(mouse_id: u8, dir: Direction) {
    let mut sched = SCHEDULER.lock();
    let mut to_wake = alloc::vec::Vec::new();
    for thread_arc in sched.threads.iter() {
        let thread = thread_arc.lock();
        match (&dir, &thread.state) {
            (Direction::Read, ThreadState::Blocked(BlockReason::WaitMouse(waiting_on))) => {
                if *waiting_on == mouse_id {
                    to_wake.push(thread.id);
                }
            }
            _ => {}
        };
    }
    for thread_id in to_wake {
        sched.unblock_task(thread_id);
    }
}
