# Streamcatcher
A Rust thread-safe, shared (asynchronous) stream buffer designed to lock only on accessing and storing new data.

Streamcatcher is designed to allow seeking on otherwise one-way streams (*e.g.*, command output)
whose output needs to be accessed by many threads without constant reallocations,
contention over safe read-only data, or unnecessary stalling. Only threads who read in
*new data* ever need to lock the data structure, and do not prevent earlier reads from occurring.

## Features
* Lockless access to pre-read data and finished streams.
* Transparent caching of newly read data.
* Allows seeking on read-only bytestreams.
* Piecewise allocation to reduce copying and support unknown input lengths.
* Optional acceleration of reads on stream completion by copying to a single backing store.
* (Stateful) bytestream transformations.

The main algorithm is outlined in [this blog post], with rope
reference tracking moved to occur only in the core.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

[this blog post]: https://mcfelix.me/blog/shared-buffers/
