//! The reader's Rune UI, shared by the Mac binary and the iOS static library.
//!
//! Host contract (the Mac and iOS crates call only these):
//! - [`configure`] once, before [`spec`], with where to keep the library and who picks files.
//! - [`spec`] builds the `AppSpec` both hosts run.
//! - [`take_request`] after each frame: something only the host can do (the iOS document picker).
//! - [`import`] with a file the host picked.
//! - [`set_insets`] with the screen's safe-area insets (iOS; Rune reports zero on native).

use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use reader_core::{Book, BookId, ImportError, Library, PageMode, Position, Prepared, Settings};
use rune::{AppSpec, Key, KeyEvent, LiveText, ScrollPos, Tx, Waker};

mod look;
mod paginate;
mod samples;
mod view;

pub use look::{Palette, Type, display_title};

/// Who answers "pick a book": Rune's own file dialog on the Mac, the host app on iOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Desktop,
    Ios,
}

/// Work the view asks the host to do, drained by [`take_request`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostRequest {
    PickBook,
}

struct Config {
    data_dir: PathBuf,
    host: Host,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

pub fn configure(data_dir: PathBuf, host: Host) {
    let _ = CONFIG.set(Config { data_dir, host });
}

pub const FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/Literata.ttf"),
    include_bytes!("../../../assets/fonts/Literata-Italic.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono.ttf"),
];
pub const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

/// Which screen shows. The library is home; reading holds everything about the open book.
pub enum Screen {
    Library,
    Reading(Box<Reading>),
}

/// An import's progress, as the library shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Import {
    Idle,
    /// `name` is the file being parsed; `queued` more wait behind it.
    Parsing { name: String, queued: usize },
    /// Shown inline until the next import or a dismiss.
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Appearance,
}

pub struct Reading {
    pub id: BookId,
    pub book: Rc<Book>,
    pub chapter: usize,
    /// The top bar shows; it fades while reading and returns on a tap mid-page.
    pub chrome: bool,
    pub panel: Option<Panel>,
    pub flow: Flow,
    /// The page area's size, as laid out.
    pub page_box: [f32; 2],
    saved: Position,
    dirty_since: Option<f64>,
}

/// How the open chapter moves, by page mode.
pub enum Flow {
    Scroll(Scroll),
    Paged(Paged),
}

pub struct Scroll {
    /// Where the viewport is, written by its scroll handler every frame it moves.
    at: Rc<Cell<Option<ScrollPos>>>,
    /// A fraction to scroll to once the chapter has been laid out.
    restore: Option<f32>,
    /// The offset when the chrome last appeared; scrolling away from it hides the chrome.
    /// `None` takes the next offset the viewport reports.
    chrome_at: Option<f32>,
    /// "Chapter II · 42%", updated without re-running the view.
    progress: LiveText,
}

pub struct Paged {
    pub pages: Vec<paginate::Page>,
    key: Option<Box<paginate::Key>>,
    pub page: usize,
    /// Where in the chapter to land when pages are next computed.
    want: f32,
    /// A swipe's travel, so the page follows the finger.
    pub drag: f32,
}

impl Flow {
    fn new(mode: PageMode, fraction: f32) -> Flow {
        match mode {
            PageMode::Scroll => Flow::Scroll(Scroll { at: Rc::default(), restore: Some(fraction), chrome_at: None, progress: LiveText::new("") }),
            PageMode::Paged => Flow::Paged(Paged { pages: Vec::new(), key: None, page: 0, want: fraction, drag: 0.0 }),
        }
    }

    pub fn fraction(&self) -> f32 {
        match self {
            Flow::Scroll(s) => match (s.restore, s.at.get()) {
                (Some(f), _) => f,
                (None, Some(at)) if at.max[1] > 0.0 => (at.offset[1] / at.max[1]).clamp(0.0, 1.0),
                _ => 0.0,
            },
            Flow::Paged(p) if p.pages.is_empty() => p.want,
            Flow::Paged(p) => p.page as f32 / p.pages.len() as f32,
        }
    }
}

