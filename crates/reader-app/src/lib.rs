//! The reader's Rune UI, shared by the Mac binary and the iOS static library.
//!
//! Host contract (the Mac and iOS crates call only these):
//! - [`configure`] once, before [`spec`], with where to keep the library and who picks files.
//! - [`spec`] builds the `AppSpec` both hosts run.
//! - [`take_request`] after each frame: something only the host can do (the iOS document picker).
//! - [`import`] with a file the host picked.
//! - [`search_folders`] with the folders the host may read.
//! - [`submit_credentials`] with an email and password the host asked for.
//! - [`set_insets`] with the screen's safe-area insets (iOS; Rune reports zero on native).

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub use account::{Account, AccountForm, SyncStatus};
use reader_core::scan::{self, Found};
use reader_core::{Book, BookId, ImportError, Library, PageMode, Position, Prepared, Settings};
use rune::{AppSpec, Image, ImageCache, Key, KeyEvent, LiveText, ScrollPos, Tx, Waker};

mod account;
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
    /// Search for books. iOS only lets an app read what the person allows, so the host searches
    /// the folders it remembers, asking for one if it has none.
    FindBooks,
    /// One more folder to remember and search.
    AddFolder,
    /// Stop remembering the folders.
    ForgetFolders,
    /// An email and password. The Rune fields have no keyboard on iOS, so the host asks natively.
    AskCredentials { create: bool },
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
    Finding(Finding),
    Account(AccountForm),
}

/// Looking through folders for books, then choosing which to add.
pub enum Finding {
    Searching,
    Results(Vec<Candidate>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub found: Found,
    pub chosen: bool,
}

impl Candidate {
    /// EPUBs are almost always books; a folder's PDFs are as likely to be receipts.
    fn of(found: Found) -> Candidate {
        let chosen = found.format == reader_core::Format::Epub;
        Candidate { found, chosen }
    }
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
    /// A turn asked for this frame and applied on the next one, by -1, 0 (settle back) or 1. A
    /// touch release delivers a last `Phase::Move` in the same frame, and its instant
    /// transaction would make any turn applied with it jump instead of slide.
    pub(crate) release: Option<i32>,
    /// Which way pages are leaving: new chapters come in from this side, old ones go out the other.
    pub heading: i32,
}

impl Flow {
    fn new(mode: PageMode, fraction: f32) -> Flow {
        match mode {
            PageMode::Scroll => Flow::Scroll(Scroll { at: Rc::default(), restore: Some(fraction), chrome_at: None, progress: LiveText::new("") }),
            PageMode::Paged => Flow::Paged(Paged { pages: Vec::new(), key: None, page: 0, want: fraction, drag: 0.0, release: None, heading: 1 }),
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
    /// Decodes covers off the main thread. Set up with the waker, so tests go without pictures.
    images: Option<ImageCache>,
    /// Holding an image keeps it decoded for the shelf.
    pub covers: HashMap<BookId, Image>,
    /// How many covers the shelf has drawn. A decode finishing rebuilds it.
    covers_ready: usize,
    /// What a search found, until the next frame takes it.
    search_results: Arc<Mutex<Option<Vec<Found>>>>,
    pub account: Account,
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
            images: None,
            covers: HashMap::new(),
            covers_ready: 0,
            search_results: Arc::default(),
            account: Account::open(dir),
        }
    }

    pub fn reading(&self) -> Option<&Reading> {
        match &self.screen {
            Screen::Reading(r) => Some(r),
            _ => None,
        }
    }

    fn reading_mut(&mut self) -> Option<&mut Reading> {
        match &mut self.screen {
            Screen::Reading(r) => Some(r),
            _ => None,
        }
    }

    /// Asks the iOS host for an email and password. The desktop has its own fields.
    pub fn ask_for_credentials(&mut self, create: bool) {
        self.requests.push_back(HostRequest::AskCredentials { create });
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
        self.start_imports(vec![path]);
    }

