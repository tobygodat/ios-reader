//! The books on disk, in the data dir:
//! - `library.json`: one entry per book (id, title, author, format, position, added time).
//! - `books/<id>.json`: the parsed book. Named by the hash of the file it came from, so
//!   importing the same file again reuses it, and two imports writing it at once write the same.
//! - `settings.json`: the appearance settings.
//!
//! Every write goes to a temporary file first and is renamed into place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::book::{Book, BookId, Format, Position};
use crate::import::{self, ImportError};
use crate::settings::Settings;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: BookId,
    pub title: String,
    pub author: Option<String>,
    pub format: Format,
    #[serde(default)]
    pub position: Position,
    /// Seconds since the Unix epoch.
    pub added: u64,
    /// How many chapters the book has, for the progress line on its cover.
    #[serde(default)]
    pub chapters: usize,
}

impl Entry {
    /// How far through the whole book, in `0.0..=1.0`, counting chapters as equal.
    pub fn progress(&self) -> f32 {
        if self.chapters == 0 {
            return 0.0;
        }
        ((self.position.chapter as f32 + self.position.fraction) / self.chapters as f32).clamp(0.0, 1.0)
    }
}

#[derive(Debug)]
pub struct Library {
    dir: PathBuf,
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Index {
    books: Vec<Entry>,
}

impl Library {
    /// Opens the library in `dir`, creating it if needed. A missing or damaged index reads as
    /// an empty library rather than failing the launch.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Library> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("books"))?;
        let entries = fs::read(dir.join("library.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
            .map(|i| i.books)
            .unwrap_or_default();
        Ok(Library { dir, entries })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, id: &BookId) -> Option<&Entry> {
        self.entries.iter().find(|e| &e.id == id)
    }

    /// Records a book [`prepare`] wrote. The same book again keeps its entry and position.
    pub fn add(&mut self, book: &Prepared) -> io::Result<&Entry> {
        let at = match self.entries.iter().position(|e| e.id == book.id) {
            Some(at) => at,
            None => {
                let entry = Entry {
                    id: book.id.clone(),
                    title: book.title.clone(),
                    author: book.author.clone(),
                    format: book.format,
                    position: Position::default(),
                    added: now(),
                    chapters: book.chapters,
                };
                self.entries.insert(0, entry);
                self.save()?;
                0
            }
        };
        Ok(&self.entries[at])
    }

    /// Stores an already parsed book and records it, as an import of its file would.
    pub fn insert(&mut self, book: &Book) -> io::Result<&Entry> {
        store(&self.dir, book)?;
        self.add(&Prepared::of(book))
    }

    pub fn load(&self, id: &BookId) -> io::Result<Book> {
        let bytes = fs::read(book_path(&self.dir, id))?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }

    pub fn set_position(&mut self, id: &BookId, position: Position) -> io::Result<()> {
        let Some(entry) = self.entries.iter_mut().find(|e| &e.id == id) else { return Ok(()) };
        if entry.position == position {
            return Ok(());
        }
        entry.position = position;
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let index = Index { books: self.entries.clone() };
        write_atomic(&self.dir.join("library.json"), &serde_json::to_vec_pretty(&index).map_err(io::Error::other)?)
    }

    pub fn load_settings(&self) -> Settings {
        fs::read(self.dir.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        write_atomic(&self.dir.join("settings.json"), &serde_json::to_vec_pretty(settings).map_err(io::Error::other)?)
    }
}

/// What an import leaves for the library to record: the book is already on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub id: BookId,
    pub title: String,
    pub author: Option<String>,
    pub format: Format,
    pub chapters: usize,
}

/// Reads, parses and stores the book at `path` under `dir`, without touching the index, so it
/// runs on any thread. A book already stored is read back instead of parsed again.
pub fn prepare(dir: &Path, path: &Path) -> Result<Prepared, ImportError> {
    let bytes = fs::read(path).map_err(|e| ImportError::Unreadable(e.to_string().to_lowercase()))?;
    let id = import::book_id(&bytes);
    let stored = book_path(dir, &id);
    let book = match fs::read(&stored).ok().and_then(|b| serde_json::from_slice::<Book>(&b).ok()) {
        Some(book) => book,
        None => {
            let book = import::parse(path, &bytes)?;
            store(dir, &book).map_err(|e| ImportError::Unreadable(e.to_string().to_lowercase()))?;
            book
        }
    };
    Ok(Prepared::of(&book))
}

impl Prepared {
    fn of(book: &Book) -> Prepared {
        Prepared { id: book.id.clone(), title: book.title.clone(), author: book.author.clone(), format: book.format, chapters: book.chapters.len() }
    }
}

fn store(dir: &Path, book: &Book) -> io::Result<()> {
    fs::create_dir_all(dir.join("books"))?;
    write_atomic(&book_path(dir, &book.id), &serde_json::to_vec(book).map_err(io::Error::other)?)
}

fn book_path(dir: &Path, id: &BookId) -> PathBuf {
    dir.join("books").join(format!("{}.json", id.0))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Writes `bytes` to a sibling temporary file and renames it over `path`, so a crash leaves
/// either the old file or the new one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importing_twice_reuses_the_entry_and_its_position() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("small.epub");
        fs::write(&file, crate::epub::tests::sample()).unwrap();

        let mut lib = Library::open(dir.path().join("data")).unwrap();
        let first = prepare(lib.dir(), &file).unwrap();
        let id = lib.add(&first).unwrap().id.clone();
        lib.set_position(&id, Position { chapter: 1, fraction: 0.5 }).unwrap();

        let again = prepare(lib.dir(), &file).unwrap();
        assert_eq!(again, first);
        lib.add(&again).unwrap();
        assert_eq!(lib.entries().len(), 1);

        let reopened = Library::open(dir.path().join("data")).unwrap();
        assert_eq!(reopened.entries()[0].position, Position { chapter: 1, fraction: 0.5 });
        assert_eq!(reopened.entries()[0].title, "A Small Book");
        assert_eq!(reopened.load(&id).unwrap().chapters.len(), 3);
        assert!((reopened.entries()[0].progress() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn settings_round_trip_and_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.load_settings(), Settings::default());
        let s = Settings { font_size: 22.0, theme: crate::PageTheme::Dusk, ..Settings::default() };
        lib.save_settings(&s).unwrap();
        assert_eq!(lib.load_settings(), s);
        let leftovers: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn a_damaged_index_opens_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("library.json"), "{ not json").unwrap();
        assert!(Library::open(dir.path()).unwrap().entries().is_empty());
    }

    #[test]
    fn a_failed_import_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "hi").unwrap();
        let lib = Library::open(dir.path().join("data")).unwrap();
        assert!(matches!(prepare(lib.dir(), &file), Err(ImportError::Unsupported(_))));
        assert_eq!(fs::read_dir(lib.dir().join("books")).unwrap().count(), 0);
    }
}
