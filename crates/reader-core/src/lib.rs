//! Everything the reader knows without a screen: a parsed book, the reader's appearance
//! settings, and the library that keeps both on disk. No Rune here, so it tests on its own.

pub mod book;
pub(crate) mod epub;
pub mod import;
pub mod library;
mod pdf;
pub mod scan;
pub mod settings;
pub mod sync;

pub use book::{Block, Book, BookId, Chapter, Format, Position};
pub use import::ImportError;
pub use library::{Entry, Library, Prepared, prepare};
pub use settings::{FontChoice, LineSpacing, PageMode, PageTheme, Settings};
