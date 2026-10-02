# Fuzz targets

Install `cargo-fuzz` and a nightly Rust toolchain with sanitizer support, then run from the repository root:

```sh
cargo +nightly fuzz build
cargo +nightly fuzz run malformed_metadata
cargo +nightly fuzz run number_parser
cargo +nightly fuzz run normalization
cargo +nightly fuzz run rename_template
cargo +nightly fuzz run journal_parser
```

The fuzz crate is a separate workspace, so normal `cargo build` and `cargo test` do not compile libFuzzer. Target errors from invalid data are expected; a panic is a failure. Metadata inputs use the first byte to choose MP3, FLAC, or MP4 and the remaining bytes as the file body.
