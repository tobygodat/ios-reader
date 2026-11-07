//! The reader's Rune UI, shared by the Mac binary and the iOS static library.
//!
//! Host contract (the Mac and iOS crates call only these):
//! - [`configure`] once, before [`spec`], with where to keep the library and who picks files.
//! - [`spec`] builds the `AppSpec` both hosts run.
//! - [`take_request`] after each frame: something only the host can do (the iOS document picker).
//! - [`import`] with a file the host picked.
//! - [`set_insets`] with the screen's safe-area insets (iOS; Rune reports zero on native).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rune::tok;
use rune::{AppSpec, Cx, El, col, text};

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

#[derive(Default)]
pub struct State {
    insets: [f32; 4],
    requests: Vec<HostRequest>,
}

pub fn take_request(state: &mut State) -> Option<HostRequest> {
    if state.requests.is_empty() { None } else { Some(state.requests.remove(0)) }
}

pub fn import(_state: &mut State, _path: &Path) {}

/// Top, right, bottom, left, in points.
pub fn set_insets(state: &mut State, insets: [f32; 4]) {
    state.insets = insets;
}

fn view(_s: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(false);
    col().w_full().h_full().center().fill(t.bg).child(text("Reader").color(t.text))
}

pub fn spec() -> AppSpec<State> {
    AppSpec::new("Reader", (1100.0, 760.0), State::default, view)
}
