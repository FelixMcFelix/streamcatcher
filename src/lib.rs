//! Top-of-line description.

#[cfg(feature = "async")]
pub mod future;
#[cfg(feature = "standard")]
mod standard;
mod state;

#[cfg(feature = "standard")]
pub use standard::*;
pub use state::Stateful;

use core::result::Result as CoreResult;
use std::error::Error;

pub type Result<T> = CoreResult<T, CatcherError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformPosition {
	Read(usize),
	Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatcherError {
	ChunkSize,
	IllegalFinaliser,
}

impl std::fmt::Display for CatcherError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{:?}", self)
	}
}

impl Error for CatcherError {}

#[derive(Clone, Debug)]
pub enum Finaliser {
	InPlace,
	NewThread,
	// #[cfg(feature = "async")]
	// Async(Box<dyn Spawn>),
	#[cfg(feature = "async-std-compat")]
	AsyncStd,
	#[cfg(feature = "tokio-compat")]
	Tokio,
	#[cfg(feature = "smol-compat")]
	Smol,
}

impl Finaliser {
	pub fn is_sync(&self) -> bool {
		matches!(self, Finaliser::InPlace | Finaliser::NewThread)
	}

	pub fn run_elsewhere(&self) -> bool {
		!matches!(self, Finaliser::InPlace)
	}
}

#[derive(Clone, Debug)]
pub struct Config {
	chunk_size: usize,
	spawn_finaliser: Finaliser,
	use_backing: bool,
	length_hint: Option<usize>,
	read_burst_len: usize,
}

impl Config {
	pub fn new() -> Self {
		Self {
			chunk_size: 4096,
			spawn_finaliser: Finaliser::NewThread,
			use_backing: true,
			length_hint: None,
			read_burst_len: 4096,
		}
	}

	/// The amount of bytes to allocate any time more space is required to
	/// store the stream.
	///
	/// A larger value is generally preferred for minimising locking and allocations
	/// but may reserve too much space before the struct is finalised.
	///
	/// If this is smaller than the minimum contiguous bytes needed for a coding type,
	/// or unspecified, then this will default to an estimated 5 seconds.
	pub fn chunk_size(&mut self, size: usize) -> &mut Self {
		self.chunk_size = size;
		self
	}

	/// Allocate a contiguous backing store to speed up reads after the stream ends.
	///
	/// Defaults to `true`.
	pub fn use_backing(&mut self, val: bool) -> &mut Self {
		self.use_backing = val;
		self
	}

	/// Spawn a new thread/task to move contents of the rope into backing storage once
	/// a stream completes.
	///
	/// Disabling this may negatively impact performance of the final read in a stream.
	///
	/// Defaults to `true`.
	pub fn spawn_finaliser(&mut self, finaliser: Finaliser) -> &mut Self {
		self.spawn_finaliser = finaliser;
		self
	}

	/// Estimate for the amount of data required to store the completed stream.
	///
	/// On `None`, this will default to `chunk_size`.
	///
	/// Defaults to `None`.
	pub fn length_hint(&mut self, hint: Option<usize>) -> &mut Self {
		self.length_hint = hint;
		self
	}
}

impl Default for Config {
	fn default() -> Self {
		Self::new()
	}
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Identity {}

#[derive(Debug)]
struct BufferChunk {
	data: Vec<u8>,

	start_pos: usize,
	end_pos: usize,
}

impl BufferChunk {
	fn new(start_pos: usize, chunk_len: usize) -> Self {
		BufferChunk {
			data: Vec::with_capacity(chunk_len),

			start_pos,
			end_pos: start_pos,
		}
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheReadLocation {
	Roped,
	Backed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinaliseState {
	Live,
	Finalising,
	Finalised,
}

impl From<u8> for FinaliseState {
	fn from(val: u8) -> Self {
		use FinaliseState::*;
		match val {
			0 => Live,
			1 => Finalising,
			2 => Finalised,
			_ => unreachable!(),
		}
	}
}

impl From<FinaliseState> for u8 {
	fn from(val: FinaliseState) -> Self {
		use FinaliseState::*;
		match val {
			Live => 0,
			Finalising => 1,
			Finalised => 2,
		}
	}
}

impl FinaliseState {
	fn is_source_live(self) -> bool {
		matches!(self, FinaliseState::Live)
	}

	fn is_source_finished(self) -> bool {
		!self.is_source_live()
	}

	fn is_backing_ready(self) -> bool {
		matches!(self, FinaliseState::Finalised)
	}
}
