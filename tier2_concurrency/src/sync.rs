//! Standard vs. Loom Portability Layer.
//!
//! Under `cfg(not(loom))`, this compiles down to a zero-cost re-export
//! of `core::sync::atomic` and `core::cell::UnsafeCell`.
//!
//! Under `cfg(loom)`, it swaps in Loom's instrumented atomics for
//! exhaustive state-space model checking.

#[cfg(not(loom))]
pub mod atomic {
    pub use core::sync::atomic::{
        compiler_fence, fence, AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering,
    };
}

#[cfg(not(loom))]
pub mod cell {
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct UnsafeCell<T>(core::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        #[inline(always)]
        pub const fn new(data: T) -> Self {
            Self(core::cell::UnsafeCell::new(data))
        }

        #[inline(always)]
        pub fn get(&self) -> *mut T {
            self.0.get()
        }

        /// # Safety
        /// Caller must uphold aliasing invariants (no concurrent access).
        #[inline(always)]
        pub unsafe fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.get())
        }

        /// # Safety
        /// Caller must uphold aliasing invariants (exclusive access).
        #[inline(always)]
        pub unsafe fn with_mut<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.get())
        }
    }
}

#[cfg(loom)]
pub mod atomic {
    pub use loom::sync::atomic::{
        fence, AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering,
    };
    // Re-export standard compiler_fence for Loom compiles since Loom doesn't track compile-only fences.
    pub use core::sync::atomic::compiler_fence;
}

#[cfg(loom)]
pub mod cell {
    pub use loom::cell::UnsafeCell;
}
