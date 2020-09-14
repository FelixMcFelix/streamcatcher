use crate::{
	cell::{UnsafeCell, UntrackedUnsafeCell},
	sync::{Arc, AtomicUsize, Mutex, Ordering},
	thread::JoinHandle,
	*,
};
use crossbeam_utils::Backoff;
use std::{
	collections::LinkedList,
	io::{
		self,
		Error as IoError,
		ErrorKind as IoErrorKind,
		Read,
		Result as IoResult,
		Seek,
		SeekFrom,
	},
	mem::{self, ManuallyDrop},
};

/// A simple shared stream buffer, leaving data unchanged.
pub type Catcher<T> = TxCatcher<T, Identity>;

/// Allows an input bytestream to be modified before it is stored.
///
/// Must be implemented alongside [NeedsBytes] in a functional transform.
///
/// [NeedsBytes]: trait.NeedsBytes.html
pub trait Transform<TInput: Read> {
	/// Data transform, given access to the underlying `Read` object and the destination buffer.
	///
	/// Transforms are free to make no change to `buf` without marking the stream as finalised:
	/// see [TransformPosition] for the semantics.
	///
	/// [TransformPosition]: enum.TransformPosition.html
	fn transform_read(&mut self, src: &mut TInput, buf: &mut [u8]) -> IoResult<TransformPosition>;
}

/// Common trait required by transforms, specifying how many contiguous bytes are needed
/// for any `read(...)` to succeed.
pub trait NeedsBytes {
	/// The minimum amount of contiguous bytes required in any rope segment.
	///
	/// Larger choices can simplify serialisation across rope segments (*i.e.*, `2` would
	/// ensure that a `u16` will never be split across segment boundaries).
	///
	/// *Defaults to `1`.*
	fn min_bytes_required(&self) -> usize {
		1
	}
}

impl<T: Read> Transform<T> for Identity {
	fn transform_read(&mut self, src: &mut T, buf: &mut [u8]) -> IoResult<TransformPosition> {
		Ok(match src.read(buf)? {
			0 => TransformPosition::Finished,
			n => TransformPosition::Read(n),
		})
	}
}

impl NeedsBytes for Identity {}

#[derive(Debug)]
/// A shared stream buffer, using an applied input data transform.
pub struct TxCatcher<T, Tx> {
	pub(crate) core: Arc<RawStore<T, Tx>>,
	pub(crate) pos: usize,
}

impl<T, Tx> TxCatcher<T, Tx>
where
	Tx: NeedsBytes + Default,
{
	/// Create a new stream buffer using the default transform and
	/// configuration.`
	pub fn new(source: T) -> Self {
		Self::with_tx(source, Default::default(), None)
			.expect("Default config should be guaranteed valid")
	}

	/// Create a new stream buffer using the default transform and
	/// a custom configuration.`
	pub fn with_config(source: T, config: Config) -> Result<Self> {
		Self::with_tx(source, Default::default(), Some(config))
	}
}

impl<T, Tx> TxCatcher<T, Tx>
where
	Tx: NeedsBytes,
{
	/// Create a new stream buffer using a custom transform and
	/// configuration.`
	pub fn with_tx(source: T, transform: Tx, config: Option<Config>) -> Result<Self> {
		RawStore::new(source, transform, config).map(|c| Self {
			core: Arc::new(c),
			pos: 0,
		})
	}
}

impl<T, Tx> TxCatcher<T, Tx> {
	/// Acquire a new handle to this object, creating a new
	/// view of the existing cached data from the beginning.
	pub fn new_handle(&self) -> Self {
		Self {
			core: self.core.clone(),
			pos: 0,
		}
	}

	/// Returns whether the underlying stream has been *finalised*, *i.e.*,
	/// whether all rope segments have been combined into a single contiguous backing store.
	///
	/// According to [Config], this may never become true if finalisation is disabled.
	///
	/// [Config]: struct.Config.html
	pub fn is_finalised(&self) -> bool {
		self.core.is_finalised()
	}

