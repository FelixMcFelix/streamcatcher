use crate::*;

/// Test description.
pub trait Stateful {
	type State;

	/// # Safety
	/// Thi is a view into a possibly mutable object.
	unsafe fn state(&self) -> Self::State;
}

impl<T, Tx> Stateful for TxCatcher<T, Tx>
	where T: Read,
		Tx: Transform<T> + Stateful
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
		self.core.state()
	}
}

impl<T, Tx> Stateful for SharedStore<T, Tx>
	where T: Read,
		Tx: Transform<T> + Stateful
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
		self.get_mut_ref().state()
	}
}

impl<T, Tx> Stateful for RawStore<T, Tx>
	where T: Read,
		Tx: Transform<T> + Stateful
{
	type State = <Tx as Stateful>::State;

	unsafe fn state(&self) -> Self::State {
		self.transform.state()
	}
}