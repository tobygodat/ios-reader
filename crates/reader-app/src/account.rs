//! The optional sync account: who's signed in, and what a sync is doing. Signed out, nothing
//! leaves the device.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use reader_core::library::write_atomic;
use reader_core::sync::{self, Client, Outcome, RemoteSettings, Session, SyncError};
use reader_core::{BookId, Settings};
use rune::{TextEdit, Tx, Waker};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{Screen, State};

pub const API: &str = "https://reader-sync.tobygodat.workers.dev";

/// How long to wait after a change before syncing it, so a reading session sends one update.
const AFTER_CHANGE: f64 = 5.0;
/// How often to look for news from other devices while the app is running.
const EVERY: f64 = 300.0;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Stored {
    session: Option<Session>,
    /// When the settings last changed here, in milliseconds since the Unix epoch.
    settings_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Running,
    Done { skipped: Vec<String> },
    Failed(String),
}

/// A finished network job, waiting for the main thread.
enum Finished {
    SignedIn(Result<Session, SyncError>),
    Synced(Result<Outcome, SyncError>),
    Deleted(Result<(), SyncError>),
}

/// The account screen's form and its state.
pub struct AccountForm {
    pub busy: bool,
    pub error: Option<String>,
    /// The first press of Delete asks; the second deletes.
    pub confirm_delete: bool,
}

pub struct Account {
    path: PathBuf,
    stored: Stored,
    pub status: SyncStatus,
    api: String,
    finished: Arc<Mutex<Vec<Finished>>>,
    /// Something changed here since the last sync began.
    wanted: bool,
    last_started: Option<f64>,
    pub email_field: TextEdit,
    pub password_field: TextEdit,
}

impl Account {
    pub fn open(dir: &Path) -> Account {
        let path = dir.join("account.json");
        let stored = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Account {
            path,
            stored,
            status: SyncStatus::Idle,
            api: API.to_string(),
            finished: Arc::default(),
            wanted: false,
            last_started: None,
            email_field: TextEdit::new(""),
            password_field: TextEdit::new("").secure(),
        }
    }

    pub fn session(&self) -> Option<&Session> {
        self.stored.session.as_ref()
    }

    pub fn set_api(&mut self, api: impl Into<String>) {
        self.api = api.into();
    }

    fn save(&self) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.stored) {
            let _ = write_atomic(&self.path, &bytes);
        }
    }

    pub fn want_sync(&mut self) {
        self.wanted = true;
    }

    /// Notes that the settings changed here, so they win over older ones elsewhere.
    pub fn settings_changed(&mut self) {
        self.stored.settings_at = now_millis().max(self.stored.settings_at + 1);
        self.save();
        self.wanted = true;
    }
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn spawn(account: &Account, waker: &Option<Waker>, job: impl FnOnce(Client) -> Finished + Send + 'static) {
    let (api, finished, waker) = (account.api.clone(), account.finished.clone(), waker.clone());
    std::thread::spawn(move || {
        let result = job(Client::new(api));
        finished.lock().expect("sync results lock").push(result);
        if let Some(wake) = waker {
            wake();
        }
    });
}

impl State {
    pub fn open_account(&mut self) {
        self.screen = Screen::Account(AccountForm { busy: false, error: None, confirm_delete: false });
    }

    pub fn close_account(&mut self) {
        self.account.email_field.clear();
        self.account.password_field.clear();
        self.screen = Screen::Library;
    }

    fn form(&mut self) -> Option<&mut AccountForm> {
        match &mut self.screen {
            Screen::Account(form) => Some(form),
            _ => None,
        }
    }

    /// Signs in with, or creates an account from, what the desktop's fields hold.
    pub fn submit_fields(&mut self, create: bool) {
        let email = self.account.email_field.text.trim().to_string();
        let password = self.account.password_field.text.to_string();
        self.submit_credentials(email, password, create);
    }

    /// Signs in or creates an account on a worker thread; the result lands in [`crate::poll`].
    pub fn submit_credentials(&mut self, email: String, password: String, create: bool) {
        if let Some(form) = self.form() {
            form.busy = true;
            form.error = None;
        }
        spawn(&self.account, &self.waker, move |client| {
            Finished::SignedIn(if create { client.sign_up(&email, &password) } else { client.sign_in(&email, &password) })
        });
    }

    /// Stops syncing. The books on this device stay, and the account keeps its copy.
    pub fn sign_out(&mut self) {
        if let Some(session) = self.account.stored.session.take() {
            spawn(&self.account, &None, move |client| {
                let _ = client.sign_out(&session.token);
                Finished::Synced(Err(SyncError::Local("signed out".into())))
            });
        }
        self.account.save();
        self.account.status = SyncStatus::Idle;
        self.account.last_started = None;
    }