impl Reading {
    pub fn position(&self) -> Position {
        Position { chapter: self.chapter, fraction: self.flow.fraction() }
    }

    pub fn chapter_label(&self) -> String {
        let n = self.chapter + 1;
        match self.book.chapters[self.chapter].title.as_deref().map(display_title) {
            Some(t) if !t.is_empty() && t.chars().count() <= 28 => t,
            _ => format!("Chapter {n}"),
        }
    }
}

type Inbox = Arc<Mutex<Vec<Result<Prepared, ImportError>>>>;

pub struct State {
    pub host: Host,
    pub library: Library,
    pub settings: Settings,
    pub screen: Screen,
    pub import: Import,
    /// Top, right, bottom, left, in points.
    pub insets: [f32; 4],
    /// The window's size, as laid out.
    pub window: [f32; 2],
    pub drop_hover: bool,
    requests: VecDeque<HostRequest>,
    inbox: Inbox,
    waker: Option<Waker>,
}

const DEFAULT_SIZE: (f32, f32) = (1100.0, 760.0);

impl State {
    /// The app over the library in `dir`. A library that can't be opened falls back to an
    /// empty one in the temp dir, with the reason shown, so the window still opens.
    pub fn open(dir: &Path, host: Host) -> State {
        let (library, import) = match Library::open(dir) {
            Ok(lib) => (lib, Import::Idle),
            Err(e) => {
                let fallback = std::env::temp_dir().join("reader-fallback");
                let lib = Library::open(&fallback).expect("the temp dir is writable");
                (lib, Import::Failed(format!("Your library at {} couldn’t be opened: {e}.", dir.display())))
            }
        };
        State {
            host,
            settings: library.load_settings(),
            library,
            screen: Screen::Library,
            import,
            insets: [0.0; 4],
            window: [DEFAULT_SIZE.0, DEFAULT_SIZE.1],
            drop_hover: false,
            requests: VecDeque::new(),
            inbox: Inbox::default(),
            waker: None,
        }
    }

    pub fn reading(&self) -> Option<&Reading> {
        match &self.screen {
            Screen::Reading(r) => Some(r),
            Screen::Library => None,
        }
    }

    fn reading_mut(&mut self) -> Option<&mut Reading> {
        match &mut self.screen {
            Screen::Reading(r) => Some(r),
            Screen::Library => None,
        }
    }

    /// Asks for a book: Rune's file dialog on the desktop, the host's picker on iOS.
    pub fn ask_for_book(&mut self, tx: &mut Tx) {
        match self.host {
            Host::Ios => self.requests.push_back(HostRequest::PickBook),
            Host::Desktop => {
                let options = rune::FileDialogOptions { title: "Import a book".into(), ..Default::default() }.filter("Books", &["epub", "pdf"]);
                tx.open_file(options, |s: &mut State, result, _tx| match result {
                    Ok(Some(path)) => s.start_import(path),
                    Ok(None) => {}
                    Err(e) => s.import = Import::Failed(format!("The file dialog couldn’t open: {e}.")),
                });
            }
        }
    }