    /// Parses `paths` one at a time on a worker thread, so a large batch doesn't fill the memory
    /// of a phone. Each result lands in [`poll`].
    fn start_imports(&mut self, paths: Vec<PathBuf>) {
        let Some(first) = paths.first() else { return };
        let name = first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let more = paths.len() - 1;
        self.import = match &self.import {
            Import::Parsing { name: current, queued } => Import::Parsing { name: current.clone(), queued: queued + paths.len() },
            _ => Import::Parsing { name, queued: more },
        };
        let dir = self.library.dir().to_path_buf();
        let inbox = self.inbox.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            for path in paths {
                let result = reader_core::prepare(&dir, &path);
                inbox.lock().expect("import inbox lock").push(result);
                if let Some(wake) = &waker {
                    wake();
                }
            }
        });
    }

    /// Starts a search: the usual folders on the desktop, the ones the person allowed on iOS.
    pub fn find_books(&mut self) {
        match self.host {
            Host::Ios => self.requests.push_back(HostRequest::FindBooks),
            Host::Desktop => {
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                self.search(desktop_folders(&home));
            }
        }
    }

    /// Searches `roots` on a worker thread; the result lands in [`poll`].
    pub fn search(&mut self, roots: Vec<PathBuf>) {
        self.screen = Screen::Finding(Finding::Searching);
        let known = self.library.entries().iter().map(|e| e.id.clone()).collect();
        let results = self.search_results.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let found = scan::scan(&roots, &known);
            *results.lock().expect("search results lock") = Some(found);
            if let Some(wake) = waker {
                wake();
            }
        });
    }

    /// Adds a finished search's results to the screen, unless the person has already left it.
    fn take_search(&mut self) -> bool {
        let Some(found) = self.search_results.lock().expect("search results lock").take() else { return false };
        let Screen::Finding(finding @ Finding::Searching) = &mut self.screen else { return false };
        *finding = Finding::Results(found.into_iter().map(Candidate::of).collect());
        true
    }

    /// Flips one search result between chosen and not.
    pub fn toggle_candidate(&mut self, at: usize) {
        if let Screen::Finding(Finding::Results(candidates)) = &mut self.screen
            && let Some(candidate) = candidates.get_mut(at)
        {
            candidate.chosen = !candidate.chosen;
        }
    }

    /// Leaves the search without adding anything.
    pub fn close_finding(&mut self) {
        self.screen = Screen::Library;
    }

    /// Asks the host for another folder. The results stay up if the person changes their mind.
    pub fn add_folder(&mut self) {
        self.requests.push_back(HostRequest::AddFolder);
    }

    /// Has the host forget the folders, so the next search asks again.
    pub fn forget_folders(&mut self) {
        self.requests.push_back(HostRequest::ForgetFolders);
        self.close_finding();
    }

    /// Imports the chosen results and goes back to the library to show them arriving.
    pub fn add_chosen(&mut self) {
        let Screen::Finding(Finding::Results(candidates)) = &self.screen else { return };
        let paths = candidates.iter().filter(|c| c.chosen).map(|c| c.found.path.clone()).collect();
        self.screen = Screen::Library;
        self.start_imports(paths);
    }

    fn take_imports(&mut self) -> bool {
        let done: Vec<_> = std::mem::take(&mut *self.inbox.lock().expect("import inbox lock"));
        if done.is_empty() {
            return false;
        }
        for result in done {
            let failed = match result {
                Ok(book) => {
                    self.account.want_sync();
                    self.library.add(&book).err().map(|e| format!("The book couldn’t be saved: {e}."))
                }
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
            self.account.want_sync();
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

    /// Asks for a turn of `by` pages on the next frame, so that it slides. See [`Paged::release`].
    pub fn queue_turn(&mut self, by: i32, tx: &mut Tx) {
        let Some(r) = self.reading_mut() else { return };
        let Flow::Paged(p) = &mut r.flow else { return };
        p.release = Some(by);
        if by != 0 {
            p.heading = by;
        }
        tx.wake_after(Duration::ZERO);
    }

    /// Starts decoding the cover of any book that has one and isn't on the shelf yet. True when
    /// another cover has finished decoding, so the shelf should be drawn again.
    fn load_covers(&mut self) -> bool {
        let Some(images) = &self.images else { return false };
        for entry in self.library.entries() {
            if !self.covers.contains_key(&entry.id)
                && let Some(path) = self.library.cover_path(entry)
            {
                self.covers.insert(entry.id.clone(), images.file(path, COVER_POINTS, COVER_SCALE));
            }
        }
        let ready = self.covers.values().filter(|c| c.is_ready()).count();
        std::mem::replace(&mut self.covers_ready, ready) != ready
    }

    fn take_release(&mut self) -> Option<i32> {
        let Some(r) = self.reading_mut() else { return None };
        let Flow::Paged(p) = &mut r.flow else { return None };
        p.release.take()
    }

    /// Whether a turn `by` pages from here would stay inside the book.
    pub fn can_turn(&self, by: i32) -> bool {
        let Some(r) = self.reading() else { return false };
        let Flow::Paged(p) = &r.flow else { return false };
        let next = p.page as i64 + by as i64;
        (next >= 0 && (next as usize) < p.pages.len()) || (next < 0 && r.chapter > 0) || (next >= p.pages.len() as i64 && r.chapter + 1 < r.book.chapters.len())
    }

    /// Turns `by` pages in paged mode, across chapter ends. Use [`State::queue_turn`] from input.
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
                self.head(-1);
            }
        } else if next as usize >= p.pages.len() {
            if chapter + 1 < chapters {
                self.go_to_chapter(chapter + 1, false);
                self.head(1);
            }
        } else {
            p.page = next as usize;
        }
    }

    fn head(&mut self, by: i32) {
        if let Some(Reading { flow: Flow::Paged(p), .. }) = self.reading_mut() {
            p.heading = by;
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
        self.account.settings_changed();
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

/// Signs in with what the host asked for, or creates an account from it.
pub fn submit_credentials(state: &mut State, email: String, password: String, create: bool) {
    state.submit_credentials(email, password, create);
}

/// Searches folders on the host's behalf. The host keeps them readable until the next search.
pub fn search_folders(state: &mut State, paths: Vec<PathBuf>) {
    state.search(paths);
}

/// Where a person's books tend to be on a Mac. The private ones ask for permission when read.
pub fn desktop_folders(home: &Path) -> Vec<PathBuf> {
    ["Documents", "Downloads", "Desktop", "Books", "Library/Mobile Documents/com~apple~CloudDocs"].iter().map(|f| home.join(f)).collect()
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
    s.images = Some(ImageCache::new(waker.clone()));
    s.waker = Some(waker);
}

/// The longest side a cover is decoded for: a large tile's height, on a 3x screen.
const COVER_POINTS: f32 = 300.0;
const COVER_SCALE: f32 = 3.0;

pub const SCROLL_KEY: &str = "chapter-scroll";
const SAVE_AFTER: f64 = 0.8;

/// Before every frame: imports that finished, pagination, a scroll position to restore, the
/// chrome fading as the reader scrolls on, and the reading position saved once it settles.
pub fn poll(s: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = s.take_imports();
    changed |= s.load_covers();
    changed |= s.take_search();
    changed |= s.take_finished();
    s.sync_if_due(now, tx);
    if let Some(by) = s.take_release() {
        s.turn(by);
        changed = true;
    }
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
    if ev.key == Key::Escape {
        match s.screen {
            Screen::Finding(_) => {
                s.close_finding();
                return true;
            }
            Screen::Account(_) => {
                s.close_account();
                return true;
            }
            _ => {}
        }
    }
    let mode = s.settings.mode;
    let Some(r) = s.reading() else { return false };
    let panel = r.panel.is_some();
    match &ev.key {
        Key::Escape if panel => s.set_panel(None),
        Key::Escape => s.close_book(),
        _ if panel => return false,
        Key::Text(t) if t == " " && mode == PageMode::Paged => s.queue_turn(if ev.mods.shift { -1 } else { 1 }, tx),
        Key::Right | Key::Down | Key::PageDown if mode == PageMode::Paged => s.queue_turn(1, tx),
        Key::Left | Key::Up | Key::PageUp if mode == PageMode::Paged => s.queue_turn(-1, tx),
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
