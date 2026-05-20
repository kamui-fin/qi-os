// non-cryptographically secure random number generator
// using xorshift128++
// TODO: entropy pool

use core::arch::asm;

use conquer_once::spin::OnceCell;
use crate::spinlock::Spinlock;
use x86_64::instructions::{interrupts, random::RdRand};

pub struct EntropyPool(u64);

impl EntropyPool {
    fn mix_entropy(&mut self, x: u64) {
        self.0 ^= x;
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
    }
}

pub fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "rdtsc",
                out("eax") low,
                out("edx") high,

        );
    }
    ((high as u64) << 32) | (low as u64)
}

#[derive(Copy, Clone, Debug)]
pub struct RdSeed(());
impl RdSeed {
    // creates Some(RdSeed) if RDSEED is supported, None otherwise
    pub fn new() -> Option<Self> {
        let cpuid = core::arch::x86_64::__cpuid(0x7);
        if (cpuid.ebx >> 18) & 1 != 0 {
            Some(RdSeed(()))
        } else {
            None
        }
    }

    // may fail in rare circumstances or heavy load.
    pub fn get_u64(self) -> Option<u64> {
        let mut res: u64 = 0;
        unsafe {
            match core::arch::x86_64::_rdseed64_step(&mut res) {
                1 => Some(res),
                _ => None,
            }
        }
    }
}

pub struct RandomGenerator {
    s0: u64,
    s1: u64,
}

impl RandomGenerator {
    pub fn seed(pool: EntropyPool) -> Self {
        Self {
            s0: pool.0,
            s1: pool.0 ^ 0x9e3779b97f4a7c15,
        }
    }

    pub fn reseed(&mut self, pool: u64) {
        self.s0 ^= pool;
        self.s1 ^= pool << 1;
    }

    pub fn rng_next(&mut self) -> u64 {
        let mut s0 = self.s0;
        let s1 = self.s1;

        self.s0 = s1;
        s0 ^= s0 << 23;
        s0 ^= s0 >> 18;
        s0 ^= s1;
        s0 ^= s1 >> 5;
        self.s1 = s0;

        s0.wrapping_add(s1)
    }
}

pub static RAND_GEN: OnceCell<Spinlock<RandomGenerator>> = OnceCell::uninit();

pub fn init_rand() {
    let hw_seeder = RdSeed::new();

    let mut pool = EntropyPool(read_tsc());
    pool.mix_entropy(read_tsc());
    pool.mix_entropy(read_tsc());

    if let Some(hw_seeder) = hw_seeder {
        if let Some(seed) = hw_seeder.get_u64() {
            pool.mix_entropy(seed);
        }
    }

    RAND_GEN.init_once(|| Spinlock::new(RandomGenerator::seed(pool)))
}

pub fn mix_entropy() {
    let rand_gen = RAND_GEN.get();
    if let Some(rand_gen) = rand_gen {
        rand_gen.lock().reseed(read_tsc());
    }
}

pub fn mix_entropy_with(val: u64) {
    let rand_gen = RAND_GEN.get();
    if let Some(rand_gen) = rand_gen {
        rand_gen.lock().reseed(val);
    }
}

pub fn get_random_number() -> u64 {
    let Spinlock = RAND_GEN.get().unwrap();
    // avoid deadlocks in interrupt bc of mix_entropy
    interrupts::without_interrupts(|| {
        let mut rand_gen = Spinlock.lock();
        rand_gen.rng_next()
    })
}

// such that min <= x <= max
pub fn get_rand_range(min: u64, max: u64) -> u64 {
    min + (get_random_number() % (max - min + 1))
}