    /// Parses `path` on a worker thread; the result lands in [`poll`].
    pub fn start_import(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.import = match &self.import {
            Import::Parsing { name: current, queued } => Import::Parsing { name: current.clone(), queued: queued + 1 },
            _ => Import::Parsing { name, queued: 0 },
        };
        let dir = self.library.dir().to_path_buf();
        let inbox = self.inbox.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = reader_core::prepare(&dir, &path);
            inbox.lock().expect("import inbox lock").push(result);
            if let Some(wake) = waker {
                wake();
            }
        });
    }

    fn take_imports(&mut self) -> bool {
        let done: Vec<_> = std::mem::take(&mut *self.inbox.lock().expect("import inbox lock"));
        if done.is_empty() {
            return false;
        }
        for result in done {
            let failed = match result {
                Ok(book) => self.library.add(&book).err().map(|e| format!("The book couldn’t be saved: {e}.")),
                Err(e) => Some(e.to_string()),
            };
            self.import = match (&self.import, failed) {
                (_, Some(message)) => Import::Failed(message),
                (Import::Parsing { queued, .. }, None) if *queued > 0 => Import::Parsing { name: "the next book".into(), queued: queued - 1 },
                (Import::Failed(m), None) => Import::Failed(m.clone()),
                _ => Import::Idle,
            };
        }
        true
    }

    pub fn open_book(&mut self, id: &BookId) {
        let Some(entry) = self.library.entry(id).cloned() else { return };
        let book = match self.library.load(id) {
            Ok(book) if !book.chapters.is_empty() => book,
            Ok(_) => return self.import = Import::Failed(format!("“{}” has nothing to read.", entry.title)),
            Err(e) => return self.import = Import::Failed(format!("“{}” couldn’t be opened: {e}.", entry.title)),
        };
        let chapter = entry.position.chapter.min(book.chapters.len() - 1);
        self.screen = Screen::Reading(Box::new(Reading {
            id: id.clone(),
            book: Rc::new(book),
            chapter,
            chrome: true,
            panel: None,
            flow: Flow::new(self.settings.mode, entry.position.fraction),
            page_box: [0.0; 2],
            saved: entry.position,
            dirty_since: None,
        }));
    }

    pub fn close_book(&mut self) {
        self.save_position();
        self.screen = Screen::Library;
    }

    fn save_position(&mut self) {
        let Screen::Reading(r) = &mut self.screen else { return };
        let position = r.position();
        r.dirty_since = None;
        if position != r.saved && self.library.set_position(&r.id, position).is_ok() {
            r.saved = position;
        }
    }

    /// Moves to chapter `to`, at its start, or at its last page when `at_end`.
    pub fn go_to_chapter(&mut self, to: usize, at_end: bool) {
        let mode = self.settings.mode;
        let Some(r) = self.reading_mut() else { return };
        if to >= r.book.chapters.len() {
            return;
        }
        r.chapter = to;
        r.flow = Flow::new(mode, if at_end { 1.0 } else { 0.0 });
        self.save_position();
    }

    /// Turns `by` pages in paged mode, across chapter ends.
    pub fn turn(&mut self, by: i32) {
        let Some(r) = self.reading_mut() else { return };
        let (chapter, chapters) = (r.chapter, r.book.chapters.len());
        let Flow::Paged(p) = &mut r.flow else { return };
        r.chrome = false;
        p.drag = 0.0;
        let next = p.page as i64 + by as i64;
        if next < 0 {
            if chapter > 0 {
                self.go_to_chapter(chapter - 1, true);
            }
        } else if next as usize >= p.pages.len() {
            if chapter + 1 < chapters {
                self.go_to_chapter(chapter + 1, false);
            }
        } else {
            p.page = next as usize;
        }
    }

    /// Changes the settings, saves them, and carries the reading position into a new mode.
    pub fn change(&mut self, f: impl FnOnce(&mut Settings)) {
        let before = self.settings;
        f(&mut self.settings);
        if self.settings == before {
            return;
        }
        let _ = self.library.save_settings(&self.settings);
        let mode = self.settings.mode;
        if let Some(r) = self.reading_mut()
            && mode != before.mode
        {
            r.flow = Flow::new(mode, r.flow.fraction());
        }
    }

    pub fn toggle_chrome(&mut self) {
        if let Some(r) = self.reading_mut() {
            r.chrome = !r.chrome;
            if let Flow::Scroll(s) = &mut r.flow {
                s.chrome_at = None;
            }
        }
    }

    pub fn set_panel(&mut self, panel: Option<Panel>) {
        if let Some(r) = self.reading_mut() {
            r.panel = panel;
            if panel.is_some() {
                r.chrome = true;
            }
        }
    }
}

pub fn take_request(state: &mut State) -> Option<HostRequest> {
    state.requests.pop_front()
}

pub fn import(state: &mut State, path: &Path) {
    state.start_import(path.to_path_buf());
}

/// Top, right, bottom, left, in points.
pub fn set_insets(state: &mut State, insets: [f32; 4]) {
    state.insets = insets;
}

