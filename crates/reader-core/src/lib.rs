//! Everything the reader knows without a screen: a parsed book, the reader's appearance
//! settings, and the library that keeps both on disk. No Rune here, so it tests on its own.

pub mod book;
pub mod settings;

pub use book::{Block, Book, BookId, Chapter, Format, Position};
pub use settings::{FontChoice, LineSpacing, PageMode, PageTheme, Settings};
