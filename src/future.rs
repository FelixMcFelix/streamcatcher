//! Top-of-line description.
use async_trait::async_trait;
use crate::*;
use core::{
	future::Future,
	pin::Pin,
	result::Result as CoreResult,
	task::{Context, Poll},
};
use futures::{
	io::{
		self,
		AsyncRead,
		AsyncReadExt,
		AsyncSeek,
	},
	lock::{Mutex, MutexGuard},
};
use std::{
	cell::UnsafeCell,
	collections::LinkedList,
	error::Error,
	io::{
		Error as IoError,
		ErrorKind as IoErrorKind,
		Read,
		Result as IoResult,
		Seek,
		SeekFrom,
	},
	marker::Unpin,
	mem::{
		self,
		ManuallyDrop,
	},
	sync::{
		atomic::{
			AtomicU8,
			AtomicUsize, 
			Ordering,
		},
		Arc,
	},
};
#[cfg(feature = "tokio-compat")]
use tokio::io::{
	AsyncRead as TokioRead,
	AsyncSeek as TokioSeek,
};

pub type Catcher<T> = TxCatcher<T, Identity>;

pub trait AsyncTransform<TInput: AsyncRead> {
	fn transform_poll_read(&mut self, src: Pin<&mut TInput>, cx: &mut Context, buf: &mut [u8]) -> Poll<IoResult<TransformPosition>>;

	/// Contiguous specifically.
	fn min_bytes_required(&self) -> usize {
		1
	}
}

impl<T: AsyncRead> AsyncTransform<T> for Identity {
	fn transform_poll_read(&mut self, src: Pin<&mut T>, cx: &mut Context, buf: &mut [u8]) -> Poll<IoResult<TransformPosition>> {
		src.poll_read(cx, buf)
			.map(|res| res.map(|count| match count {
				0 => TransformPosition::Finished,
				n => TransformPosition::Read(n),
			}))	
	}
}

#[derive(Clone, Debug)]
/// Test desc.
pub struct TxCatcher<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Unpin,
{
	pub(crate) core: Arc<SharedStore<T, Tx>>,
	pos: usize,
}

impl<T, Tx> TxCatcher<T, Tx>
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin + Default,
{
	pub fn new(source: T, config: Option<Config>) -> Result<Self> {
		Self::new_tx(source, Default::default(), config)
	}
}

impl<T, Tx> TxCatcher<T, Tx>
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin,
{
	pub fn new_tx(source: T, transform: Tx, config: Option<Config>) -> Result<Self> {
		RawStore::new(source, transform, config)
			.map(|c| Self {
				core: Arc::new(SharedStore{ raw: UnsafeCell::new(c) }),
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
}

impl<T, Tx> TxCatcher<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	/// Read all bytes from the underlying stream
	/// into the backing store in the current task.
	pub fn load_all(mut self) -> LoadAll<T, Tx> {
		LoadAll::new(self)
	}
}

pub struct LoadAll<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	catcher: TxCatcher<T, Tx>,
	in_pos: usize,
}

impl<T, Tx> LoadAll<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn new(catcher: TxCatcher<T, Tx>) -> Self {
		let in_pos = catcher.pos;

		Self {
			catcher,
			in_pos,
		}
	}
}

impl<T, Tx> Future for LoadAll<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	type Output = TxCatcher<T, Tx>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		loop {
			if self.catcher.is_finalised() {
				break;
			}

			let mut skip_attempt = self.catcher.skip(7680);

			match Future::poll(Pin::new(&mut skip_attempt), cx) {
				Poll::Ready(0) => { break },
				Poll::Ready(n) => {},
				Poll::Pending => { return Poll::Pending; }
			}
		}

		self.catcher.pos = self.in_pos;

		Poll::Ready(self.catcher.new_handle())
	}
}