	/// Returns whether the underlying stream is *finished*, *i.e.*, all bytes it will
	/// produce have been stored in some fashion.
	pub fn is_finished(&self) -> bool {
		self.core.is_finished()
	}

	/// Returns this handle's position.
	pub fn pos(&self) -> usize {
		self.pos
	}

	/// Returns the number of bytes taken and stored from the transformed stream.
	pub fn len(&self) -> usize {
		self.core.len()
	}

	/// Returns whether any bytes have been stored from the transformed source.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// Returns a builder object to configure and connstruct a cache.
	pub fn builder() -> Config {
		Default::default()
	}
}

impl<T, Tx> TxCatcher<T, Tx>
where
	T: Read + 'static,
	Tx: Transform<T> + NeedsBytes + 'static,
{
	/// Spawn a new thread to read all bytes from the underlying stream
	/// into the backing store.
	pub fn spawn_loader(&self) -> JoinHandle<()> {
		let mut handle = self.new_handle();
		thread::spawn(move || {
			handle.load_all();
		})
	}

	/// Block the current thread to read all bytes from the underlying stream
	/// into the backing store.
	pub fn load_all(&mut self) {
		let pos = self.pos;
		while self.skip(1920 * mem::size_of::<f32>()) > 0 && !self.is_finalised() {}
		self.pos = pos;
	}
}

impl<T, Tx> Clone for TxCatcher<T, Tx> {
	fn clone(&self) -> Self {
		let mut out = self.new_handle();
		out.pos = self.pos;
		out
	}
}

impl<T, Tx> Read for TxCatcher<T, Tx>
where
	T: Read + 'static,
	Tx: Transform<T> + NeedsBytes + 'static,
{
	fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
		let (bytes_read, should_finalise_here) = self.core.read_from_pos(self.pos, buf);

		if should_finalise_here {
			let handle = self.core.clone();
			thread::spawn(move || handle.do_finalise());
		}

		if let Ok(size) = bytes_read {
			self.pos += size;
		}

		bytes_read
	}
}

impl<T, Tx> Seek for TxCatcher<T, Tx>
where
	T: Read + 'static,
	Tx: Transform<T> + NeedsBytes + 'static,
{
	fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
		let old_pos = self.pos as u64;

		let (valid, new_pos) = match pos {
			SeekFrom::Current(adj) => {
				// overflow expected in many cases.
				let new_pos = old_pos.wrapping_add(adj as u64);
				(adj >= 0 || (adj.abs() as u64) <= old_pos, new_pos)
			},
			SeekFrom::End(adj) => {
				// Reduce amount of skip calls in big load...
				self.pos = self.len();

				// Slower to load in the whole stream first, but safer.
				// We could, in theory, use metadata as the basis,
				// but none of our code takes this path, and incorrect
				// metadata would be tricky to work around.
				self.load_all();

				let len = self.len() as u64;
				let new_pos = len.wrapping_add(adj as u64);
				(adj >= 0 || (adj.abs() as u64) <= len, new_pos)
			},
			SeekFrom::Start(new_pos) => (true, new_pos),
		};

		if valid {
			if new_pos > old_pos {
				self.pos = (new_pos as usize).min(self.len());
				self.skip(new_pos as usize - self.pos);
			}

			let len = self.len() as u64;

			self.pos = new_pos.min(len) as usize;
			Ok(self.pos as u64)
		} else {
			Err(IoError::new(
				IoErrorKind::InvalidInput,
				"Tried to seek before start of stream.",
			))
		}
	}
}

// Shared basis for the below cache-based seekables.
#[derive(Debug)]
pub(crate) struct RawStore<T, Tx> {
	pub(crate) config: Config,
	pub(crate) transform: UnsafeCell<Tx>,
	pub(crate) source: UnsafeCell<Option<T>>,

	pub(crate) lock: Mutex<()>,
	pub(crate) rope_users_and_state: AtomicUsize,

