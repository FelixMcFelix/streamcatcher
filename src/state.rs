#[cfg(feature = "async")]
use crate::future::{
	AsyncTransform,
	RawStore as AsyncRawStore,
	SharedStore as AsyncSharedStore,
	TxCatcher as AsyncTxCatcher,
};
use crate::*;
#[cfg(feature = "async")]
use futures::io::AsyncRead;
#[cfg(feature = "standard")]
use std::io::Read;

/// Transforms who can be queried about their internal state.
pub trait Stateful {
	/// Transform state.
	type State;

	fn state(&self) -> Self::State;
}

/// External access to (Async)[`Transform`] state via a [`TxCatcher`] (resp. async variants).
///
/// [`Transform`]: trait.Transform.html
/// [`TxCatcher`]: struct.TxCatcher.html
pub trait StateAccess {
	/// Transform state.
	type State;

	/// Directly access a transform's state before a stream has finished.
	///
	/// # Safety
	/// This accesses the underlying transform without first acquiring a lock,
	/// possibly causing shared access to the transform struct.
	/// Retrieved state could be generated based on an inconsistent or semi-committed
	/// mutable access to (Async)[`Transform`].
	///
	/// To use safely, implementors of [`Stateful`] *must* ensure that appropriate
	/// concurrency controls are used (*e.g.*, atomics or locks) when producing state
	/// data.
	///
	/// [`Transform`]: trait.Transform.html
	/// [`Stateful`]: trait.Stateful.html
	unsafe fn get_state_unchecked(&self) -> Self::State;

	/// Returns the transform's state if the source (and transform) are finished.
	///
	/// As no future write accesses to the transform object can occur once the stream
	/// finishes, accesses are guaranteed to be safe.
	fn get_final_state(&self) -> Option<Self::State>;
}

#[cfg(feature = "standard")]
impl<T, Tx> StateAccess for TxCatcher<T, Tx>
where
	T: Read,
	Tx: Transform<T> + Stateful,
{
	type State = <Tx as Stateful>::State;

	unsafe fn get_state_unchecked(&self) -> Self::State {
		self.core.state()
	}

	fn get_final_state(&self) -> Option<Self::State> {
		if self.is_finished() {
			// Safety: no more reads, so state of transform
			// cannot change
			Some(self.core.state())
		} else {
			None
		}
	}
}

#[cfg(feature = "standard")]
impl<T, Tx> Stateful for SharedStore<T, Tx>
where
	T: Read,
	Tx: Transform<T> + Stateful,
{
	type State = <Tx as Stateful>::State;

	fn state(&self) -> Self::State {
		self.get_mut_ref().state()
	}
}

#[cfg(feature = "standard")]
impl<T, Tx> Stateful for RawStore<T, Tx>
where
	T: Read,
	Tx: Transform<T> + Stateful,
{
	type State = <Tx as Stateful>::State;

	fn state(&self) -> Self::State {
		self.transform.state()
	}
}

#[cfg(feature = "async")]
impl<T, Tx> StateAccess for AsyncTxCatcher<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Stateful + Unpin,
{
	type State = <Tx as Stateful>::State;

	unsafe fn get_state_unchecked(&self) -> Self::State {
		self.core.state()
	}

	fn get_final_state(&self) -> Option<Self::State> {
		if self.is_finished() {
			// Safety: no more reads, so state of transform
			// cannot change
			Some(self.core.state())
		} else {
			None
		}
	}
}

#[cfg(feature = "async")]
impl<T, Tx> Stateful for AsyncSharedStore<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Stateful + Unpin,
{
	type State = <Tx as Stateful>::State;

	fn state(&self) -> Self::State {
		self.get_mut_ref().state()
	}
}

#[cfg(feature = "async")]
impl<T, Tx> Stateful for AsyncRawStore<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Stateful + Unpin,
{
	type State = <Tx as Stateful>::State;

	fn state(&self) -> Self::State {
		self.transform.state()
	}
}
