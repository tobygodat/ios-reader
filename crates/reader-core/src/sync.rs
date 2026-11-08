//! Keeping a library the same across devices. Every device keeps its own full copy and reads from
//! it; the server holds a copy per account. [`plan`] compares the two and says what to move. Each
//! book's position is one value stamped with when it changed, and the later stamp wins. Books are
//! only ever added, and a book's id is the hash of its file, so they can't conflict.

use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::book::{BookId, Format, Position};
use crate::library::{self, Entry};

/// A book as the server lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteBook {
    pub id: BookId,
    pub title: String,
    pub author: Option<String>,
    pub format: Format,
    pub chapters: usize,
    pub cover: Option<String>,
    pub added: u64,
    pub chapter: usize,
    pub fraction: f32,
    pub position_at: u64,
}

impl RemoteBook {
    pub fn of(e: &Entry) -> RemoteBook {
        RemoteBook {
            id: e.id.clone(),
            title: e.title.clone(),
            author: e.author.clone(),
            format: e.format,
            chapters: e.chapters,
            cover: e.cover.clone(),
            added: e.added,
            chapter: e.position.chapter,
            fraction: e.position.fraction,
            position_at: e.position_at,
        }
    }

    pub fn position(&self) -> Position {
        Position { chapter: self.chapter, fraction: self.fraction }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteSettings {
    pub json: String,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteLibrary {
    pub books: Vec<RemoteBook>,
    pub settings: Option<RemoteSettings>,
}

/// What a sync has to move. Applying it twice changes nothing the second time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// On this device only: send the text, then the cover, then the row.
    pub upload: Vec<BookId>,
    /// On both, but this device has something newer: send the row, and the cover if named.
    pub push: Vec<(BookId, bool)>,
    /// On the server only: fetch the text and cover.
    pub download: Vec<RemoteBook>,
    /// On both, but the server has something newer.
    pub pull: Vec<RemoteBook>,
}

pub fn plan(local: &[Entry], remote: &[RemoteBook]) -> Plan {
    let mut plan = Plan::default();
    for entry in local {
        match remote.iter().find(|r| r.id == entry.id) {
            None => plan.upload.push(entry.id.clone()),
            Some(r) => {
                let send_cover = entry.cover.is_some() && r.cover.is_none();
                if entry.position_at > r.position_at || send_cover {
                    plan.push.push((entry.id.clone(), send_cover));
                }
                if r.position_at > entry.position_at || (r.cover.is_some() && entry.cover.is_none()) {
                    plan.pull.push(r.clone());
                }
            }
        }
    }
    plan.download = remote.iter().filter(|r| !local.iter().any(|e| e.id == r.id)).cloned().collect();
    plan
}

/// Why a sync stopped, in words the reader is shown.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncError {
    /// The account's session has ended: sign in again.
    SignedOut,
    /// The server said why.
    Refused(String),
    /// The server couldn't be reached.
    Offline,
    Local(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::SignedOut => write!(f, "You’ve been signed out. Sign in again to keep syncing."),
            SyncError::Refused(why) => write!(f, "{why}"),
            SyncError::Offline => write!(f, "Couldn’t reach the sync service. Your books are safe here, and syncing will try again."),
            SyncError::Local(why) => write!(f, "Couldn’t sync: {why}."),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<io::Error> for SyncError {
    fn from(e: io::Error) -> SyncError {
        SyncError::Local(e.to_string().to_lowercase())
    }
}

/// The session a sign-in returned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub email: String,
    pub token: String,
}

/// The sync service over HTTPS, using the operating system's TLS.
pub struct Client {
    base: String,
    agent: ureq::Agent,
}

const MAX_DOWNLOAD: u64 = 30 * 1024 * 1024;

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Client {
        let tls = ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(120)))
            .user_agent("Reader")
            .build();
        Client { base: base.into(), agent: config.into() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn check(mut res: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, SyncError> {
        let status = res.status().as_u16();
        let body = res.body_mut().with_config().limit(MAX_DOWNLOAD).read_to_vec().map_err(|_| SyncError::Offline)?;
        match status {
            200..=299 => Ok(body),
            401 if serde_json::from_slice::<ErrorBody>(&body).is_ok_and(|e| e.error.starts_with("Sign in")) => Err(SyncError::SignedOut),
            _ => Err(SyncError::Refused(
                serde_json::from_slice::<ErrorBody>(&body).map(|e| e.error).unwrap_or_else(|_| "The sync service had a problem. Try again in a moment.".into()),
            )),
        }
    }

    fn credentials(&self, path: &str, email: &str, password: &str) -> Result<Session, SyncError> {
        let body = serde_json::json!({ "email": email, "password": password }).to_string();
        let res = self.agent.post(self.url(path)).content_type("application/json").send(&body).map_err(|_| SyncError::Offline)?;
        let bytes = Self::check(res)?;
        serde_json::from_slice(&bytes).map_err(|_| SyncError::Refused("The sync service sent something unexpected.".into()))
    }

    pub fn sign_up(&self, email: &str, password: &str) -> Result<Session, SyncError> {
        self.credentials("/v1/sign-up", email, password)
    }

    pub fn sign_in(&self, email: &str, password: &str) -> Result<Session, SyncError> {
        self.credentials("/v1/sign-in", email, password)
    }

    pub fn sign_out(&self, token: &str) -> Result<(), SyncError> {
        let res = self.agent.post(self.url("/v1/sign-out")).header("authorization", format!("Bearer {token}")).send_empty().map_err(|_| SyncError::Offline)?;
        Self::check(res).map(|_| ())
    }

    pub fn delete_account(&self, token: &str) -> Result<(), SyncError> {
        let res = self.agent.delete(self.url("/v1/account")).header("authorization", format!("Bearer {token}")).call().map_err(|_| SyncError::Offline)?;
        Self::check(res).map(|_| ())
    }

    fn library(&self, token: &str) -> Result<RemoteLibrary, SyncError> {
        let res = self.agent.get(self.url("/v1/library")).header("authorization", format!("Bearer {token}")).call().map_err(|_| SyncError::Offline)?;
        serde_json::from_slice(&Self::check(res)?).map_err(|_| SyncError::Refused("The sync service sent something unexpected.".into()))
    }

    fn put_blob(&self, token: &str, kind: &str, name: &str, bytes: &[u8]) -> Result<(), SyncError> {
        let res = self.agent.put(self.url(&format!("/v1/blobs/{kind}/{name}"))).header("authorization", format!("Bearer {token}")).send(bytes).map_err(|_| SyncError::Offline)?;
        Self::check(res).map(|_| ())
    }

    fn get_blob(&self, token: &str, kind: &str, name: &str) -> Result<Vec<u8>, SyncError> {
        let res = self.agent.get(self.url(&format!("/v1/blobs/{kind}/{name}"))).header("authorization", format!("Bearer {token}")).call().map_err(|_| SyncError::Offline)?;
        Self::check(res)
    }

    fn put_row(&self, token: &str, book: &RemoteBook) -> Result<(), SyncError> {
        let body = serde_json::to_string(book).map_err(|e| SyncError::Local(e.to_string()))?;
        let res = self.agent.put(self.url(&format!("/v1/books/{}", book.id.0))).header("authorization", format!("Bearer {token}")).content_type("application/json").send(&body).map_err(|_| SyncError::Offline)?;
        Self::check(res).map(|_| ())
    }

    fn put_settings(&self, token: &str, settings: &RemoteSettings) -> Result<(), SyncError> {
        let body = serde_json::to_string(settings).map_err(|e| SyncError::Local(e.to_string()))?;
        let res = self.agent.put(self.url("/v1/settings")).header("authorization", format!("Bearer {token}")).content_type("application/json").send(&body).map_err(|_| SyncError::Offline)?;
        Self::check(res).map(|_| ())
    }
}

/// What a sync brought back, for the main thread to apply to the library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Newly downloaded books and newer positions. Their files are already on disk.
    pub books: Vec<RemoteBook>,
    pub settings: Option<RemoteSettings>,
    /// Books too large for the service, or refused for another reason. The rest still synced.
    pub skipped: Vec<String>,
}

/// Runs one sync on a worker thread. It only writes book and cover files, which are named by their
/// content, and leaves the index to the caller. `local` is a snapshot from the main thread.
pub fn run(client: &Client, token: &str, dir: &Path, local: &[Entry], settings: &RemoteSettings) -> Result<Outcome, SyncError> {
    let remote = client.library(token)?;
    let plan = plan(local, &remote.books);
    let mut outcome = Outcome::default();
    let entry = |id: &BookId| local.iter().find(|e| &e.id == id);

    for id in &plan.upload {
        let Some(e) = entry(id) else { continue };
        let text = fs::read(library::book_path(dir, id))?;
        match upload(client, token, dir, e, &text) {
            Err(SyncError::Refused(_)) => outcome.skipped.push(e.title.clone()),
            other => other?,
        }
    }
    for (id, send_cover) in &plan.push {
        let Some(e) = entry(id) else { continue };
        if *send_cover {
            upload_cover(client, token, dir, e)?;
        }
        client.put_row(token, &RemoteBook::of(e))?;
    }
    for book in &plan.download {
        let text = client.get_blob(token, "book", &book.id.0)?;
        library::write_atomic(&library::book_path(dir, &book.id), &text)?;
        download_cover(client, token, dir, book);
        outcome.books.push(book.clone());
    }
    for book in plan.pull {
        download_cover(client, token, dir, &book);
        outcome.books.push(book);
    }

    match remote.settings {
        Some(theirs) if theirs.updated_at > settings.updated_at => outcome.settings = Some(theirs),
        Some(theirs) if theirs.updated_at == settings.updated_at => {}
        _ if settings.updated_at > 0 => client.put_settings(token, settings)?,
        _ => {}
    }
    Ok(outcome)
}

fn upload(client: &Client, token: &str, dir: &Path, e: &Entry, text: &[u8]) -> Result<(), SyncError> {
    client.put_blob(token, "book", &e.id.0, text)?;
    upload_cover(client, token, dir, e)?;
    client.put_row(token, &RemoteBook::of(e))
}

fn upload_cover(client: &Client, token: &str, dir: &Path, e: &Entry) -> Result<(), SyncError> {
    let Some(name) = &e.cover else { return Ok(()) };
    let bytes = fs::read(dir.join("covers").join(name))?;
    client.put_blob(token, "cover", name, &bytes)
}

/// A cover that can't be fetched only costs the book its picture until the next sync.
fn download_cover(client: &Client, token: &str, dir: &Path, book: &RemoteBook) {
    let Some(name) = &book.cover else { return };
    let path = dir.join("covers").join(name);
    if path.exists() {
        return;
    }
    if let Ok(bytes) = client.get_blob(token, "cover", name)
        && fs::create_dir_all(dir.join("covers")).is_ok()
    {
        let _ = library::write_atomic(&path, &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, chapter: usize, at: u64, cover: Option<&str>) -> Entry {
        Entry {
            id: BookId(id.into()),
            title: id.into(),
            author: None,
            format: Format::Epub,
            position: Position { chapter, fraction: 0.0 },
            position_at: at,
            added: 1,
            chapters: 9,
            cover: cover.map(str::to_string),
        }
    }

    #[test]
    fn sorts_each_book_by_where_the_news_is() {
        let local = [entry("mine", 0, 0, None), entry("ahead", 5, 200, None), entry("behind", 1, 100, None), entry("same", 2, 100, None)];
        let remote = [
            RemoteBook::of(&entry("theirs", 3, 50, None)),
            RemoteBook::of(&entry("ahead", 1, 100, None)),
            RemoteBook::of(&entry("behind", 7, 300, None)),
            RemoteBook::of(&entry("same", 2, 100, None)),
        ];
        let p = plan(&local, &remote);
        assert_eq!(p.upload, [BookId("mine".into())]);
        assert_eq!(p.push, [(BookId("ahead".into()), false)]);
        assert_eq!(p.download.iter().map(|b| b.id.0.as_str()).collect::<Vec<_>>(), ["theirs"]);
        assert_eq!(p.pull.iter().map(|b| b.id.0.as_str()).collect::<Vec<_>>(), ["behind"]);
    }

    #[test]
    fn a_cover_only_one_side_has_moves_without_touching_the_position() {
        let p = plan(&[entry("a", 1, 100, Some("a.jpg"))], &[RemoteBook::of(&entry("a", 1, 100, None))]);
        assert_eq!(p.push, [(BookId("a".into()), true)]);
        assert!(p.pull.is_empty());
        let p = plan(&[entry("a", 1, 100, None)], &[RemoteBook::of(&entry("a", 1, 100, Some("a.jpg")))]);
        assert!(p.push.is_empty());
        assert_eq!(p.pull.len(), 1);
    }

    /// A small deterministic generator, so a failure reproduces from its seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, below: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % below
        }
    }