	// Can't easily track this due to std data structure.
	// Maybe intrusive could manage this better?
	pub(crate) rope: UntrackedUnsafeCell<Option<LinkedList<BufferChunk>>>,
	pub(crate) backing_store: UnsafeCell<Option<Vec<u8>>>,
	pub(crate) len: AtomicUsize,
}

impl<T, Tx> RawStore<T, Tx>
where
	Tx: NeedsBytes,
{
	pub(crate) fn new(source: T, transform: Tx, config: Option<Config>) -> Result<Self> {
		let config = config.unwrap_or_else(Default::default);
		let min_bytes = transform.min_bytes_required();

		if config.chunk_size.lower_bound() < min_bytes {
			return Err(CatcherError::ChunkSize);
		};

		let mut start_size = if let Some(length) = config.length_hint {
			length
		} else {
			config.chunk_size.lower_bound()
		};

		if start_size < min_bytes {
			start_size = min_bytes;
		}

		let mut list = LinkedList::new();
		list.push_back(BufferChunk::new(Default::default(), start_size));

		Ok(Self {
			config,

			len: Default::default(),

			transform: UnsafeCell::new(transform),

			source: UnsafeCell::new(Some(source)),

			backing_store: UnsafeCell::new(None),
			rope: UntrackedUnsafeCell::new(Some(list)),
			rope_users_and_state: AtomicUsize::new(0),
			lock: Mutex::new(()),
		})
	}
}

impl<T, Tx> RawStore<T, Tx> {
	pub(crate) fn len(&self) -> usize {
		self.len.load(Ordering::Acquire)
	}

	pub(crate) fn finalised(&self) -> FinaliseState {
		self.rope_users_and_state.load(Ordering::Acquire).state()
	}

	pub(crate) fn is_finalised(&self) -> bool {
		self.finalised() == FinaliseState::Finalised
	}

	pub(crate) fn is_finished(&self) -> bool {
		self.finalised() != FinaliseState::Live
	}

	/// Marks stream as finished.
	///
	/// Returns `true` if a new handle must be spawned by the parent
	/// to finalise in another thread.
	pub(crate) fn finalise(&self) -> bool {
		let state_on_call = self.upgrade_state(Ordering::AcqRel).state();

		if state_on_call.is_source_live() {
			if self.config.spawn_finaliser.run_elsewhere() {
				true
			} else {
				self.do_finalise();
				false
			}
		} else {
			false
		}
	}

	pub(crate) fn do_finalise(&self) {
		if !self.config.use_backing {
			// If we don't want to use backing, then still remove the source.
			// This state will prevent anyone from trying to use the backing store.
			self.source.with_mut(|ptr| *(unsafe { &mut *ptr }) = None);
			return;
		}

		let backing_len = self.len();

		self.rope.with_mut(|ptr| {
			// Move the rope of bytes into the backing store.
			let rope = (unsafe { &mut *ptr })
				.as_mut()
				.expect("Finalisation should only occur while the rope exists.");

			if rope.len() > 1 {
				// Allocate one big store, then start moving entries over
				// chunk-by-chunk.
				let mut back = vec![0u8; backing_len];

				for el in rope.iter() {
					let start = el.start_pos;
					let end = el.end_pos;
					back[start..end].copy_from_slice(&el.data[..end - start]);
				}

				// Insert the new backing store, but DO NOT purge the old.
				// This is left to the last Arc<> holder of the rope.
				self.backing_store
					.with_mut(move |ptr| *(unsafe { &mut *ptr }) = Some(back));
			} else {
				// Least work, but unsafe.
				// We move the first chunk's buffer to become the backing store,
				// temporarily aliasing it until the list is destroyed.
				// In this case, when the list is destroyed, the first element
				// MUST be leaked to keep the backing store memory valid.
				//
				// (see remove_rope for this leakage)
				//
				// The alternative (write first chunk into always-present
				// backing store) mandates a lock for the final expansion, because
				// the backing store is IN USE. Thus, we can't employ it.
				if let Some(el) = rope.front_mut() {
					// We can be certain that this pointer is not invalidated because:
					// * All writes to the rope/rope are finished. Thus, no
					//   reallocations/moves.
					// * The Vec will live exactly as long as the RawStore, pointer never escapes.
					// Likewise, we knoe that it is safe to build the new vector as:
					// * The stored type and pointer do not change, so alignment is preserved.
					// * The data pointer is created by an existing Vec<T>.
					self.backing_store.with_mut(move |ptr| unsafe {
						let data = el.data.as_mut_ptr();
						*ptr = Some(Vec::from_raw_parts(data, el.data.len(), el.data.capacity()))
					});
				}
			}
		});

		// Drop the old input.
		self.source.with_mut(|ptr| *(unsafe { &mut *ptr }) = None);

		// It's crucial that we do this *last*, as this is the signal
		// for other threads to migrate from rope to backing store.
		self.upgrade_state(Ordering::Release);
	}

