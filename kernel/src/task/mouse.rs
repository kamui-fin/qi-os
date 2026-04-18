use crate::{mouse::GenericPs2Packet, println, serial_println, BOOT_INFO};
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

/// Called by the mouse interrupt handler
///
/// Must not block or allocate.
pub(crate) fn add_packet(packet: GenericPs2Packet) {
    if let Ok(queue) = PACKET_QUEUE.try_get() {
        queue.force_push(packet);
        WAKER.wake();
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

pub async fn print_mouse_movement() {
    use embedded_graphics::prelude::Point;

    let mut packets = Ps2PacketStream::new();
    let mut x = 0;
    let mut y = 0;
    while let Some(packet) = packets.next().await {
        let mut boot_info = BOOT_INFO.get().unwrap().lock();
        let mut screen = boot_info.screen;

        // draw cursor
        x += packet.get_x();
        y -= packet.get_y();

        // make sure to check out of bounds and stuff

        let point = Point::new(x as i32, y as i32);

        // screen.clear(Rgb565::BLACK).unwrap();
        Circle::new(point, 15)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::RED))
            .draw(&mut screen)
            .unwrap();
    }
}
