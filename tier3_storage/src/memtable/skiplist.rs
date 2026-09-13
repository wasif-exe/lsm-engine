use std::cell::Cell;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

const MAX_HEIGHT: usize = 20;
const BRANCH_FACTOR: u32 = 4;

thread_local! {
    static RNG_STATE: Cell<u32> = const { Cell::new(0xDEADBEEF) };
}

#[inline]
fn fast_rand_u32() -> u32 {
    RNG_STATE.with(|cell| {
        let mut x = cell.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        cell.set(x);
        x
    })
}

struct Node {
    key: Vec<u8>,
    val: Option<Vec<u8>>,
    seq: u64,
    _height: usize,
    next: [AtomicPtr<Node>; MAX_HEIGHT],
}

impl Node {
    fn new(key: Vec<u8>, val: Option<Vec<u8>>, seq: u64, height: usize) -> *mut Self {
        let mut next: [AtomicPtr<Node>; MAX_HEIGHT] = Default::default();
        for item in next.iter_mut() {
            *item = AtomicPtr::new(ptr::null_mut());
        }
        Box::into_raw(Box::new(Self {
            key,
            val,
            seq,
            _height: height,
            next,
        }))
    }
}

pub struct ConcurrentSkipList {
    head: *mut Node,
    max_height: AtomicUsize,
    approximate_size: AtomicUsize,
}

unsafe impl Send for ConcurrentSkipList {}
unsafe impl Sync for ConcurrentSkipList {}

impl ConcurrentSkipList {
    pub fn new() -> Self {
        let head = Node::new(vec![], None, 0, MAX_HEIGHT);
        Self {
            head,
            max_height: AtomicUsize::new(1),
            approximate_size: AtomicUsize::new(0),
        }
    }

    fn random_height(&self) -> usize {
        let mut height = 1;
        while height < MAX_HEIGHT && (fast_rand_u32() % BRANCH_FACTOR == 0) {
            height += 1;
        }
        height
    }

    pub fn insert(&self, key: Vec<u8>, val: Option<Vec<u8>>, seq: u64) {
        let k_len = key.len();
        let v_len = val.as_ref().map_or(0, |v| v.len());
        let height = self.random_height();

        let curr_max = self.max_height.load(Ordering::Relaxed);
        if height > curr_max {
            self.max_height
                .compare_exchange_weak(curr_max, height, Ordering::Relaxed, Ordering::Relaxed)
                .ok();
        }

        let node_ptr = Node::new(key, val, seq, height);
        let node = unsafe { &*node_ptr };


        loop {
            let mut preds = [ptr::null_mut(); MAX_HEIGHT];
            let mut succs = [ptr::null_mut(); MAX_HEIGHT];
            self.find_position(&node.key, &mut preds, &mut succs);

            node.next[0].store(succs[0], Ordering::Relaxed);

            let pred = if preds[0].is_null() { unsafe { &*self.head } } else { unsafe { &*preds[0] } };
            
            if pred.next[0]
                .compare_exchange(succs[0], node_ptr, Ordering::Release, Ordering::Acquire)
                .is_ok()
            {

                break;
            }

        }


        for level in 1..height {
            loop {
                let mut preds = [ptr::null_mut(); MAX_HEIGHT];
                let mut succs = [ptr::null_mut(); MAX_HEIGHT];
                self.find_position(&node.key, &mut preds, &mut succs);

                node.next[level].store(succs[level], Ordering::Relaxed);
                
                let pred = if preds[level].is_null() { unsafe { &*self.head } } else { unsafe { &*preds[level] } };
                
                if pred.next[level]
                    .compare_exchange(succs[level], node_ptr, Ordering::Release, Ordering::Acquire)
                    .is_ok()
                {

                    break;
                }

            }
        }

        self.approximate_size
            .fetch_add(k_len + v_len + 24, Ordering::Relaxed);
    }

    pub fn get(&self, key: &[u8]) -> Option<(Option<Vec<u8>>, u64)> {
        let mut curr = self.head;
        let mut next_ptr;
        let mut level = self.max_height.load(Ordering::Acquire);

        loop {

            let curr_node = unsafe { &*curr };
            next_ptr = curr_node.next[level - 1].load(Ordering::Acquire);

            if !next_ptr.is_null() {
                let next_node = unsafe { &*next_ptr };
                if next_node.key.as_slice() < key {
                    curr = next_ptr;
                    continue;
                } else if next_node.key.as_slice() == key {
                    return Some((next_node.val.clone(), next_node.seq));
                }
            }

            if level > 1 {
                level -= 1;
            } else {
                break;
            }
        }
        None
    }

    fn find_position(
        &self,
        key: &[u8],
        preds: &mut [*mut Node; MAX_HEIGHT],
        succs: &mut [*mut Node; MAX_HEIGHT],
    ) {
        let mut curr = self.head;
        let mut level = MAX_HEIGHT;

        while level > 0 {
            let curr_node = unsafe { &*curr };
            let mut next = curr_node.next[level - 1].load(Ordering::Acquire);

            while !next.is_null() {
                let next_node = unsafe { &*next };

                if next_node.key.as_slice() < key {
                    curr = next;
                    next = next_node.next[level - 1].load(Ordering::Acquire);
                } else {
                    break;
                }
            }

            preds[level - 1] = curr;
            succs[level - 1] = next;
            level -= 1;
        }
    }

    pub fn approximate_size(&self) -> usize {
        self.approximate_size.load(Ordering::Relaxed)
    }

    pub fn iter(&self) -> SkipListIterator {

        let first = unsafe { (*self.head).next[0].load(Ordering::Acquire) };
        SkipListIterator { curr: first }
    }
}

impl Drop for ConcurrentSkipList {
    fn drop(&mut self) {
        let mut curr = self.head;
        while !curr.is_null() {
            let next = unsafe { (*curr).next[0].load(Ordering::Relaxed) };
            unsafe {
                let _ = Box::from_raw(curr);
            }
            curr = next;
        }
    }
}

pub struct SkipListIterator {
    curr: *mut Node,
}

impl Iterator for SkipListIterator {
    type Item = (Vec<u8>, Option<Vec<u8>>, u64);

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr.is_null() {
            return None;
        }
        let node = unsafe { &*self.curr };
        let item = (node.key.clone(), node.val.clone(), node.seq);
        self.curr = node.next[0].load(Ordering::Acquire);
        Some(item)
    }
}