	pub(crate) fn upgrade_state(&self, order: Ordering) -> usize {
		self.rope_users_and_state
			.fetch_add(1 << usize::SHIFT_AMT, order)
	}

	pub(crate) fn add_rope(&self) -> usize {
		self.rope_users_and_state.fetch_add(1, Ordering::AcqRel)
	}

	pub(crate) fn remove_rope(&self) -> usize {
		self.rope_users_and_state.fetch_sub(1, Ordering::AcqRel)
	}

	pub(crate) fn remove_rope_full(&self) {
		// We can only remove the rope if the core holds the last reference.
		// Given that the number of active handles at this moment is returned,
		// we also know the amount *after* subtraction.
		let val = self.remove_rope() - 1;
		let remaining = val.holders();
		let finished = val.state();

		if finished.is_backing_ready() {
			self.try_delete_rope(remaining);
		}
	}

	pub(crate) fn try_delete_rope(&self, seen_count: Holders<usize>) {
		// This branch will only be visited if BOTH the rope and
		// backing store exist simultaneously.
		if seen_count.0 == 0 {
			// In worst case, several upgrades might pile up.
			// Only one holder should concern itself with drop logic,
			// the rest should carry on and start using the backing store.
			let maybe_lock = self.lock.try_lock();
			if maybe_lock.is_none() {
				return;
			}

			self.rope.with_mut(|ptr| {
				let rope_access = unsafe { &mut *ptr };

				if let Some(rope) = rope_access {
					// Prevent the backing store from being wiped out
					// if the first link in the rope sufficed.
					// This ensures safety as we undo the aliasing
					// in the above special case.
					if rope.len() == 1 {
						let el = rope
							.pop_front()
							.expect("Length of rope was established as >= 1.");
						ManuallyDrop::new(el.data);
					}
				}

				// Drop everything else.
				*rope_access = None;
			});
		}
	}

	// Note: if you get a Rope, you need to later call remove_rope to remain sound.
	// This call has the side effect of trying to safely delete the rope.
	pub(crate) fn get_location(&self) -> (CacheReadLocation, FinaliseState) {
		let info_before = self.add_rope();
		let finalised = info_before.state();

		let loc = if finalised.is_backing_ready() {
			// try to remove rope.
			// This gives the user count *before*
			let remaining_users = info_before.holders();
			self.try_delete_rope(remaining_users);
			self.rope_users_and_state.fetch_sub(1, Ordering::AcqRel);

			CacheReadLocation::Backed
		} else {
			CacheReadLocation::Roped
		};

		(loc, finalised)
	}

