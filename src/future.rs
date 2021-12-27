//! Support types for `AsyncRead`/`AsyncSeek` compatible stream buffers.
//! Requires the `"async"` feature.
use crate::*;
#[cfg(feature = "tokio-compat")]
pub use async_compat::Compat;
use async_trait::async_trait;
use core::{
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};
use futures_util::io::{self, AsyncRead, AsyncReadExt, AsyncSeek};
use std::{
	io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult, SeekFrom},
	marker::Unpin,
	mem,
	sync::atomic::Ordering,
};

/// Async variant of [`Transform`].
///
/// [`Transform`]: ../trait.Transform.html
pub trait AsyncTransform<TInput: AsyncRead> {
	fn transform_poll_read(
		&mut self,
		src: Pin<&mut TInput>,
		cx: &mut Context,
		buf: &mut [u8],
	) -> Poll<IoResult<TransformPosition>>;

	fn min_bytes_required(&self) -> usize {
		1
	}
}

impl<T: AsyncRead> AsyncTransform<T> for Identity {
	fn transform_poll_read(
		&mut self,
		src: Pin<&mut T>,
		cx: &mut Context,
		buf: &mut [u8],
	) -> Poll<IoResult<TransformPosition>> {
		src.poll_read(cx, buf).map(|res| {
			res.map(|count| match count {
				0 => TransformPosition::Finished,
				n => TransformPosition::Read(n),
			})
		})
	}
}

impl<T, Tx> TxCatcher<T, Tx>
where
	T: AsyncRead + Unpin + 'static,
	Tx: AsyncTransform<T> + Unpin + 'static,
{
	/// Read all bytes from the underlying stream
	/// into the backing store in the current task.
	pub fn load_all_async(self) -> LoadAll<T, Tx> {
		LoadAll::new(self)
	}
}

/// Future returned by [`TxCatcher::load_all_async`].
///
/// [`TxCatcher::load_all_async`]: ../struct.TxCatcher.html#method.load_all_async
pub struct LoadAll<T, Tx>
where
	T: AsyncRead + Unpin + 'static,
	Tx: AsyncTransform<T> + Unpin + 'static,
{
	catcher: TxCatcher<T, Tx>,
	in_pos: usize,
}

impl<T, Tx> LoadAll<T, Tx>
where
	T: AsyncRead + Unpin + 'static,
	Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn new(catcher: TxCatcher<T, Tx>) -> Self {
		let in_pos = catcher.pos;

		Self { catcher, in_pos }
	}
}

impl<T, Tx> Future for LoadAll<T, Tx>
where
	T: AsyncRead + Unpin + 'static,
	Tx: AsyncTransform<T> + Unpin + 'static,
{
	type Output = TxCatcher<T, Tx>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		self.catcher.pos = self.catcher.len();

		loop {
			if self.catcher.is_finalised() {
				break;
			}

			let mut skip_attempt = self.catcher.skip(7680);

			match Future::poll(Pin::new(&mut skip_attempt), cx) {
				Poll::Ready(0) => break,
				Poll::Ready(_n) => {},
				Poll::Pending => {
					return Poll::Pending;
				},
			}
		}

		self.catcher.pos = self.in_pos;

		Poll::Ready(self.catcher.new_handle())
	}
}

impl<T, Tx> AsyncRead for TxCatcher<T, Tx>
where
	T: AsyncRead + Unpin + 'static,
	Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context,
		buf: &mut [u8],
	) -> Poll<IoResult<usize>> {
		self.core.read_from_pos_async(self.pos, cx, buf).map(
			|(bytes_read, should_finalise_here)| {
				if should_finalise_here {
					let handle = self.core.clone();
					match self.core.config.spawn_finaliser {
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
							smol::spawn(async move {
								handle.do_finalise();
							})
							.detach();
						},
					}
				}

				if let Ok(size) = bytes_read {
					self.pos += size;
				}

				bytes_read
			},
		)
	}
}

#[cfg(feature = "tokio-compat")]
impl<T, Tx> TxCatcher<T, Tx> {
	pub fn tokio(self) -> Compat<Self> {
		Compat::new(self)
	}
}

