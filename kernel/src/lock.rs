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

pub static IRQ_DISABLE_COUNTER: AtomicUsize = AtomicUsize::new(0);
pub static NEEDS_RESCHEDULE: AtomicBool = AtomicBool::new(false);

/*
 * We need to be aware of about state of the interrupts BEFORE lock was created. So we need to save
 */
static WAS_IRQ_ENABLED: AtomicBool = AtomicBool::new(false);

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
        let was_interrupt_enabled = x86_64::instructions::interrupts::are_enabled();

        x86_64::instructions::interrupts::disable();

        let prev = IRQ_DISABLE_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Acquire);

        if prev == 0 {
            // if first time locking
            WAS_IRQ_ENABLED.store(was_interrupt_enabled, core::sync::atomic::Ordering::SeqCst);
        }

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

        if prev == 1 && WAS_IRQ_ENABLED.load(core::sync::atomic::Ordering::SeqCst) {
            x86_64::instructions::interrupts::enable();
        }
    }
}