impl<T, Tx> AsyncRead for TxCatcher<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn poll_read(
	    mut self: Pin<&mut Self>,
	    cx: &mut Context,
	    buf: &mut [u8],
	) -> Poll<IoResult<usize>> {
		println!("Polling outer");
		self.core.read_from_pos(self.pos, cx, buf)
			.map(|(bytes_read, should_finalise_here)| {
				println!("Mapping outer");

				if should_finalise_here {
					let handle = self.core.clone();
					match self.core.finaliser() {
						Finaliser::InPlace => unreachable!(),
						Finaliser::NewThread => {
							std::thread::spawn(move || handle.do_finalise());
						},
						#[cfg(feature = "async-std-compat")]
						Finaliser::AsyncStd => {
							async_std::task::spawn(async move {
								handle.do_finalise();
							});
						},
						#[cfg(feature = "tokio-compat")]
						Finaliser::Tokio => {
							let _ = tokio::spawn(async move {
								handle.do_finalise();
							});
						},
						#[cfg(feature = "smol-compat")]
						Finaliser::Smol => {
							smol::Task::spawn(async move {
								handle.do_finalise();
							}).detach();
						},
					}
				}

				if let Ok(size) = bytes_read {
					self.pos += size;
				}

				bytes_read
			})
	}
}

#[cfg(feature = "tokio-compat")]
impl<T, Tx> TokioRead for TxCatcher<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn poll_read(
	    mut self: Pin<&mut Self>,
	    cx: &mut Context,
	    buf: &mut [u8],
	) -> Poll<IoResult<usize>> {
		AsyncRead::poll_read(self, cx, buf)
	}
}

impl<T, Tx> AsyncSeek for TxCatcher<T, Tx>
	where T: AsyncRead + Unpin + 'static,
		Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn poll_seek(
		mut self: Pin<&mut Self>,
		cx: &mut Context,
		pos: SeekFrom,
	) -> Poll<IoResult<u64>> {
		let old_pos = self.pos as u64;

		let (valid, new_pos) = match pos {
			SeekFrom::Current(adj) => {
				// overflow expected in many cases.
				let new_pos = old_pos.wrapping_add(adj as u64);
				(adj >= 0 || (adj.abs() as u64) <= old_pos, new_pos)
			}
			SeekFrom::End(adj) => {
				// Slower to load in the whole stream first, but safer.
				// We could, in theory, use metadata as the basis,
				// but incorrect metadata would be tricky to work around.
				let mut end_read_future = self.new_handle().load_all();
				if let Poll::Pending = Future::poll(Pin::new(&mut end_read_future), cx) {
					return Poll::Pending;
				}

				let len = self.core.len() as u64;
				let new_pos = len.wrapping_add(adj as u64);
				(adj >= 0 || (adj.abs() as u64) <= len, new_pos)
			}
			SeekFrom::Start(new_pos) => {
				(true, new_pos)
			}
		};

		Poll::Ready(if valid {
			if new_pos > old_pos {
				self.skip((new_pos - old_pos) as usize);
			}

			let len = self.core.len() as u64;

			self.pos = new_pos.min(len) as usize;
			Ok(self.pos as u64)
		} else {
			Err(IoError::new(IoErrorKind::InvalidInput, "Tried to seek before start of stream."))
		})
	}
}

// #[cfg(feature = "tokio-compat")]
// impl<T, Tx> TokioSeek for TxCatcher<T, Tx>
// 	where T: AsyncRead + Unpin + 'static,
// 		Tx: AsyncTransform<T> + Unpin + 'static,
// {
// 	fn poll_seek(
// 	    mut self: Pin<&mut Self>,
// 	    cx: &mut Context,
// 	    pos: SeekFrom,
// 	) -> Poll<IoResult<usize>> {
// 		AsyncSeek::poll_seek(self, cx, buf)
// 	}
// }

#[derive(Debug)]
pub(crate) struct SharedStore<T, Tx>
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin,
{
	raw: UnsafeCell<RawStore<T, Tx>>,
}

