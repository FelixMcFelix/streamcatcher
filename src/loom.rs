pub(crate) mod cell {
	#[cfg(loom)]
	pub(crate) use loom::cell::UnsafeCell;

	#[cfg(not(loom))]
	#[derive(Debug)]
	pub(crate) struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

	#[cfg(not(loom))]
	impl<T> UnsafeCell<T> {
	    pub(crate) fn new(data: T) -> UnsafeCell<T> {
	        UnsafeCell(std::cell::UnsafeCell::new(data))
	    }

	    pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
	        f(self.0.get())
	    }

	    pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
	        f(self.0.get())
	    }
	}
}

pub(crate) mod sync {
	#[cfg(loom)]
	pub(crate) use loom::sync::{
		atomic::{AtomicU8, AtomicUsize, Ordering},
		Arc,
	};

	#[cfg(not(loom))]
	pub(crate) use std::sync::{
		atomic::{AtomicU8, AtomicUsize, Ordering},
		Arc,
	};

	#[cfg(not(loom))]
	pub(crate) use parking_lot::Mutex;

	#[cfg(loom)]
	#[derive(Debug)]
	pub(crate) struct Mutex<T>(loom::sync::Mutex<T>);

	#[cfg(loom)]
	impl<T> Mutex<T> {
	    pub(crate) fn new(data: T) -> Self {
	        Self(loom::sync::Mutex::new(data))
	    }

	    pub(crate) fn lock(&self) -> loom::sync::MutexGuard<T> {
	        self.0.lock().unwrap()
	    }

	    pub(crate) fn try_lock(&self) -> Option<loom::sync::MutexGuard<T>> {
	        self.0.try_lock().ok()
	    }
	}
}

pub(crate) mod thread {
	#[cfg(loom)]
	pub(crate) use loom::thread::{spawn, JoinHandle};

	#[cfg(not(loom))]
	pub(crate) use std::thread::{spawn, JoinHandle};
}