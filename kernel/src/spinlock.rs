// spinlock impl inspired by xv6
use crate::serial_println;
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::lapic::mycpu;

pub struct Spinlock<T> {
    is_locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Spinlock<T> {}

pub fn pushcli() {
    //let caller = core::panic::Location::caller();
    //serial_println!("pushcli from {}", caller);
    let enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    let cpu = mycpu();
    if cpu.irq_disable_depth.load(Ordering::SeqCst) == 0 {
        cpu.int_enabled.store(
            enabled,
            Ordering::SeqCst,
        );
    }
    cpu.irq_disable_depth.fetch_add(1, Ordering::SeqCst);
}

pub fn popcli() {
    //let caller = core::panic::Location::caller();
    //serial_println!("popcli from {}", caller);
    let cpu = mycpu();
    if cpu.irq_disable_depth.load(Ordering::SeqCst) == 0 {
        panic!("cannot popcli; depth = 0");
    }
    cpu.irq_disable_depth.fetch_sub(1, Ordering::SeqCst);
    if cpu.irq_disable_depth.load(Ordering::SeqCst) == 0 && cpu.int_enabled.load(Ordering::SeqCst) {
        x86_64::instructions::interrupts::enable();
    }
}

impl<T> Spinlock<T> {
    pub fn new(data: T) -> Self {
        Self {
            is_locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        pushcli();

        while self.is_locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }

        SpinlockGuard { lock: self }
    }

    pub fn force_release(&self) {
        self.is_locked.store(false, Ordering::Release);

        popcli();
    }
}

pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}

impl<'a, T> SpinlockGuard<'a, T> {
    pub fn into_raw(self) {
        core::mem::forget(self);
    }
}

impl<'a, T> Deref for SpinlockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.force_release()
    }
}
