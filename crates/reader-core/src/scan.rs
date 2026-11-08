//! Looking through folders for books the library doesn't have yet. It only reads: nothing is
//! imported until the reader chooses it.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::book::{BookId, Format};
use crate::{epub, import};

/// How deep to follow folders, and how many books to report, so a huge disk still finishes.
const MAX_DEPTH: usize = 10;
const MAX_FOUND: usize = 1000;
/// Larger files are skipped: reading one to hash it would stall the search.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Folders that hold app data or build output rather than a person's books.
const SKIPPED: &[&str] = &["node_modules", "target", "build", "Library", "Applications", "DerivedData", "vendor", "__pycache__"];
const SKIPPED_BUNDLES: &[&str] = &["app", "photoslibrary", "xcarchive", "framework", "bundle", "musiclibrary", "tvlibrary"];

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub path: PathBuf,
    pub id: BookId,
    pub title: String,
    pub author: Option<String>,
    pub format: Format,
    pub bytes: u64,
    /// The name of the folder it's in, to tell two copies or similar names apart.
    pub folder: String,
}

/// Finds the EPUBs and PDFs under `roots` that aren't in `known`, one entry per distinct file,
/// EPUBs first and then by title. Folders that can't be read are passed over.
pub fn scan(roots: &[PathBuf], known: &HashSet<BookId>) -> Vec<Found> {
    let mut seen = known.clone();
    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if entry.file_name().to_string_lossy().starts_with('.') || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth < MAX_DEPTH && !skipped_folder(&path) {
                    stack.push((path, depth + 1));
                }
            } else if found.len() < MAX_FOUND
                && let Some(book) = inspect(&path)
                && seen.insert(book.id.clone())
            {
                found.push(book);
            }
        }
    }
    found.sort_by_cached_key(|b| (b.format != Format::Epub, b.title.to_lowercase()));
    found
}

fn skipped_folder(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let bundle = path.extension().is_some_and(|e| SKIPPED_BUNDLES.contains(&e.to_string_lossy().to_lowercase().as_str()));
    bundle || SKIPPED.contains(&name.as_ref())
}

fn inspect(path: &Path) -> Option<Found> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    let format = match extension.as_str() {
        "epub" => Format::Epub,
        "pdf" => Format::Pdf,
        _ => return None,
    };
    let bytes = fs::metadata(path).ok()?.len();
    if bytes > MAX_BYTES {
        return None;
    }
    let contents = fs::read(path).ok()?;
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let (title, author) = match format {
        Format::Epub => {
            let (title, author) = epub::metadata(&contents)?;
            (title.unwrap_or(name), author)
        }
        Format::Pdf => {
            if !contents.starts_with(b"%PDF") {
                return None;
            }
            (name, None)
        }
    };
    let folder = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Some(Found { path: path.to_path_buf(), id: import::book_id(&contents), title, author, format, bytes, folder })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn sorted_titles(found: &[Found]) -> Vec<&str> {
        found.iter().map(|b| b.title.as_str()).collect()
    }

    #[test]
    fn finds_books_in_nested_folders_with_epubs_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("reports/invoice.pdf"), b"%PDF-1.7 invoice");
        write(&root.join("novels/deep/small.epub"), &epub::tests::sample());
        write(&root.join("notes.txt"), b"not a book");

        let found = scan(&[root.to_path_buf()], &HashSet::new());
        assert_eq!(sorted_titles(&found), ["A Small Book", "invoice"]);
        assert_eq!(found[0].format, Format::Epub);
        assert_eq!(found[0].folder, "deep");
        assert_eq!(found[1].format, Format::Pdf);
    }

    #[test]
    fn leaves_out_books_already_known_and_second_copies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a/same.pdf"), b"%PDF same");
        write(&root.join("b/copy of same.pdf"), b"%PDF same");
        write(&root.join("c/owned.pdf"), b"%PDF owned");

        let known = HashSet::from([import::book_id(b"%PDF owned")]);
        let found = scan(&[root.to_path_buf()], &known);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].id, import::book_id(b"%PDF same"));
    }

    #[test]
    fn skips_hidden_folders_app_bundles_and_files_that_only_look_like_books() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join(".cache/hidden.pdf"), b"%PDF hidden");
        write(&root.join("Some.app/Contents/help.pdf"), b"%PDF bundled");
        write(&root.join("node_modules/pkg/docs.pdf"), b"%PDF dependency");
        write(&root.join("fake.pdf"), b"not a pdf");
        write(&root.join("fake.epub"), b"not a zip");

        assert!(scan(&[root.to_path_buf()], &HashSet::new()).is_empty());
    }

    #[test]
    fn a_missing_root_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("book.pdf"), b"%PDF ok");
        let found = scan(&[dir.path().join("nowhere"), dir.path().to_path_buf()], &HashSet::new());
        assert_eq!(found.len(), 1);
    }
}
