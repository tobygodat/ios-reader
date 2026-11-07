# Reader

A minimalist EPUB and PDF reader for Mac and iPhone, in Rust on [Rune](https://github.com/anomalyco/rune).
Work is tracked in Linear issue TOB-402.

## Layout

- `crates/reader-core` holds the book model, EPUB and PDF parsing, settings and the on-disk library. No Rune.
- `crates/reader-app` holds the Rune UI both platforms run. Its `lib.rs` doc lists the host contract.
- `crates/reader-mac` is the Mac binary (`reader`).
- `crates/reader-ios` is a static library with the C ABI in `ios/Reader/ReaderCore.h`.
- `ios/` is the Swift shell that hosts the Rune view on iPhone.

## Commands

Homebrew's rustup keeps its proxies off `PATH`. Run `export PATH=/opt/homebrew/opt/rustup/bin:$PATH` first.
`rust-toolchain.toml` pins Rust 1.99.1 to match Rune.

- `cargo test -p reader-core`
- `cargo run -p reader-mac` opens the Mac app.
- `cargo run -p reader-mac -- --snap out.png --scene <name>` renders a scene headless to PNG (Rune's agent tooling).
- `cargo build -p reader-ios --target aarch64-apple-ios-sim` builds the iOS library.

Rune is pre-release. Its rev is pinned in the root `Cargo.toml`. Read Rune's `crates/rune/README.md` at that rev for the API.
