[![docs-badge][]][docs] [![crates.io version][]][crates.io link] [![crates.io downloads][]][crates.io link] [![license][]][license link] [![build badge]][build]

# Streamcatcher
A Rust thread-safe, shared (asynchronous) stream buffer designed to lock only on accessing and storing new data.

Streamcatcher is designed to allow seeking on otherwise one-way streams (*e.g.*, command output) whose output needs to be accessed by many threads without constant reallocations, contention over safe read-only data, or unnecessary stalling. Only threads who read in *new data* ever need to lock the data structure, and do not prevent earlier reads from occurring.

## Features
* Lockless access to pre-read data and finished streams.
* Transparent caching of newly read data.
* Allows seeking on read-only bytestreams.
* Piecewise allocation to reduce copying and support unknown input lengths.
* Optional acceleration of reads on stream completion by copying to a single backing store.
* (Stateful) bytestream transformations.
* Async support with the `"async"` feature, and runtimes via [`"async-std-compat"`, `"smol-compat"`, `"tokio-compat"`].

The main algorithm is outlined in [this blog post], with rope reference tracking moved to occur only in the core.

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

Detailed guidelines are given in [the CONTRIBUTING file].

[this blog post]: https://mcfelix.me/blog/shared-buffers/
[the CONTRIBUTING file]: CONTRIBUTING.md

[build badge]: https://img.shields.io/github/workflow/status/FelixMcFelix/streamcatcher/Build%20and%20Test%20(Stable)?style=flat-square
[build]: https://github.com/FelixMcFelix/streamcatcher/actions

[docs-badge]: https://img.shields.io/badge/docs-online-4d76ae.svg?style=flat-square
[docs]: https://docs.rs/streamcatcher

[crates.io link]: https://crates.io/crates/streamcatcher
[crates.io version]: https://img.shields.io/crates/v/streamcatcher.svg?style=flat-square
[crates.io downloads]: https://img.shields.io/crates/d/streamcatcher.svg?style=flat-square

[license]: https://img.shields.io/crates/l/streamcatcher?style=flat-square
[license link]: https://opensource.org/licenses/Apache-2.0