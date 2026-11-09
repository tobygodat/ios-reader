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
- `scripts/fetch-fixtures.sh` downloads the real EPUB and PDF that `reader-core`'s tests read into `target/fixtures`. Run it before `cargo test`.
- `scripts/snap-scenes.sh` renders every scene into `docs/screenshots`.
- `scripts/mac-app.sh` builds release and writes an ad-hoc signed `Reader.app`, printing its path.
- `cargo run -p reader-core --example dump -- <file> [chapter]` prints what a book parses into.
- `scripts/ios-build.sh` builds the iOS library, generates `ios/Reader.xcodeproj` with xcodegen, builds the simulator app and prints its path.

## Finding books

- `reader_core::scan` walks folders for EPUBs and PDFs not yet in the library, reading each to hash it. Nothing is imported until chosen.
- The Mac searches `desktop_folders`. iOS can't read outside the sandbox, so the shell asks for a folder once and `ios/Reader/BookFolders.swift` remembers it as a bookmark in `book-folders.plist`. Later searches skip the picker. It holds each folder's security scope until the next search, since chosen books are read from it in place. The app's own Documents folder is always searched.
- iCloud files not yet downloaded are hidden placeholders on iOS and are skipped.

## Sync

- Optional. Signed out, nothing leaves the device. `crates/reader-app/src/account.rs` holds the account and when a sync runs; `reader_core::sync` holds the merge and the HTTP client, which uses the OS's TLS.
- Every device keeps a full local copy. Book text and covers move once, named by content hash. Each book's position carries `position_at`, and the later stamp wins. Books can't be deleted yet, so there are no tombstones.
- `server/` is the Cloudflare Worker at `reader-sync.tobygodat.workers.dev`: D1 for accounts and rows, KV for book text and covers (R2 isn't enabled on the account). Migrations are in `server/migrations`.
- Worker tests: `cd server && bun run dev`, then in another shell `bun test test/`. Add `READER_API=<url>` to run them against a deployed Worker, and delete the `@example.com` accounts they leave.
- Live client test: `READER_API=https://reader-sync.tobygodat.workers.dev cargo test -p reader-core --test sync_live`.
- The iOS shell has no keyboard for Rune fields, so sign-in there is a native alert. A new build with sign-in needs no demo account for Beta App Review only while sign-in stays optional.

## TestFlight

- `scripts/testflight.sh` archives in Release and uploads with the Apple ID signed in to Xcode. `scripts/testflight.sh export` stops at the signed `.ipa`. The build number is the commit count, so commit first.
- `ios/Reader/PrivacyInfo.xcprivacy` declares the required-reason APIs the Rust library imports (`stat` family, `mach_absolute_time`). After adding a dependency, check with `nm -u` on the app binary.
- `ITSAppUsesNonExemptEncryption` is `false`. `lopdf` carries AES and ChaCha20 to open encrypted PDFs, which is why this is a declaration to review before any public release.
- `scripts/make-icon.py` renders the iOS icon. The Mac icon still comes from `assets/icon.svg`.

## Build speed

- The root `Cargo.toml` builds the parsing, text layout and image crates at `opt-level = 2` even in debug. If a test or the debug app spends its time in another dependency, add that crate to the list. Don't switch to `"*"`: it doubles a cold build and the tests run no faster.
- This Mac runs cargo through sccache, set in `~/.cargo/config.toml`, not in the repo. A fresh worktree reuses most dependencies from it. Crates whose build scripts write to `OUT_DIR`, Rune among them, still rebuild in each worktree.
- `lld` can't link against the macOS 27 SDK yet, so keep Apple's default linker.

## iOS notes

- The Xcode project is generated. Edit `ios/project.yml`, not the `.xcodeproj`.
- `.cargo/config.toml` sets `IPHONEOS_DEPLOYMENT_TARGET=18.0` for every cargo run. Keep it there, not in scripts: the objc2 and cc-rs crates rebuild whenever it changes, which cost 17 to 20 s a build when a bare `cargo build` and the Xcode build disagreed. After changing its value, run `cargo clean -p blake3 --target <triple>`, since cc-rs doesn't notice.
- The Rune view draws its own controls, so the accessibility snapshot is empty. Drive the Simulator by coordinates in points.
- Files copied into the app's Documents by shell don't show in the picker's folder view right away. Find them with the picker's search field.

Rune is pre-release. Its rev is pinned in the root `Cargo.toml`. Read Rune's `crates/rune/README.md` at that rev for the API.