fn init() -> State {
    match CONFIG.get() {
        Some(c) => State::open(&c.data_dir, c.host),
        None => State::open(&std::env::temp_dir().join("reader-unconfigured"), Host::Desktop),
    }
}

fn setup(s: &mut State, waker: Waker) {
    s.waker = Some(waker);
}

pub const SCROLL_KEY: &str = "chapter-scroll";
const SAVE_AFTER: f64 = 0.8;

/// Before every frame: imports that finished, pagination, a scroll position to restore, the
/// chrome fading as the reader scrolls on, and the reading position saved once it settles.
pub fn poll(s: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = s.take_imports();
    let ty = Type::of(&s.settings);
    let Screen::Reading(r) = &mut s.screen else { return changed };
    let label = r.chapter_label();
    let chapter_title = r.book.chapters[r.chapter].title.as_deref().map(display_title);
    match &mut r.flow {
        Flow::Scroll(sc) => {
            if let (Some(f), Some(at)) = (sc.restore, sc.at.get()) {
                if at.max[1] > 0.0 {
                    tx.scroll_to_instant(SCROLL_KEY, 0.0, f * at.max[1]);
                }
                sc.chrome_at = None;
                sc.restore = None;
                sc.progress.set(progress_text(&label, f));
            } else if sc.restore.is_none()
                && let Some(at) = sc.at.get()
            {
                let from = *sc.chrome_at.get_or_insert(at.offset[1]);
                if r.chrome && r.panel.is_none() && (at.offset[1] - from).abs() > 48.0 {
                    r.chrome = false;
                    changed = true;
                }
            }
        }
        Flow::Paged(p) => {
            let [w, h] = r.page_box;
            let col = view::column_width(&ty, s.window[0]);
            let key = paginate::Key { chapter: r.chapter, ty, size: [col as u32, h as u32] };
            if w > 0.0 && h > 0.0 && p.key.as_deref() != Some(&key) {
                let fraction = if p.pages.is_empty() { p.want } else { p.page as f32 / p.pages.len() as f32 };
                p.pages = paginate::paginate(&r.book.chapters[r.chapter], chapter_title.as_deref(), &ty, col, h);
                p.page = ((fraction * p.pages.len() as f32 + 1e-3).floor() as usize).min(p.pages.len() - 1);
                p.key = Some(Box::new(key));
                changed = true;
            }
        }
    }
    if r.position() != r.saved {
        match r.dirty_since {
            None => {
                r.dirty_since = Some(now);
                tx.wake_after(Duration::from_secs_f64(SAVE_AFTER));
            }
            Some(t) if now - t >= SAVE_AFTER => s.save_position(),
            Some(_) => tx.wake_after(Duration::from_secs_f64(SAVE_AFTER)),
        }
    }
    changed
}

pub(crate) fn progress_text(label: &str, fraction: f32) -> String {
    format!("{label} · {}%", (fraction * 100.0).round() as i32)
}

pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.is_command('o') {
        s.ask_for_book(tx);
        return true;
    }
    let mode = s.settings.mode;
    let Some(r) = s.reading() else { return false };
    let panel = r.panel.is_some();
    match &ev.key {
        Key::Escape if panel => s.set_panel(None),
        Key::Escape => s.close_book(),
        _ if panel => return false,
        Key::Text(t) if t == " " && mode == PageMode::Paged => s.turn(if ev.mods.shift { -1 } else { 1 }),
        Key::Right | Key::Down | Key::PageDown if mode == PageMode::Paged => s.turn(1),
        Key::Left | Key::Up | Key::PageUp if mode == PageMode::Paged => s.turn(-1),
        Key::Text(t) if t == " " => tx.scroll_page(SCROLL_KEY, if ev.mods.shift { -1 } else { 1 }),
        _ => return false,
    }
    true
}

pub fn spec() -> AppSpec<State> {
    AppSpec {
        scenes: samples::SCENES,
        fonts: FONTS,
        families: FAMILIES,
        min_size: Some((360.0, 480.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Reader", DEFAULT_SIZE, init, view::view)
    }
}

#[cfg(test)]
mod tests;
