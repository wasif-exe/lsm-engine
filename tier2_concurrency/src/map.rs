use crate::ebr::{Collector, ThreadLocalLocal};
use crate::sync::atomic::{AtomicPtr, Ordering};
use core::hash::{Hash, Hasher};
use core::mem::ManuallyDrop;

const TOMBSTONE: *mut u8 = 1 as *mut u8;

struct Entry<K, V> {
    key: K,
    value: ManuallyDrop<V>,
}

pub struct ConcurrentHashMap<K, V> {
    capacity: usize,
    mask: usize,
    table: Box<[AtomicPtr<Entry<K, V>>]>,
    collector: Collector,
}

unsafe impl<K: Send + Sync, V: Send + Sync> Send for ConcurrentHashMap<K, V> {}
unsafe impl<K: Send + Sync, V: Send + Sync> Sync for ConcurrentHashMap<K, V> {}

impl<K: Eq + Hash, V> ConcurrentHashMap<K, V> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two());
        let mut table = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            table.push(AtomicPtr::new(core::ptr::null_mut()));
        }
        Self {
            capacity,
            mask: capacity - 1,
            table: table.into_boxed_slice(),
            collector: Collector::new(),
        }
    }

    pub fn collector(&self) -> &Collector {
        &self.collector
    }

    #[inline]
    fn hash(&self, key: &K) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) & self.mask
    }

    pub fn get<R, F>(&self, key: &K, local: &ThreadLocalLocal, f: F) -> Option<R>
    where
        F: FnOnce(&V) -> R,
    {
        let _guard = self.collector.pin(local.thread_index());
        let mut idx = self.hash(key);

        for _ in 0..self.capacity {
            let ptr = self.table[idx].load(Ordering::Acquire);
            if ptr.is_null() {
                return None;
            }
            if ptr as *mut u8 == TOMBSTONE {
                idx = (idx + 1) & self.mask;
                continue;
            }

            // SAFETY: EBR guarantees this pointer remains valid inside the guard.
            let entry = unsafe { &*ptr };
            if entry.key == *key {
                return Some(f(&entry.value));
            }
            idx = (idx + 1) & self.mask;
        }
        None
    }

    pub fn insert(&self, key: K, value: V, local: &mut ThreadLocalLocal) -> bool {
        let _guard = self.collector.pin(local.thread_index());
        let mut idx = self.hash(&key);
        let new_entry = Box::into_raw(Box::new(Entry {
            key,
            value: ManuallyDrop::new(value),
        }));

        for _ in 0..self.capacity {
            let slot = &self.table[idx];
            let curr = slot.load(Ordering::Acquire);

            if curr.is_null() || curr as *mut u8 == TOMBSTONE {
                match slot.compare_exchange(
                    curr,
                    new_entry,
                    Ordering::Release,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return true,
                    Err(_) => {
                        continue;
                    }
                }
            }

            // SAFETY: EBR guarantees safety of this pointer.
            let entry = unsafe { &*curr };
            if &entry.key == unsafe { &(*new_entry).key } {
                match slot.compare_exchange(
                    curr,
                    new_entry,
                    Ordering::Release,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        unsafe {
                            local.defer(&self.collector, curr, drop_entry_node);
                        }
                        return true;
                    }
                    Err(_) => {
                        continue;
                    }
                }
            }

            idx = (idx + 1) & self.mask;
        }

        unsafe {
            let boxed = Box::from_raw(new_entry);
            drop(ManuallyDrop::into_inner(boxed.value));
        }
        false
    }

    pub fn remove(&self, key: &K, local: &mut ThreadLocalLocal) -> Option<V> {
        let _guard = self.collector.pin(local.thread_index());
        let mut idx = self.hash(key);

        for _ in 0..self.capacity {
            let slot = &self.table[idx];
            let curr = slot.load(Ordering::Acquire);

            if curr.is_null() {
                return None;
            }
            if curr as *mut u8 == TOMBSTONE {
                idx = (idx + 1) & self.mask;
                continue;
            }

            // SAFETY: EBR guarantees this pointer remains valid inside the guard.
            let entry = unsafe { &*curr };
            if entry.key == *key {
                match slot.compare_exchange(
                    curr,
                    TOMBSTONE as *mut Entry<K, V>,
                    Ordering::Release,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        let val = unsafe { core::ptr::read(&*entry.value) };
                        unsafe {
                            local.defer(&self.collector, curr, free_entry_shell);
                        }
                        return Some(val);
                    }
                    Err(_) => {
                        continue;
                    }
                }
            }
            idx = (idx + 1) & self.mask;
        }
        None
    }
}

impl<K, V> Drop for ConcurrentHashMap<K, V> {
    fn drop(&mut self) {
        for slot in self.table.iter() {
            let ptr = slot.load(Ordering::Relaxed);
            if !ptr.is_null() && ptr as *mut u8 != TOMBSTONE {
                unsafe {
                    let mut boxed = Box::from_raw(ptr);
                    ManuallyDrop::drop(&mut boxed.value);
                }
            }
        }
    }
}

unsafe fn drop_entry_node<K, V>(ptr: *mut Entry<K, V>) {
    let mut boxed = Box::from_raw(ptr);
    ManuallyDrop::drop(&mut boxed.value);
}

unsafe fn free_entry_shell<K, V>(ptr: *mut Entry<K, V>) {
    let boxed = Box::from_raw(ptr);
    drop(boxed.key);
}
