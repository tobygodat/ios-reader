//! Named states for headless snapshots (`--scene`). Each builds a fresh library in the temp dir,
//! so a snapshot never reads or writes the real one.

use reader_core::{Block, Book, BookId, Chapter, Format, Library, PageMode, PageTheme, Position, Settings};

use crate::{Host, Import, Panel, State};

type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("empty-library", |s| fresh(s, "empty")),
    ("library", |s| shelf(s, "library")),
    ("import-error", |s| {
        shelf(s, "import-error");
        s.import = Import::Failed(reader_core::ImportError::Scanned.to_string());
    }),
    ("importing", |s| {
        shelf(s, "importing");
        s.import = Import::Parsing { name: "Middlemarch.epub".into(), queued: 0 };
    }),
    ("reading-paper", |s| reading(s, PageTheme::Paper)),
    ("reading-sepia", |s| reading(s, PageTheme::Sepia)),
    ("reading-sage", |s| reading(s, PageTheme::Sage)),
    ("reading-dusk", |s| reading(s, PageTheme::Dusk)),
    ("reading-night", |s| reading(s, PageTheme::Night)),
    ("appearance", |s| {
        reading(s, PageTheme::Paper);
        s.set_panel(Some(Panel::Appearance));
    }),
    ("appearance-dusk", |s| {
        reading(s, PageTheme::Dusk);
        s.settings.font = reader_core::FontChoice::Sans;
        s.set_panel(Some(Panel::Appearance));
    }),
    ("pages", |s| {
        shelf(s, "pages");
        s.settings.mode = PageMode::Paged;
        let id = pride().id;
        s.library.set_position(&id, Position { chapter: 1, fraction: 0.3 }).expect("scene library is writable");
        s.open_book(&id);
        hide_chrome(s);
    }),
];

const PRIDE: &str = include_str!("../assets/sample-pride.json");

pub fn pride() -> Book {
    serde_json::from_str(PRIDE).expect("the bundled sample book parses")
}

fn short(id: &str, title: &str, author: &str, opening: &str) -> Book {
    Book {
        id: BookId(id.into()),
        title: title.into(),
        author: Some(author.into()),
        format: Format::Epub,
        chapters: vec![Chapter { title: Some("Chapter 1".into()), blocks: vec![Block::Paragraph(opening.into())] }],
    }
}

fn fresh(s: &mut State, name: &str) {
    let dir = std::env::temp_dir().join(format!("reader-scene-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.library = Library::open(&dir).expect("the temp dir is writable");
    s.settings = Settings::default();
    s.host = Host::Desktop;
    s.import = Import::Idle;
}

fn shelf(s: &mut State, name: &str) {
    fresh(s, name);
    let books = [
        short(
            "sample-dorian",
            "The Picture of Dorian Gray",
            "Oscar Wilde",
            "The studio was filled with the rich odour of roses, and when the light summer wind stirred amidst the trees of the garden, there came through the open door the heavy scent of the lilac, or the more delicate perfume of the pink-flowering thorn.",
        ),
        short(
            "sample-frankenstein",
            "Frankenstein",
            "Mary Wollstonecraft Shelley",
            "You will rejoice to hear that no disaster has accompanied the commencement of an enterprise which you have regarded with such evil forebodings.",
        ),
        short(
            "sample-moby",
            "Moby-Dick; or, The Whale",
            "Herman Melville",
            "Call me Ishmael. Some years ago—never mind how long precisely—having little or no money in my purse, and nothing particular to interest me on shore, I thought I would sail about a little and see the watery part of the world.",
        ),
        pride(),
    ];
    for b in &books {
        s.library.insert(b).expect("scene library is writable");
    }
    let set = |s: &mut State, id: &str, chapter, fraction| {
        s.library.set_position(&BookId(id.into()), Position { chapter, fraction }).expect("scene library is writable");
    };
    set(s, "sample-moby", 0, 0.35);
    set(s, "sample-pride-and-prejudice", 1, 0.4);
}

fn reading(s: &mut State, theme: PageTheme) {
    shelf(s, "reading");
    s.settings.theme = theme;
    let id = pride().id;
    s.open_book(&id);
}

fn hide_chrome(s: &mut State) {
    if let crate::Screen::Reading(r) = &mut s.screen {
        r.chrome = false;
    }
}