impl<T, Tx> AsyncSeek for TxCatcher<T, Tx>
where
	T: AsyncRead + Unpin + 'static,
	Tx: AsyncTransform<T> + Unpin + 'static,
{
	fn poll_seek(mut self: Pin<&mut Self>, cx: &mut Context, pos: SeekFrom) -> Poll<IoResult<u64>> {
		let old_pos = self.pos as u64;

		let (valid, new_pos) = match pos {
			SeekFrom::Current(adj) => {
				// overflow expected in many cases.
				let new_pos = old_pos.wrapping_add(adj as u64);
				(adj >= 0 || (adj.abs() as u64) <= old_pos, new_pos)
			},
			SeekFrom::End(adj) => {
				// Slower to load in the whole stream first, but safer.
				// We could, in theory, use metadata as the basis,
				// but incorrect metadata would be tricky to work around.
				let mut end_read_future = self.new_handle().load_all_async();
				if Future::poll(Pin::new(&mut end_read_future), cx).is_pending() {
					return Poll::Pending;
				}

				let len = self.len() as u64;
				let new_pos = len.wrapping_add(adj as u64);
				(adj >= 0 || (adj.abs() as u64) <= len, new_pos)
			},
			SeekFrom::Start(new_pos) => (true, new_pos),
		};

		Poll::Ready(if valid {
			if new_pos > old_pos {
				self.pos = (new_pos as usize).min(self.len());
				if new_pos != self.pos as u64 {
					let mut skip_future = self.skip(new_pos as usize - self.pos);
					if Future::poll(Pin::new(&mut skip_future), cx).is_pending() {
						return Poll::Pending;
					}
				}
			}

			let len = self.len() as u64;

			self.pos = new_pos.min(len) as usize;
			Ok(self.pos as u64)
		} else {
			Err(IoError::new(
				IoErrorKind::InvalidInput,
				"Tried to seek before start of stream.",
			))
		})
	}
}

impl<T, Tx> RawStore<T, Tx>
where
	T: AsyncRead + Unpin,
	Tx: AsyncTransform<T> + Unpin,
{
	/// Returns read count, should_upgrade, should_finalise_external
	fn read_from_pos_async(
		&self,
		pos: usize,
		cx: &mut Context,
		buf: &mut [u8],
	) -> Poll<(IoResult<usize>, bool)> {
		// Place read of finalised first to be certain that if we see finalised,
		// then backing_len *must* be the true length.
		let (loc, mut finalised) = self.get_location();

		let mut backing_len = self.len();

		let mut should_finalise_external = false;

		let target_len = pos + buf.len();

		let mut progress_before_pending = false;

		let out = if finalised.is_source_finished() || target_len <= backing_len {
			// If finalised, there is zero risk of triggering more writes.
			progress_before_pending = true;
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
					let mut guard = self.lock.lock();

					if Future::poll(Pin::new(&mut guard), cx).is_pending() {
						break;
					}

					finalised = self.finalised();
					backing_len = self.len();

					// If length changed between our check and
					// acquiring the lock, then drop it -- we don't need new bytes *yet*
					// and might not!
					remaining_in_store = backing_len - pos - read;
					if remaining_in_store == 0 && finalised.is_source_live() {
						if let Poll::Ready(read_count) =
							self.fill_from_source_async(cx, buf.len() - read)
						{
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
					read += self.read_from_local(pos + read, loc, &mut buf[read..], count);
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
	fn fill_from_source_async(
		&self,
		cx: &mut Context,
		mut bytes_needed: usize,
	) -> Poll<IoResult<(usize, bool)>> {
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

		let mut progress_before_pending = false;

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

					let poll = self.transform.with_mut(|tx_ptr| {
						self.source.with_mut(|src_ptr| {
							let src = unsafe { &mut *src_ptr }
								.as_mut()
								.expect("Source must exist while not finalised.");

							unsafe { &mut *tx_ptr }.transform_poll_read(
								Pin::new(src),
								cx,
								&mut rope_el.data[old_len..],
							)
						})
					});

					if let Poll::Ready(pos) = poll {
						progress_before_pending = true;

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
					} else {
						// Pending
						true
					}
				}
			});

			if should_break {
				break;
			}
		}

		if progress_before_pending {
			Poll::Ready(
				recorded_error.unwrap_or(Ok((bytes_needed - remaining_bytes, spawn_new_finaliser))),
			)
		} else {
			Poll::Pending
		}
	}
}

#[async_trait]
/// Async variant of [`ReadSkipExt`].
///
/// [`ReadSkipExt`]: ../trait.ReadSkipExt.html
pub trait AsyncReadSkipExt {
	async fn skip(&mut self, amt: usize) -> usize
	where
		Self: Sized;
}

#[async_trait]
impl<R: AsyncRead + Sized + Unpin + Send> AsyncReadSkipExt for R {
	async fn skip(&mut self, amt: usize) -> usize {
		io::copy(&mut self.take(amt as u64), &mut io::sink())
			.await
			.unwrap_or(0) as usize
	}
}