	#[inline]
	pub(crate) fn read_from_local(
		&self,
		mut pos: usize,
		loc: CacheReadLocation,
		buf: &mut [u8],
		count: usize,
	) -> usize {
		use CacheReadLocation::*;
		match loc {
			Backed =>
				if pos < self.len() {
					self.backing_store.with(|ptr| {
						let store = unsafe { &*ptr }.as_ref().expect(
							"Reader should not attempt to use a backing store before it exists",
						);

						buf[..count].copy_from_slice(&store[pos..pos + count]);

						count
					})
				} else {
					0
				},
			Roped => {
				self.rope.with(|ptr| {
					let rope = unsafe { &*ptr }.as_ref().expect(
						"Rope should still exist while any handles hold a ::Roped(_) \
									 (and thus an Arc)",
					);

					let mut written = 0;

					for link in rope.iter() {
						// Although this isn't atomic, Release on store to .len ensures that
						// all writes made before setting len STAY before len.
						// backing_pos might be larger than len, and fluctuates
						// due to resizes, BUT we're gated by the atomically written len,
						// via count, which gives us a safe bound on accessible bytes this call.
						if pos >= link.start_pos && pos < link.end_pos {
							let local_available = link.end_pos - pos;
							let to_write = (count - written).min(local_available);

							let first_el = pos - link.start_pos;

							let next_len = written + to_write;

							buf[written..next_len]
								.copy_from_slice(&link.data[first_el..first_el + to_write]);

							written = next_len;
							pos += to_write;
						}

						if written >= buf.len() {
							break;
						}
					}

					count
				})
			},
		}
	}
}

