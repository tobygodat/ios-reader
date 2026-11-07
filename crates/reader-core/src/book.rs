use serde::{Deserialize, Serialize};

/// A book's identity: a hex digest of the imported file's bytes, so importing the same file
/// twice finds the existing book.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BookId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Epub,
    Pdf,
}

/// One imported book, reduced to text the reader reflows. EPUB and PDF both parse into this;
/// nothing downstream knows which it came from except `format`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: BookId,
    pub title: String,
    pub author: Option<String>,
    pub format: Format,
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: Option<String>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph(String),
    Quote(String),
    /// A scene break: `<hr>`, or a line of only `* * *`.
    Break,
}

/// Where the reader left off: a chapter, and how far through it as a fraction in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub chapter: usize,
    pub fraction: f32,
}
