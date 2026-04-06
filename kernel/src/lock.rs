/*
* This module contains implementations for:
*   - SimpleIrqLock: just used for the scheduler
*   - PreemptIrqLock: same as SimpleIrqLock, except it postpones pre-empts to after the duration of
*   the lock
*
* Leveraging the Drop trait, atomics
*/

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicUsize},
};

use crate::thread::SCHEDULER;

static IRQ_DISABLE_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub static PREEMPT_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static NEEDS_RESCHEDULE: AtomicBool = AtomicBool::new(false);

pub struct SimpleIrqLock<T> {
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SimpleIrqLock<T> {}

impl<T> SimpleIrqLock<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SimpleIrqLockGuard<'_, T> {
        x86_64::instructions::interrupts::disable();
        IRQ_DISABLE_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        SimpleIrqLockGuard { lock: self }
    }
}

pub struct SimpleIrqLockGuard<'a, T> {
    lock: &'a SimpleIrqLock<T>,
}

impl<'a, T> Deref for SimpleIrqLockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SimpleIrqLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SimpleIrqLockGuard<'a, T> {
    fn drop(&mut self) {
        let prev = IRQ_DISABLE_COUNTER.fetch_sub(1, core::sync::atomic::Ordering::Release);
        if prev == 1 {
            x86_64::instructions::interrupts::enable();
        }
    }
}

pub struct PreemptIrqLock<T> {
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for PreemptIrqLock<T> {}

impl<T> PreemptIrqLock<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> PreemptIrqLockGuard<'_, T> {
        x86_64::instructions::interrupts::disable();
        IRQ_DISABLE_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        PREEMPT_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        PreemptIrqLockGuard { lock: self }
    }
}

pub struct PreemptIrqLockGuard<'a, T> {
    lock: &'a PreemptIrqLock<T>,
}

impl<'a, T> Deref for PreemptIrqLockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for PreemptIrqLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for PreemptIrqLockGuard<'a, T> {
    fn drop(&mut self) {
        let prev = PREEMPT_COUNT.fetch_sub(1, core::sync::atomic::Ordering::Release);
        if prev == 1 && NEEDS_RESCHEDULE.swap(false, core::sync::atomic::Ordering::Relaxed) {
            let mut scheduler = SCHEDULER.lock();
            scheduler.schedule();
        }

        let prev = IRQ_DISABLE_COUNTER.fetch_sub(1, core::sync::atomic::Ordering::Release);
        if prev == 1 {
            x86_64::instructions::interrupts::enable();
        }
    }
}