impl<T, Tx> RawStore<T, Tx>
where
	T: Read,
	Tx: Transform<T> + NeedsBytes,
{
	/// Returns read count, should_upgrade, should_finalise_external
	fn read_from_pos(&self, pos: usize, buf: &mut [u8]) -> (IoResult<usize>, bool) {
		// Place read of finalised first to be certain that if we see finalised,
		// then backing_len *must* be the true length.
		let (loc, mut finalised) = self.get_location();

		let mut backing_len = self.len();

		let mut should_finalise_external = false;

		let target_len = pos + buf.len();

		let out = if finalised.is_source_finished() || target_len <= backing_len {
			// If finalised, there is zero risk of triggering more writes.
			let read_amt = buf.len().min(backing_len - pos);
			Ok(self.read_from_local(pos, loc, buf, read_amt))
		} else {
			let mut read = 0;
			let mut base_result = None;
			let backoff = Backoff::new();

			'byteread: loop {
				finalised = self.finalised();
				backing_len = self.len();
				let mut remaining_in_store = backing_len - pos - read;

				if remaining_in_store == 0 {
					// Need to do this to trigger the lock
					// while holding mutability to the other members.
					let lock: *const Mutex<()> = &self.lock;
					#[cfg(loom)]
					let guard = unsafe {
						let lock = &*lock;

						#[cfg(loom)]
						lock.lock()
					};
					#[cfg(not(loom))]
					let guard = unsafe {
						let lock = &*lock;

						lock.try_lock()
					};

					#[cfg(not(loom))]
					if guard.is_none() {
						backoff.spin();
						continue 'byteread;
					}

					finalised = self.finalised();
					backing_len = self.len();

					// If length changed between our check and
					// acquiring the lock, then drop it -- we don't need new bytes *yet*
					// and might not!
					remaining_in_store = backing_len - pos - read;
					if remaining_in_store == 0 && finalised.is_source_live() {
						let read_count = self.fill_from_source(buf.len() - read);
						if let Ok((read_count, finalise_elsewhere)) = read_count {
							remaining_in_store += read_count;
							should_finalise_external |= finalise_elsewhere;
						}
						base_result = Some(read_count.map(|a| a.0));

						finalised = self.finalised();
					}

					// Unlocked here.
					mem::drop(guard);
					#[cfg(loom)]
					::loom::thread::yield_now();
				}

				if remaining_in_store > 0 {
					backoff.reset();
					let count = remaining_in_store.min(buf.len() - read);
					read += self.read_from_local(pos, loc, &mut buf[read..], count);
				}

				// break out if:
				// * no space in reader's buffer
				// * hit an error
				// * or nothing remaining, AND finalised
				if matches!(base_result, Some(Err(_)))
					|| read == buf.len() || (finalised.is_source_finished()
					&& backing_len == pos + read)
				{
					break;
				}
			}

			base_result.unwrap_or(Ok(0)).map(|_| read)
		};

		if loc == CacheReadLocation::Roped {
			self.remove_rope_full();
		}

		(out, should_finalise_external)
	}

	// ONLY SAFE TO CALL WITH LOCK.
	// The critical section concerns:
	// * adding new elements to the rope
	// * drawing bytes from the source
	// * modifying len
	// * modifying encoder state
	fn fill_from_source(&self, mut bytes_needed: usize) -> IoResult<(usize, bool)> {
		let minimum_to_write = self
			.transform
			.with(|ptr| unsafe { &*ptr }.min_bytes_required());

		let overspill = bytes_needed % self.config.read_burst_len;
		if overspill != 0 {
			bytes_needed += self.config.read_burst_len - overspill;
		}

		let mut remaining_bytes = bytes_needed;
		let mut recorded_error = None;

		let mut spawn_new_finaliser = false;

		loop {
			let should_break = self.rope.with_mut(|ptr| {
				let rope = unsafe { &mut *ptr }
					.as_mut()
					.expect("Writes should only occur while the rope exists.");

				let chunk_count = rope.len();

				let rope_el = rope
					.back_mut()
					.expect("There will always be at least one element in rope.");

				let old_len = rope_el.data.len();
				let cap = rope_el.data.capacity();
				let space = cap - old_len;

				let new_len = old_len + space.min(remaining_bytes);

				if space < minimum_to_write {
					let end = rope_el.end_pos;
					// Make a new chunk!
					rope.push_back(BufferChunk::new(
						end,
						self.config.next_chunk_size(cap, chunk_count),
					));

					false
				} else {
					rope_el.data.resize(new_len, 0);

					let pos = self.transform.with_mut(|tx_ptr| {
						self.source.with_mut(|src_ptr| {
							let src = unsafe { &mut *src_ptr }
								.as_mut()
								.expect("Source must exist while not finalised.");

							unsafe { &mut *tx_ptr }
								.transform_read(src, &mut rope_el.data[old_len..])
						})
					});

					match pos {
						Ok(TransformPosition::Read(len)) => {
							rope_el.end_pos += len;
							rope_el.data.truncate(old_len + len);

							remaining_bytes -= len;
							self.len.fetch_add(len, Ordering::Release);
						},
						Ok(TransformPosition::Finished) => {
							spawn_new_finaliser = self.finalise();
						},
						Err(e) if e.kind() == IoErrorKind::Interrupted => {
							// DO nothing, so try again.
						},
						Err(e) => {
							recorded_error = Some(Err(e));
						},
					}

					self.finalised().is_source_finished()
						|| remaining_bytes < minimum_to_write
						|| recorded_error.is_some()
				}
			});

			if should_break {
				break;
			}
		}

		recorded_error.unwrap_or(Ok((bytes_needed - remaining_bytes, spawn_new_finaliser)))
	}
}

impl<T, Tx> Drop for RawStore<T, Tx> {
	fn drop(&mut self) {
		// This is necesary to prevent unsoundness.
		// I.e., 1-chunk case after finalisation if
		// one handle is left in Rope, then dropped last
		// would cause a double free due to aliased chunk.
		let remaining_users = self.rope_users_and_state.load(Ordering::Acquire).holders();
		self.try_delete_rope(remaining_users);
	}
}

// We need to declare these as thread-safe, since we don't have a mutex around
// several raw fields. However, the way that they are used should remain
// consistent.
unsafe impl<T, Tx> Sync for RawStore<T, Tx> {}
unsafe impl<T, Tx> Send for RawStore<T, Tx> {}

/// Utility trait to scan forward by discarding bytes.
pub trait ReadSkipExt {
	fn skip(&mut self, amt: usize) -> usize
	where
		Self: Sized;
}

impl<R: Read + Sized> ReadSkipExt for R {
	fn skip(&mut self, amt: usize) -> usize {
		io::copy(&mut self.by_ref().take(amt as u64), &mut io::sink()).unwrap_or(0) as usize
	}
}

#[cfg(test)]
mod tests {
	use crate::*;
}
