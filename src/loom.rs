pub(crate) mod cell {
	#[cfg(loom)]
	pub(crate) use loom::cell::UnsafeCell;

	#[cfg(not(loom))]
	pub(crate) use UntrackedUnsafeCell as UnsafeCell;

	#[derive(Debug)]
	pub(crate) struct UntrackedUnsafeCell<T>(std::cell::UnsafeCell<T>);

	impl<T> UntrackedUnsafeCell<T> {
		pub(crate) fn new(data: T) -> UntrackedUnsafeCell<T> {
			Self(std::cell::UnsafeCell::new(data))
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
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	#[cfg(not(loom))]
	pub(crate) use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	#[cfg(not(loom))]
	pub(crate) use futures_util::lock::Mutex;

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