    /// One device's turn against an in-memory server, doing what [`run`] does and what the app
    /// then does with the outcome. The server's rule is the Worker's: a later stamp wins.
    fn sync(device: &mut Vec<Entry>, server: &mut Vec<RemoteBook>) {
        let p = plan(device, server);
        let row = |device: &Vec<Entry>, id: &BookId| RemoteBook::of(device.iter().find(|e| &e.id == id).unwrap());
        for id in &p.upload {
            server.push(row(device, id));
        }
        for (id, _) in &p.push {
            let mine = row(device, id);
            let theirs = server.iter_mut().find(|b| &b.id == id).unwrap();
            if mine.position_at > theirs.position_at {
                (theirs.chapter, theirs.fraction, theirs.position_at) = (mine.chapter, mine.fraction, mine.position_at);
            }
            theirs.cover = theirs.cover.take().or(mine.cover);
        }
        for r in p.download.iter().chain(&p.pull) {
            library::merge_remote(device, r);
        }
    }

    #[test]
    fn any_two_devices_end_up_the_same_with_the_latest_position_of_every_book() {
        for seed in 1..=500u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut server = Vec::new();
            let mut devices = [Vec::new(), Vec::new()];
            let mut clock = 0u64;
            let mut latest = std::collections::HashMap::new();
            for _ in 0..rng.next(40) + 1 {
                let which = rng.next(2) as usize;
                clock += 1;
                match rng.next(4) {
                    0 => {
                        let name = format!("book{}", rng.next(6));
                        if !devices[which].iter().any(|e: &Entry| e.id.0 == name) {
                            let cover = (rng.next(2) == 0).then(|| format!("{name}.jpg"));
                            devices[which].push(entry(&name, 0, 0, cover.as_deref()));
                            latest.entry(name).or_insert((0, 0));
                        }
                    }
                    1 | 2 if !devices[which].is_empty() => {
                        let at = rng.next(devices[which].len() as u64) as usize;
                        let chapter = rng.next(9) as usize;
                        let e = &mut devices[which][at];
                        e.position.chapter = chapter;
                        e.position_at = clock;
                        latest.insert(e.id.0.clone(), (clock, chapter));
                    }
                    _ => sync(&mut devices[which], &mut server),
                }
            }
            for _ in 0..2 {
                sync(&mut devices[0], &mut server);
                sync(&mut devices[1], &mut server);
            }
            let view = |d: &Vec<Entry>| {
                let mut v: Vec<_> = d.iter().map(|e| (e.id.0.clone(), e.position.chapter, e.position_at, e.cover.is_some())).collect();
                v.sort();
                v
            };
            assert_eq!(view(&devices[0]), view(&devices[1]), "seed {seed}: the devices differ");
            for e in &devices[0] {
                let (at, chapter) = latest[&e.id.0];
                assert_eq!((e.position_at, e.position.chapter), (at, chapter), "seed {seed}: {} lost its latest position", e.id.0);
            }
            assert!(plan(&devices[0], &server) == Plan::default(), "seed {seed}: a settled device still has work");
        }
    }
}
