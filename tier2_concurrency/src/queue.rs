use crate::sync::atomic::{AtomicUsize, Ordering};
use crate::sync::cell::UnsafeCell;
use core::mem::MaybeUninit;

struct Slot<T> {
    sequence: AtomicUsize,
    value: UnsafeCell<MaybeUninit<T>>,
}

#[repr(align(64))]
struct CacheAlignedAtomic(AtomicUsize);

#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    Full,
    Empty,
}

pub struct MPMCQueue<T> {
    buffer_mask: usize,
    buffer: Box<[Slot<T>]>,
    tail: CacheAlignedAtomic,
    head: CacheAlignedAtomic,
}

unsafe impl<T: Send> Send for MPMCQueue<T> {}
unsafe impl<T: Send> Sync for MPMCQueue<T> {}

impl<T> MPMCQueue<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= 2 && capacity.is_power_of_two());
        let mut buffer = Vec::with_capacity(capacity);
        for i in 0..capacity {
            buffer.push(Slot {
                sequence: AtomicUsize::new(i),
                value: UnsafeCell::new(MaybeUninit::uninit()),
            });
        }
        Self {
            buffer_mask: capacity - 1,
            buffer: buffer.into_boxed_slice(),
            tail: CacheAlignedAtomic(AtomicUsize::new(0)),
            head: CacheAlignedAtomic(AtomicUsize::new(0)),
        }
    }

    pub fn push(&self, data: T) -> Result<(), (QueueError, T)> {
        let mut tail = self.tail.0.load(Ordering::Relaxed);
        loop {
            let slot_idx = tail & self.buffer_mask;
            let slot = &self.buffer[slot_idx];
            let seq = slot.sequence.load(Ordering::Acquire);
            let diff = seq as isize - tail as isize;

            if diff == 0 {
                match self.tail.0.compare_exchange_weak(
                    tail,
                    tail.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        unsafe {
                            slot.value.with_mut(|ptr| {
                                core::ptr::write(ptr, MaybeUninit::new(data));
                            });
                        }
                        slot.sequence.store(tail.wrapping_add(1), Ordering::Release);
                        return Ok(());
                    }
                    Err(actual) => {
                        tail = actual;
                    }
                }
            } else if diff < 0 {
                return Err((QueueError::Full, data));
            } else {
                tail = self.tail.0.load(Ordering::Relaxed);
            }
        }
    }

    pub fn pop(&self) -> Result<T, QueueError> {
        let mut head = self.head.0.load(Ordering::Relaxed);
        loop {
            let slot_idx = head & self.buffer_mask;
            let slot = &self.buffer[slot_idx];
            let seq = slot.sequence.load(Ordering::Acquire);
            let diff = seq as isize - head.wrapping_add(1) as isize;

            if diff == 0 {
                match self.head.0.compare_exchange_weak(
                    head,
                    head.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let data = unsafe {
                            slot.value.with(|ptr| core::ptr::read(ptr).assume_init())
                        };
                        slot.sequence.store(
                            head.wrapping_add(self.buffer_mask.wrapping_add(1)),
                            Ordering::Release,
                        );
                        return Ok(data);
                    }
                    Err(actual) => {
                        head = actual;
                    }
                }
            } else if diff < 0 {
                return Err(QueueError::Empty);
            } else {
                head = self.head.0.load(Ordering::Relaxed);
            }
        }
    }
}

impl<T> Drop for MPMCQueue<T> {
    fn drop(&mut self) {
        while self.pop().is_ok() {}
    }
}

pub struct UnpaddedMPMCQueue<T> {
    buffer_mask: usize,
    buffer: Box<[Slot<T>]>,
    tail: AtomicUsize,
    head: AtomicUsize,
}

unsafe impl<T: Send> Send for UnpaddedMPMCQueue<T> {}
unsafe impl<T: Send> Sync for UnpaddedMPMCQueue<T> {}

impl<T> UnpaddedMPMCQueue<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= 2 && capacity.is_power_of_two());
        let mut buffer = Vec::with_capacity(capacity);
        for i in 0..capacity {
            buffer.push(Slot {
                sequence: AtomicUsize::new(i),
                value: UnsafeCell::new(MaybeUninit::uninit()),
            });
        }
        Self {
            buffer_mask: capacity - 1,
            buffer: buffer.into_boxed_slice(),
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
        }
    }

    pub fn push(&self, data: T) -> Result<(), (QueueError, T)> {
        let mut tail = self.tail.load(Ordering::Relaxed);
        loop {
            let slot_idx = tail & self.buffer_mask;
            let slot = &self.buffer[slot_idx];
            let seq = slot.sequence.load(Ordering::Acquire);
            let diff = seq as isize - tail as isize;

            if diff == 0 {
                match self.tail.compare_exchange_weak(
                    tail,
                    tail.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        unsafe {
                            slot.value.with_mut(|ptr| {
                                core::ptr::write(ptr, MaybeUninit::new(data));
                            });
                        }
                        slot.sequence.store(tail.wrapping_add(1), Ordering::Release);
                        return Ok(());
                    }
                    Err(actual) => {
                        tail = actual;
                    }
                }
            } else if diff < 0 {
                return Err((QueueError::Full, data));
            } else {
                tail = self.tail.load(Ordering::Relaxed);
            }
        }
    }

    pub fn pop(&self) -> Result<T, QueueError> {
        let mut head = self.head.load(Ordering::Relaxed);
        loop {
            let slot_idx = head & self.buffer_mask;
            let slot = &self.buffer[slot_idx];
            let seq = slot.sequence.load(Ordering::Acquire);
            let diff = seq as isize - head.wrapping_add(1) as isize;

            if diff == 0 {
                match self.head.compare_exchange_weak(
                    head,
                    head.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let data = unsafe {
                            slot.value.with(|ptr| core::ptr::read(ptr).assume_init())
                        };
                        slot.sequence.store(
                            head.wrapping_add(self.buffer_mask.wrapping_add(1)),
                            Ordering::Release,
                        );
                        return Ok(data);
                    }
                    Err(actual) => {
                        head = actual;
                    }
                }
            } else if diff < 0 {
                return Err(QueueError::Empty);
            } else {
                head = self.head.load(Ordering::Relaxed);
            }
        }
    }
}

impl<T> Drop for UnpaddedMPMCQueue<T> {
    fn drop(&mut self) {
        while self.pop().is_ok() {}
    }
}