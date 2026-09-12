use crate::sync::atomic::{AtomicUsize, Ordering, fence};
use core::mem::MaybeUninit;
use core::ptr;

const UNPINNED: usize = usize::MAX;

#[cfg(not(loom))]
pub const MAX_THREADS: usize = 128;
#[cfg(loom)]
pub const MAX_THREADS: usize = 4;

#[cfg(not(loom))]
const BAG_CAPACITY: usize = 64;
#[cfg(loom)]
const BAG_CAPACITY: usize = 4;

struct Deferred {
    ptr: *mut u8,
    dtor: unsafe fn(*mut u8),
}

struct Bag {
    items: [MaybeUninit<Deferred>; BAG_CAPACITY],
    len: usize,
}

impl Bag {
    fn new() -> Self {
        Self {
            items: unsafe { MaybeUninit::uninit().assume_init() },
            len: 0,
        }
    }

    #[inline]
    fn is_full(&self) -> bool {
        self.len == BAG_CAPACITY
    }

    #[inline]
    fn push(&mut self, ptr: *mut u8, dtor: unsafe fn(*mut u8)) -> Result<(), ()> {
        if self.is_full() {
            return Err(());
        }
        self.items[self.len] = MaybeUninit::new(Deferred { ptr, dtor });
        self.len += 1;
        Ok(())
    }

    unsafe fn reclaim_all(&mut self) {
        for i in 0..self.len {
            let deferred = self.items[i].as_ptr().read();
            (deferred.dtor)(deferred.ptr);
        }
        self.len = 0;
    }
}

impl Drop for Bag {
    fn drop(&mut self) {
        unsafe { self.reclaim_all(); }
    }
}

#[repr(align(64))]
struct ThreadState {
    epoch: AtomicUsize,
    active: AtomicUsize,
}

pub struct Collector {
    global_epoch: AtomicUsize,
    registry: [ThreadState; MAX_THREADS],
}

unsafe impl Send for Collector {}
unsafe impl Sync for Collector {}

impl Collector {
    pub fn new() -> Self {
        let registry: [ThreadState; MAX_THREADS] = unsafe {
            let mut uninit: MaybeUninit<[ThreadState; MAX_THREADS]> = MaybeUninit::uninit();
            let ptr = uninit.as_mut_ptr() as *mut ThreadState;
            for i in 0..MAX_THREADS {
                let slot = ptr.add(i);
                ptr::write(&mut (*slot).epoch, AtomicUsize::new(UNPINNED));
                ptr::write(&mut (*slot).active, AtomicUsize::new(0));
            }
            uninit.assume_init()
        };
        Self {
            global_epoch: AtomicUsize::new(0),
            registry,
        }
    }

    pub fn register(&self) -> usize {
        for i in 0..MAX_THREADS {
            if self.registry[i].active.compare_exchange(
                0, 1, Ordering::Relaxed, Ordering::Relaxed,
            ).is_ok() {
                return i;
            }
        }
        panic!("EBR Registry full! Increase MAX_THREADS.");
    }

    pub fn unregister(&self, index: usize) {
        assert!(index < MAX_THREADS);
        self.registry[index].epoch.store(UNPINNED, Ordering::Release);
        self.registry[index].active.store(0, Ordering::Release);
    }

    #[inline]
    pub fn pin<'a>(&'a self, thread_index: usize) -> Guard<'a> {
        let state = &self.registry[thread_index];
        let global = self.global_epoch.load(Ordering::Relaxed);
        state.epoch.store(global, Ordering::Release);
        // Full fence prevents reads inside the critical section from being
        // reordered before the pin store, preventing observation of stale data.
        fence(Ordering::SeqCst);
        Guard { collector: self, thread_index }
    }

    /// Advances the global epoch from E to (E+1) mod 3.
    /// SAFETY INVARIANT: All active threads MUST be pinned to the current global
    /// epoch E (or UNPINNED). Any thread pinned to E-1 blocks advancement,
    /// because reclaiming (E+1) mod 3 would free memory those threads may see.
    pub fn try_advance(&self) -> bool {
        let current_global = self.global_epoch.load(Ordering::Acquire);

        for i in 0..MAX_THREADS {
            let thread_active = self.registry[i].active.load(Ordering::Relaxed) != 0;
            if thread_active {
                let thread_epoch = self.registry[i].epoch.load(Ordering::Acquire);
                if thread_epoch != UNPINNED && thread_epoch != current_global {
                    return false;
                }
            }
        }

        let next_epoch = (current_global + 1) % 3;
        self.global_epoch.store(next_epoch, Ordering::Release);
        true
    }
}

pub struct Guard<'a> {
    collector: &'a Collector,
    thread_index: usize,
}

impl<'a> Guard<'a> {
    #[inline]
    pub fn collector(&self) -> &'a Collector {
        self.collector
    }
}

impl<'a> Drop for Guard<'a> {
    #[inline]
    fn drop(&mut self) {
        self.collector.registry[self.thread_index]
            .epoch
            .store(UNPINNED, Ordering::Release);
    }
}

pub struct ThreadLocalLocal {
    thread_index: usize,
    bags: [Bag; 3],
}

impl ThreadLocalLocal {
    pub fn new(collector: &Collector) -> Self {
        let idx = collector.register();
        Self {
            thread_index: idx,
            bags: [Bag::new(), Bag::new(), Bag::new()],
        }
    }

    pub fn destroy(mut self, collector: &Collector) {
        for bag in &mut self.bags {
            unsafe { bag.reclaim_all(); }
        }
        collector.unregister(self.thread_index);
        core::mem::forget(self);
    }

    #[inline]
    pub fn thread_index(&self) -> usize {
        self.thread_index
    }

    pub unsafe fn defer<T>(&mut self, collector: &Collector, ptr: *mut T, dtor: unsafe fn(*mut T)) {
        let global_epoch = collector.global_epoch.load(Ordering::Relaxed);
        let bag = &mut self.bags[global_epoch];
        let type_erased_dtor: unsafe fn(*mut u8) = core::mem::transmute(dtor);

        if bag.push(ptr as *mut u8, type_erased_dtor).is_err() {
            if collector.try_advance() {
                let new_global = collector.global_epoch.load(Ordering::Acquire);
                let reclaim_idx = (new_global + 1) % 3;
                unsafe { self.bags[reclaim_idx].reclaim_all(); }
            }
            let fresh_global = collector.global_epoch.load(Ordering::Relaxed);
            if self.bags[fresh_global].push(ptr as *mut u8, type_erased_dtor).is_err() {
                dtor(ptr);
            }
        }
    }

    pub fn flush(&mut self, collector: &Collector) {
        if collector.try_advance() {
            let new_global = collector.global_epoch.load(Ordering::Acquire);
            let reclaim_idx = (new_global + 1) % 3;
            unsafe { self.bags[reclaim_idx].reclaim_all(); }
        }
    }
}

impl Drop for ThreadLocalLocal {
    fn drop(&mut self) {
        for bag in &mut self.bags {
            unsafe { bag.reclaim_all(); }
        }
    }
}
