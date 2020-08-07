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

/// Test description.
pub trait Stateful {
	type State;

	/// # Safety
	/// This is a view into a possibly mutable object.
	unsafe fn state(&self) -> Self::State;
}

#[cfg(feature = "standard")]
impl<T, Tx> Stateful for TxCatcher<T, Tx>
where
	T: Read,
	Tx: Transform<T> + Stateful,
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
		self.core.state()
	}
}

#[cfg(feature = "standard")]
impl<T, Tx> Stateful for SharedStore<T, Tx>
where
	T: Read,
	Tx: Transform<T> + Stateful,
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
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

	unsafe fn state(&self) -> Self::State {
		self.transform.state()
	}
}

#[cfg(feature = "async")]
impl<T, Tx> Stateful for AsyncTxCatcher<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Stateful + Unpin,
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
		self.core.state()
	}
}

#[cfg(feature = "async")]
impl<T, Tx> Stateful for AsyncSharedStore<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Stateful + Unpin,
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
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

	unsafe fn state(&self) -> Self::State {
		self.transform.state()
	}
}