    /// The first press asks; the second removes the account and everything stored in it.
    pub fn delete_account(&mut self) {
        let Some(token) = self.account.session().map(|s| s.token.clone()) else { return };
        let Some(form) = self.form() else { return };
        if !form.confirm_delete {
            form.confirm_delete = true;
            return;
        }
        form.busy = true;
        form.error = None;
        spawn(&self.account, &self.waker, move |client| Finished::Deleted(client.delete_account(&token)));
    }

    pub fn sync_now(&mut self) {
        self.account.wanted = true;
        self.account.last_started = None;
    }

    /// Starts a sync when one is due. Called before every frame.
    pub(crate) fn sync_if_due(&mut self, now: f64, tx: &mut Tx) {
        if self.account.session().is_none() || self.account.status == SyncStatus::Running {
            return;
        }
        let waited = self.account.last_started.map(|t| now - t);
        let due = waited.is_none_or(|w| w >= EVERY || (self.account.wanted && w >= AFTER_CHANGE));
        if !due {
            if self.account.wanted {
                tx.wake_after(Duration::from_secs_f64(AFTER_CHANGE));
            }
            return;
        }
        self.account.last_started = Some(now);
        self.account.wanted = false;
        self.account.status = SyncStatus::Running;
        let token = self.account.session().map(|s| s.token.clone()).unwrap_or_default();
        let dir = self.library.dir().to_path_buf();
        let entries = self.library.entries().to_vec();
        let settings = RemoteSettings { json: serde_json::to_string(&self.settings).unwrap_or_default(), updated_at: self.account.stored.settings_at };
        spawn(&self.account, &self.waker, move |client| Finished::Synced(sync::run(&client, &token, &dir, &entries, &settings)));
    }

    /// Applies finished network jobs. True when anything on screen may have changed.
    pub(crate) fn take_finished(&mut self) -> bool {
        let done: Vec<_> = std::mem::take(&mut *self.account.finished.lock().expect("sync results lock"));
        let any = !done.is_empty();
        for job in done {
            match job {
                Finished::SignedIn(Ok(session)) => self.signed_in(session),
                Finished::SignedIn(Err(e)) => self.show_form_error(e),
                Finished::Deleted(Ok(())) => {
                    self.sign_out_locally();
                    self.close_account();
                }
                Finished::Deleted(Err(e)) => self.show_form_error(e),
                Finished::Synced(result) if self.account.session().is_some() => self.finish_sync(result),
                Finished::Synced(_) => {}
            }
        }
        any
    }

    fn signed_in(&mut self, session: Session) {
        self.account.stored.session = Some(session);
        // Settings chosen before signing in beat a brand new account's nothing, but not settings
        // someone has already synced from another device.
        if self.account.stored.settings_at == 0 && self.settings != Settings::default() {
            self.account.stored.settings_at = 1;
        }
        self.account.save();
        self.account.last_started = None;
        self.account.password_field.clear();
        self.account.email_field.clear();
        if let Some(form) = self.form() {
            form.busy = false;
        }
    }

    fn sign_out_locally(&mut self) {
        self.account.stored.session = None;
        self.account.save();
        self.account.status = SyncStatus::Idle;
    }

    fn show_form_error(&mut self, e: SyncError) {
        if let Some(form) = self.form() {
            form.busy = false;
            form.error = Some(e.to_string());
        }
    }

    fn finish_sync(&mut self, result: Result<Outcome, SyncError>) {
        match result {
            Ok(outcome) => {
                let open: Option<BookId> = self.reading().map(|r| r.id.clone());
                if let Err(e) = self.library.apply_remote(&outcome.books, open.as_ref()) {
                    self.account.status = SyncStatus::Failed(format!("Couldn’t save what arrived: {e}."));
                    return;
                }
                if let Some(theirs) = outcome.settings
                    && let Ok(settings) = serde_json::from_str::<Settings>(&theirs.json)
                {
                    self.change(|s| *s = settings);
                    self.account.stored.settings_at = theirs.updated_at;
                    self.account.wanted = false;
                    self.account.save();
                }
                self.account.status = SyncStatus::Done { skipped: outcome.skipped };
            }
            Err(SyncError::SignedOut) => {
                self.sign_out_locally();
                self.account.status = SyncStatus::Failed(SyncError::SignedOut.to_string());
            }
            Err(e) => self.account.status = SyncStatus::Failed(e.to_string()),
        }
    }
}
