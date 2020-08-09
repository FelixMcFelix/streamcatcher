//! Top-of-line description.
use crate::*;
use parking_lot::{lock_api::MutexGuard, Mutex};
use std::{
	cell::UnsafeCell,
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
	sync::{
		atomic::{AtomicU8, AtomicUsize, Ordering},
		Arc,
	},
};

/// The basics.
pub type Catcher<T> = TxCatcher<T, Identity>;

pub trait Transform<TInput: Read> {
	fn transform_read(&mut self, src: &mut TInput, buf: &mut [u8]) -> IoResult<TransformPosition>;

	/// Contiguous specifically.
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

#[derive(Clone, Debug)]
/// Test desc.
pub struct TxCatcher<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	pub(crate) core: Arc<SharedStore<T, Tx>>,
	pub(crate) pos: usize,
}

impl<T, Tx> TxCatcher<T, Tx>
where
	T: Read,
	Tx: Transform<T> + Default,
{
	pub fn new(source: T, config: Option<Config>) -> Result<Self> {
		Self::new_tx(source, Default::default(), config)
	}
}

impl<T, Tx> TxCatcher<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	pub fn new_tx(source: T, transform: Tx, config: Option<Config>) -> Result<Self> {
		RawStore::new(source, transform, config).map(|c| Self {
			core: Arc::new(SharedStore {
				raw: UnsafeCell::new(c),
			}),
			pos: 0,
		})
	}

	/// Acquire a new handle to this object, to begin a new
	/// source from the exsting cached data.
	pub fn new_handle(&self) -> Self {
		Self {
			core: self.core.clone(),
			pos: 0,
		}
	}

	pub fn is_finalised(&self) -> bool {
		self.core.is_finalised()
	}

	pub fn pos(&self) -> usize {
		self.pos
	}

	pub fn len(&self) -> usize {
		self.core.len()
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

impl<T, Tx> TxCatcher<T, Tx>
where
	T: Read + 'static,
	Tx: Transform<T> + 'static,
{
	/// Spawn a new thread to read all bytes from the underlying stream
	/// into the backing store.
	pub fn spawn_loader(&self) -> std::thread::JoinHandle<()> {
		let mut handle = self.new_handle();
		std::thread::spawn(move || {
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

impl<T, Tx> Read for TxCatcher<T, Tx>
where
	T: Read + 'static,
	Tx: Transform<T> + 'static,
{
	fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
		let (bytes_read, should_finalise_here) = self.core.read_from_pos(self.pos, buf);

		if should_finalise_here {
			let handle = self.core.clone();
			std::thread::spawn(move || handle.do_finalise());
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
	Tx: Transform<T> + 'static,
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

#[derive(Debug)]
pub(crate) struct SharedStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	raw: UnsafeCell<RawStore<T, Tx>>,
}

impl<T, Tx> SharedStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	// The main reason for employing `unsafe` here is *shared mutability*:
	// due to the granularity of the locks we need, (i.e., a moving critical
	// section otherwise lock-free), we need to assert that these operations
	// are safe.
	//
	// Note that only our code can use this, so that we can ensure correctness
	// and concurrent safety.
	#[allow(clippy::mut_from_ref)]
	pub(crate) fn get_mut_ref(&self) -> &mut RawStore<T, Tx> {
		unsafe { &mut *self.raw.get() }
	}

	fn read_from_pos(&self, pos: usize, buffer: &mut [u8]) -> (IoResult<usize>, bool) {
		self.get_mut_ref().read_from_pos(pos, buffer)
	}

	fn len(&self) -> usize {
		self.get_mut_ref().len()
	}

	fn is_finalised(&self) -> bool {
		self.get_mut_ref().finalised().is_source_finished()
	}

	fn do_finalise(&self) {
		self.get_mut_ref().do_finalise()
	}
}

// Shared basis for the below cache-based seekables.
#[derive(Debug)]
pub(crate) struct RawStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	config: Config,

	len: AtomicUsize,
	finalised: AtomicU8,

	pub(crate) transform: Tx,

	source: Option<T>,

	backing_store: Option<Vec<u8>>,
	rope: Option<LinkedList<BufferChunk>>,
	rope_users: AtomicUsize,
	lock: Mutex<()>,
}

impl<T, Tx> RawStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	fn new(source: T, transform: Tx, config: Option<Config>) -> Result<Self> {
		let config = config.unwrap_or_else(Default::default);
		let min_bytes = transform.min_bytes_required();

		if config.chunk_size < min_bytes {
			return Err(CatcherError::ChunkSize);
		};

		if !config.spawn_finaliser.is_sync() {
			return Err(CatcherError::IllegalFinaliser);
		};

		let mut start_size = if let Some(length) = config.length_hint {
			length
		} else {
			config.chunk_size
		};

		if start_size < min_bytes {
			start_size = min_bytes;
		}

		let mut list = LinkedList::new();
		list.push_back(BufferChunk::new(Default::default(), start_size));

		Ok(Self {
			config,

			len: Default::default(),
			finalised: AtomicU8::new(FinaliseState::Live.into()),

			transform,

			source: Some(source),

			backing_store: None,
			rope: Some(list),
			rope_users: AtomicUsize::new(1),
			lock: Mutex::new(()),
		})
	}

	fn len(&self) -> usize {
		self.len.load(Ordering::Acquire)
	}

	fn finalised(&self) -> FinaliseState {
		self.finalised.load(Ordering::Acquire).into()
	}

	/// Marks stream as finished.
	///
	/// Returns `true` if a new handle must be spawned by the parent
	/// to finalise in another thread.
	fn finalise(&mut self) -> bool {
		let state_on_call: FinaliseState = self
			.finalised
			.compare_and_swap(
				FinaliseState::Live.into(),
				FinaliseState::Finalising.into(),
				Ordering::AcqRel,
			)
			.into();

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

	fn do_finalise(&mut self) {
		if !self.config.use_backing {
			// If we don't want to use backing, then still remove the source.
			// This state will prevent anyone from trying to use the backing store.
			self.source = None;
			self.finalised
				.store(FinaliseState::Finalising.into(), Ordering::Release);
			return;
		}

		let backing_len = self.len();

		// Move the rope of bytes into the backing store.
		let rope = self
			.rope
			.as_mut()
			.expect("Writes should only occur while the rope exists.");

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
			self.backing_store = Some(back);
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
				self.backing_store = Some(unsafe {
					let data = el.data.as_mut_ptr();
					Vec::from_raw_parts(data, el.data.len(), el.data.capacity())
				})
			}
		}

		// Drop the old input.
		self.source = None;

		// It's crucial that we do this *last*, as this is the signal
		// for other threads to migrate from rope to backing store.
		self.finalised
			.store(FinaliseState::Finalised.into(), Ordering::Release);
	}

	fn add_rope(&mut self) {
		self.rope_users.fetch_add(1, Ordering::AcqRel);
	}

	fn remove_rope_ref(&mut self, finished: FinaliseState) {
		// We can only remove the rope if the core holds the last reference.
		// Given that the number of active handles at this moment is returned,
		// we also know the amount *after* subtraction.
		let remaining = self.rope_users.fetch_sub(1, Ordering::AcqRel) - 1;

		if finished.is_backing_ready() {
			self.try_delete_rope(remaining);
		}
	}

	fn try_delete_rope(&mut self, seen_count: usize) {
		// This branch will only be visited if BOTH the rope and
		// backing store exist simultaneously.
		if seen_count == 1 {
			// In worst case, several upgrades might pile up.
			// Only one holder should concern itself with drop logic,
			// the rest should carry on and start using the backing store.
			let maybe_lock = self.lock.try_lock();
			if maybe_lock.is_none() {
				return;
			}

			if let Some(rope) = &mut self.rope {
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
			self.rope = None;
			self.rope_users.store(0, Ordering::Release);
		}
	}

	// Note: if you get a Rope, you need to later call remove_rope to remain sound.
	// This call has the side effect of trying to safely delete the rope.
	fn get_location(&mut self) -> (CacheReadLocation, FinaliseState) {
		let finalised = self.finalised();

		let loc = if finalised.is_backing_ready() {
			// try to remove rope.
			let remaining_users = self.rope_users.load(Ordering::Acquire);
			self.try_delete_rope(remaining_users);
			CacheReadLocation::Backed
		} else {
			self.add_rope();
			CacheReadLocation::Roped
		};

		(loc, finalised)
	}

	/// Returns read count, should_upgrade, should_finalise_external
	fn read_from_pos(&mut self, pos: usize, buf: &mut [u8]) -> (IoResult<usize>, bool) {
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

			loop {
				finalised = self.finalised();
				backing_len = self.len();
				let mut remaining_in_store = backing_len - pos - read;

				if remaining_in_store == 0 {
					// Need to do this to trigger the lock
					// while holding mutability to the other members.
					let lock: *mut Mutex<()> = &mut self.lock;
					let guard = unsafe {
						let lock = &*lock;
						lock.lock()
					};

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
					MutexGuard::unlock_fair(guard);
				}

				if remaining_in_store > 0 {
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
			self.remove_rope_ref(finalised);
		}

		(out, should_finalise_external)
	}

	// ONLY SAFE TO CALL WITH LOCK.
	// The critical section concerns:
	// * adding new elements to the rope
	// * drawing bytes from the source
	// * modifying len
	// * modifying encoder state
	fn fill_from_source(&mut self, mut bytes_needed: usize) -> IoResult<(usize, bool)> {
		let minimum_to_write = self.transform.min_bytes_required();

		let overspill = bytes_needed % self.config.read_burst_len;
		if overspill != 0 {
			bytes_needed += self.config.read_burst_len - overspill;
		}

		let mut remaining_bytes = bytes_needed;
		let mut recorded_error = None;

		let mut spawn_new_finaliser = false;

		loop {
			let rope = self
				.rope
				.as_mut()
				.expect("Writes should only occur while the rope exists.");

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
				rope.push_back(BufferChunk::new(end, self.config.chunk_size));
			} else {
				rope_el.data.resize(new_len, 0);

				let src = self
					.source
					.as_mut()
					.expect("Source must exist while not finalised.");

				let pos = self
					.transform
					.transform_read(src, &mut rope_el.data[old_len..]);
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

				if self.finalised().is_source_finished()
					|| remaining_bytes < minimum_to_write
					|| recorded_error.is_some()
				{
					break;
				}
			}
		}

		recorded_error.unwrap_or(Ok((bytes_needed - remaining_bytes, spawn_new_finaliser)))
	}

	#[inline]
	fn read_from_local(
		&self,
		mut pos: usize,
		loc: CacheReadLocation,
		buf: &mut [u8],
		count: usize,
	) -> usize {
		use CacheReadLocation::*;
		match loc {
			Backed => {
				let store = self
					.backing_store
					.as_ref()
					.expect("Reader should not attempt to use a backing store before it exists");

				if pos < self.len() {
					buf[..count].copy_from_slice(&store[pos..pos + count]);

					count
				} else {
					0
				}
			},
			Roped => {
				let rope = self.rope.as_ref().expect(
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
			},
		}
	}
}

impl<T, Tx> Drop for RawStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
	fn drop(&mut self) {
		// This is necesary to prevent unsoundness.
		// I.e., 1-chunk case after finalisation if
		// one handle is left in Rope, then dropped last
		// would cause a double free due to aliased chunk.
		let remaining_users = self.rope_users.load(Ordering::Acquire);
		self.try_delete_rope(remaining_users);
	}
}

// We need to declare these as thread-safe, since we don't have a mutex around
// several raw fields. However, the way that they are used should remain
// consistent.
unsafe impl<T, Tx> Sync for SharedStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
}
unsafe impl<T, Tx> Send for SharedStore<T, Tx>
where
	T: Read,
	Tx: Transform<T>,
{
}

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

	#[cfg(all(
		feature = "async",
		feature = "smol-compat",
		feature = "tokio-compat",
		feature = "async-std-compat"
	))]
	#[test]
	fn only_sync_finalisers() {
		const INPUT: [u8; 1] = [0];

		use Finaliser::*;
		let mut illegals = vec![Finaliser::AsyncStd, Finaliser::Tokio, Finaliser::Smol];

		for fin in illegals.drain(0..) {
			let mut cfg = Config::new();

			cfg.spawn_finaliser(fin);

			let catcher = Catcher::new(&INPUT[..], Some(cfg));

			assert!(matches!(catcher, Err(CatcherError::IllegalFinaliser)));
		}
	}
}
