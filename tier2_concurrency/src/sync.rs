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

        #[inline(always)]
        pub unsafe fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.get())
        }

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
    pub use core::sync::atomic::compiler_fence;
}

#[cfg(loom)]
pub mod cell {
    pub use loom::cell::UnsafeCell;
}
