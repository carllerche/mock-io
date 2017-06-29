# Mock out Rust I/O types

A Rust utility library to help test I/O code. `mock-io` provides a type
implementing `Read` and `Write` that can be configured to follow a predefined
script and panic otherwise.

## Quick start

Add this to your `Cargo.toml`:

```toml
[dev-dependencies]
mock-io = { git = "https://github.com/carllerche/mock-io" }
```

Next, add this to your tests:

```rust
extern crate mock_io;
```

Now you can use `mock-io` in your tests.

## Tokio integration

`mock-io` provides Tokio support by default. This can be disabled by ommitting
the `tokio` feature.

## License

`mock-io` is primarily distributed under the terms of both the MIT license and
the Apache License (Version 2.0), with portions covered by various BSD-like
licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.