impl<T, Tx> SharedStore<T, Tx>
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin,
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

	fn read_from_pos(&self, pos: usize, cx: &mut Context, buffer: &mut [u8]) -> Poll<(IoResult<usize>, bool)> {
		self.get_mut_ref()
			.read_from_pos(pos, cx, buffer)
	}

	fn finaliser(&self) -> Finaliser {
		self.get_mut_ref()
			.config
			.spawn_finaliser
			.clone()
	}

	fn len(&self) -> usize {
		self.get_mut_ref()
			.len()
	}

	fn is_finalised(&self) -> bool {
		self.get_mut_ref()
			.finalised()
			.is_source_finished()
	}

	fn do_finalise(&self) {
		self.get_mut_ref()
			.do_finalise()
	}
}

// Shared basis for the below cache-based seekables.
#[derive(Debug)]
pub(crate) struct RawStore<T, Tx>
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin,
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
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin,
{
	fn new(source: T, transform: Tx, config: Option<Config>) -> Result<Self> {
		let config = config.unwrap_or_else(Default::default);

		let min_bytes = transform.min_bytes_required();

		if config.chunk_size < min_bytes {
			return Err(CatcherError::ChunkSize)
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
		let state_on_call: FinaliseState = self.finalised.compare_and_swap(
			FinaliseState::Live.into(),
			FinaliseState::Finalising.into(),
			Ordering::AcqRel
		).into();
		
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
			self.finalised.store(FinaliseState::Finalising.into(), Ordering::Release);
			return;
		}

		let backing_len = self.len();

		// Move the rope of bytes into the backing store.
		let rope = self.rope.as_mut()
			.expect("Writes should only occur while the rope exists.");

		if rope.len() > 1 {
			// Allocate one big store, then start moving entries over
			// chunk-by-chunk.
			let mut back = vec![0u8; backing_len];

			for el in rope.iter() {
				let start = el.start_pos;
				let end = el.end_pos;
				back[start..end]
					.copy_from_slice(&el.data[..end-start]);
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
		self.finalised.store(FinaliseState::Finalised.into(), Ordering::Release);
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
					let el = rope.pop_front().expect("Length of rope was established as >= 1.");
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
	fn read_from_pos(&mut self, pos: usize, cx: &mut Context, buf: &mut [u8]) -> Poll<(IoResult<usize>, bool)> {
		// Place read of finalised first to be certain that if we see finalised,
		// then backing_len *must* be the true length.
		let (loc, mut finalised) = self.get_location();

		println!("loc {:?} done {:?}", loc, finalised);

		let mut backing_len = self.len();

		let mut should_finalise_external = false;

		let target_len = pos + buf.len();

		let mut progress_before_pending = false;

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
					println!("Trying lock");
					let lock: *mut Mutex<()> = &mut self.lock;
					let mut guard = unsafe {
						let lock = & *lock;
						lock.lock()
					};

					if let Poll::Pending = Future::poll(Pin::new(&mut guard), cx) {
						break;
					}

					println!("Lock!");

					finalised = self.finalised();
					backing_len = self.len();

					// If length changed between our check and
					// acquiring the lock, then drop it -- we don't need new bytes *yet*
					// and might not!
					remaining_in_store = backing_len - pos - read;
					if remaining_in_store == 0 && finalised.is_source_live() {
						if let Poll::Ready(read_count) = self.fill_from_source(cx, buf.len() - read) {
							progress_before_pending = true;
							if let Ok((read_count, finalise_elsewhere)) = read_count {
								remaining_in_store += read_count;
								should_finalise_external |= finalise_elsewhere;
							}
							base_result = Some(read_count.map(|a| a.0));

							finalised = self.finalised();
						} else {
							break;
						}
					}

					// (Explicitly) unlocked here.
					mem::drop(guard);
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
					|| read == buf.len()
					|| (finalised.is_source_finished() && backing_len == pos + read) {
					break;
				}
			}

			base_result
				.unwrap_or(Ok(0))
				.map(|_| read)
		};

		if loc == CacheReadLocation::Roped {
			self.remove_rope_ref(finalised);
		}

		println!("end pbp {:?} out {:?} should:fin {:?}", progress_before_pending, out, should_finalise_external);

		if progress_before_pending {
			Poll::Ready((out, should_finalise_external))
		} else {
			Poll::Pending
		}
	}

	// ONLY SAFE TO CALL WITH LOCK.
	// The critical section concerns:
	// * adding new elements to the rope
	// * drawing bytes from the source
	// * modifying len
	// * modifying encoder state
	fn fill_from_source(&mut self, cx: &mut Context, mut bytes_needed: usize) -> Poll<IoResult<(usize, bool)>> {
		println!("From source");

		let minimum_to_write = self.transform.min_bytes_required();

		let overspill = bytes_needed % self.config.read_burst_len;
		if overspill != 0 {
			bytes_needed += self.config.read_burst_len - overspill;
		}

		let mut remaining_bytes = bytes_needed;
		let mut recorded_error = None;

		let mut spawn_new_finaliser = false;

		let mut progress_before_pending = false;

		loop {
			let rope = self.rope.as_mut()
				.expect("Writes should only occur while the rope exists.");

			let rope_el = rope.back_mut()
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
					self.config.chunk_size,
				));
			} else {
				rope_el.data.resize(new_len, 0);

				let src = self.source
					.as_mut()
					.expect("Source must exist while not finalised.");

				if let Poll::Ready(pos) = self.transform.transform_poll_read(Pin::new(src), cx, &mut rope_el.data[old_len..]) {
					progress_before_pending = true;

					println!("Transform gave goods {:?}.", pos);

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
						}
					}
				} else {
					// Pending
					println!("Transform gave pending.");
					break;
				}

				if self.finalised().is_source_finished() || remaining_bytes < minimum_to_write || recorded_error.is_some() {
					break;
				}
			}
		}

		if progress_before_pending {
			Poll::Ready(recorded_error.unwrap_or(Ok((bytes_needed - remaining_bytes, spawn_new_finaliser))))
		} else {
			Poll::Pending
		}
	}

	#[inline]
	fn read_from_local(&self, mut pos: usize, loc: CacheReadLocation, buf: &mut [u8], count: usize) -> usize {
		use CacheReadLocation::*;
		match loc {
			Backed => {
				let store = self.backing_store
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
				let rope = self.rope
					.as_ref()
					.expect("Rope should still exist while any handles hold a ::Roped(_) \
							 (and thus an Arc)");

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

						buf[written..next_len].copy_from_slice(&link.data[first_el..first_el + to_write]);

						written = next_len;
						pos += to_write;
					}

					if written >= buf.len() {
						break;
					}
				}

				count
			}
		}
	}
}

impl<T, Tx> Drop for RawStore<T, Tx>
	where T: AsyncRead + Unpin,
		Tx: AsyncTransform<T> + Unpin,
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
unsafe impl<T,Tx> Sync for SharedStore<T,Tx> where T: AsyncRead + Unpin, Tx: AsyncTransform<T> + Unpin {}
unsafe impl<T,Tx> Send for SharedStore<T,Tx> where T: AsyncRead + Unpin, Tx: AsyncTransform<T> + Unpin {}

#[async_trait]
pub trait AsyncReadSkipExt {
    async fn skip(&mut self, amt: usize) -> usize where Self: Sized;
}

#[async_trait]
impl<R: AsyncRead + Sized + Unpin + Send> AsyncReadSkipExt for R {
    async fn skip(&mut self, amt: usize) -> usize {
        io::copy(&mut self.take(amt as u64), &mut io::sink())
        	.await
        	.unwrap_or(0) as usize
    }
}

#[cfg(test)]
mod tests {
	#[test]
	fn it_works() {
		assert_eq!(2 + 2, 4);
	}
}
