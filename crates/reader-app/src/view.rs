//! Every screen, drawn from [`State`]. The whole app takes the page theme, so the library, the
//! page and the panel read as one surface.

use reader_core::{Entry, FontChoice, LineSpacing, PageMode, PageTheme, Settings};
use rune::tok::motion;
use rune::{Align, Color, Cursor, Cx, El, Image, Phase, ScrollPos, ScrollSpec, Side, SurfaceFit, TextStyle, Transition, Tx, col, el, row, text, text_input};

use crate::look::{Kind, Palette, SANS, SERIF, Type, blend, display_title, label_family};
use crate::paginate::Piece;
use crate::{AccountForm, Candidate, Finding, Flow, Host, SyncStatus, Import, Panel, Reading, SCROLL_KEY, Screen, State, progress_text};

const TOP_BAR: f32 = 52.0;
const BOTTOM_LINE: f32 = 40.0;
/// Below this width the appearance panel is a bottom sheet.
const NARROW: f32 = 600.0;

fn ui(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight).family(SANS)
}

/// The page's side margin for a window this wide.
fn margin(window_w: f32) -> f32 {
    (window_w * 0.07).clamp(20.0, 72.0).round()
}

/// The reading column: about 34 em of the body size, and never wider than the window allows.
pub fn column_width(ty: &Type, window_w: f32) -> f32 {
    ty.measure.min(window_w - 2.0 * margin(window_w)).max(200.0).floor()
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let pal = Palette::of(s.settings.theme);
    let mut layers = Vec::new();
    let screen = match &s.screen {
        Screen::Library => library(s, cx, &pal),
        Screen::Reading(r) => reader(s, r, cx, &pal, &mut layers),
        Screen::Finding(finding) => finding_view(s, finding, &pal),
        Screen::Account(form) => account_view(s, form, &pal),
    };
    let mut root = col()
        .key("root")
        .w_full()
        .h_full()
        .fill(pal.page)
        .on_layout(|s: &mut State, r| s.window = [r[2], r[3]])
        .child(screen);
    if s.host == Host::Desktop {
        root = root
            .on_drop_files(|s: &mut State, files, _tx| {
                s.drop_hover = false;
                for f in files {
                    s.start_import(f.clone());
                }
            })
            .accepts_extensions(&["epub", "pdf"])
            .on_drop_hover(|s: &mut State, over, _tx| s.drop_hover = over);
        if s.drop_hover {
            layers.push(
                el().key("drop-hint")
                    .pin(Some(10.0), Some(10.0), Some(10.0), Some(10.0))
                    .radius(14.0)
                    .fill(pal.ink.alpha(0.03))
                    .border(1.5, pal.faint)
                    .border_dash(6.0, 5.0)
                    .transition(Transition::opacity())
                    .inert(),
            );
        }
    }
    root.children(layers)
}

fn button(key: &str, label: &str, pal: &Palette) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .px(16.0)
        .py(8.0)
        .radius(999.0)
        .border(1.0, pal.hairline)
        .hover(pal.ink.alpha(0.05))
        .press(pal.ink.alpha(0.09))
        .child(text(label).font(ui(14.0, 500.0)).color(pal.ink))
}

fn library(s: &State, cx: &mut Cx<State>, pal: &Palette) -> El {
    let [top, right, bottom, left] = s.insets;
    let m = margin(s.window[0]);
    let top_pad = top.max(if s.host == Host::Desktop { 30.0 } else { 0.0 }) + 28.0;
    let import = button("import", "Import", pal).on_press_tx(|s: &mut State, tx: &mut Tx| s.ask_for_book(tx));
    let header = row()
        .w_full()
        .items_center()
        .child(text("Library").font(TextStyle::new(34.0, 500.0).family(SERIF)).color(pal.ink))
        .child(el().grow(1.0))
        .gap(8.0)
        .child(button("find", "Find books", pal).on_press(|s: &mut State| s.find_books()))
        .child(import);
    let status = import_status(&s.import, pal);
    let entries = s.library.entries();

    let body = if entries.is_empty() {
        let mut empty = col()
            .key("empty")
            .grow(1.0)
            .w_full()
            .center()
            .gap(18.0)
            .pb(top_pad)
            .child(text("Nothing to read yet.").font(TextStyle::new(22.0, 400.0).family(SERIF)).color(pal.muted))
            .child(button("import-empty", "Import a book", pal).on_press_tx(|s: &mut State, tx: &mut Tx| s.ask_for_book(tx)))
            .child(account_link(s, pal));
        if s.host == Host::Desktop {
            empty = empty.child(text("or drop an EPUB or PDF here").font(ui(13.0, 400.0)).color(pal.faint));
        }
        col().w_full().grow(1.0).min_h(0.0).px(m).pl(m + left).pr(m + right).pt(top_pad).gap(18.0).child(header).children(status).child(empty)
    } else {
        let avail = s.window[0] - 2.0 * m - left - right;
        let gap = if avail < 500.0 { 20.0 } else { 32.0 };
        let cols = ((avail + gap) / (150.0 + gap)).floor().max(2.0);
        let tile_w = ((avail - gap * (cols - 1.0)) / cols).min(190.0).floor();
        let grid = row().key("shelf").w_full().flex_wrap().gaps(36.0, gap).children(entries.iter().map(|e| cover(e, s.covers.get(&e.id), tile_w, pal)));
        let content = col().w_full().px(m).pl(m + left).pr(m + right).pt(top_pad).pb(28.0 + bottom).gap(28.0).child(header).children(status).child(grid).child(account_link(s, pal));
        col().key("library").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(content)
    };
    let _ = cx;
    col().w_full().h_full().child(body)
}

/// The way into sync from the library: quiet, since most of the screen is the books.
fn account_link(s: &State, pal: &Palette) -> El {
    let label = match (s.account.session(), &s.account.status) {
        (None, _) => "Sync across devices".to_string(),
        (Some(_), SyncStatus::Running) => "Syncing…".to_string(),
        (Some(_), SyncStatus::Failed(_)) => "Sync paused".to_string(),
        (Some(session), _) => format!("Synced as {}", session.email),
    };
    row()
        .key("account")
        .role("button")
        .label(label.as_str())
        .focusable(true)
        .cursor(Cursor::Pointer)
        .py(10.0)
        .on_press(|s: &mut State| s.open_account())
        .child(text(label).font(ui(13.0, 500.0)).color(pal.muted))
}

fn primary_button(key: &str, label: &str, enabled: bool, pal: &Palette) -> El {
    let b = row()
        .key(key)
        .role("button")
        .label(label)
        .w_full()
        .justify_center()
        .py(13.0)
        .radius(999.0)
        .fill(if enabled { pal.ink } else { pal.track })
        .child(text(label).font(ui(15.0, 600.0)).color(if enabled { pal.page } else { pal.muted }));
    if enabled { b.focusable(true).cursor(Cursor::Pointer).press_scale(0.98) } else { b }
}

fn account_view(s: &State, form: &AccountForm, pal: &Palette) -> El {
    let [top, right, bottom, left] = s.insets;
    let m = margin(s.window[0]);
    let top_pad = top.max(if s.host == Host::Desktop { 30.0 } else { 0.0 }) + 28.0;
    let header = row()
        .w_full()
        .items_center()
        .child(text("Sync").font(TextStyle::new(34.0, 500.0).family(SERIF)).color(pal.ink))
        .child(el().grow(1.0))
        .child(button("account-done", "Done", pal).on_press(|s: &mut State| s.close_account()));
    let note = |words: &str| text(words.to_string()).font(ui(14.0, 400.0).lh(20.0)).color(pal.muted).wrap();
    let column = col().key("account-screen").w_full().max_w(460.0).gap(16.0);
    let error = form.error.as_deref().map(|e| text(e.to_string()).font(ui(14.0, 500.0).lh(20.0)).color(pal.ink).wrap());
    let body = match s.account.session() {
        None => {
            let intro = note("Sign in to keep your books, your place in each one and your settings the same on all your devices. Each device also keeps its own copy, so reading still works offline.");
            let enabled = !form.busy;
            let mut create = primary_button("account-create", if form.busy { "One moment…" } else { "Create account" }, enabled, pal);
            let mut sign_in = button("account-sign-in", "Sign in", pal);
            let mut column = column.child(intro);
            if s.host == Host::Desktop {
                let field = |edit| {
                    text_input(edit, ui(15.0, 400.0)).w_full().px(14.0).py(11.0).radius(10.0).border(1.0, pal.hairline)
                };
                column = column
                    .child(field(&s.account.email_field).key("account-email").single_line().placeholder("Email", pal.faint).on_edit(|s: &mut State| &mut s.account.email_field))
                    .child(field(&s.account.password_field).key("account-password").secure().placeholder("Password", pal.faint).on_edit(|s: &mut State| &mut s.account.password_field));
                if enabled {
                    create = create.on_press(|s: &mut State| s.submit_fields(true));
                    sign_in = sign_in.on_press(|s: &mut State| s.submit_fields(false));
                }
            } else if enabled {
                create = create.on_press(|s: &mut State| s.ask_for_credentials(true));
                sign_in = sign_in.on_press(|s: &mut State| s.ask_for_credentials(false));
            }
            column
                .children(error)
                .child(create)
                .child(row().w_full().justify_center().child(sign_in))
                .child(note("There’s no password reset yet, so keep your password somewhere safe."))
        }
        Some(session) => {
            let status = match &s.account.status {
                SyncStatus::Running => "Syncing…".to_string(),
                SyncStatus::Failed(why) => why.clone(),
                SyncStatus::Done { skipped } if !skipped.is_empty() => format!("Up to date, except {}, which is too large to sync.", skipped.join(", ")),
                _ => "Up to date.".to_string(),
            };
            let delete_label = if form.confirm_delete { "Press again to delete your account" } else { "Delete account" };
            let mut delete = row()
                .key("account-delete")
                .role("button")
                .label(delete_label)
                .py(10.0)
                .child(text(delete_label).font(ui(13.0, 500.0)).color(pal.muted));
            let mut sync_now = button("account-sync", "Sync now", pal);
            let mut sign_out = button("account-sign-out", "Sign out", pal);
            if !form.busy {
                delete = delete.focusable(true).cursor(Cursor::Pointer).on_press(|s: &mut State| s.delete_account());
                sync_now = sync_now.on_press(|s: &mut State| s.sync_now());
                sign_out = sign_out.on_press(|s: &mut State| s.sign_out());
            }
            let mut column = column
                .child(text(session.email.clone()).font(TextStyle::new(18.0, 500.0).family(SERIF)).color(pal.ink).truncate())
                .child(note(&status))
                .children(error)
                .child(row().gap(8.0).child(sync_now).child(sign_out));
            if form.confirm_delete {
                column = column.child(note("This removes your account and everything stored in it. The books on this device stay."));
            }
            column.child(delete)
        }
    };
    col().key("account-page").w_full().grow(1.0).min_h(0.0).px(m).pl(m + left).pr(m + right).pt(top_pad).pb(bottom + 20.0).gap(22.0).child(header).child(body)
}

fn finding_view(s: &State, finding: &Finding, pal: &Palette) -> El {
    let [top, right, bottom, left] = s.insets;
    let m = margin(s.window[0]);
    let top_pad = top.max(if s.host == Host::Desktop { 30.0 } else { 0.0 }) + 28.0;
    let header = row()
        .w_full()
        .items_center()
        .child(text("Find books").font(TextStyle::new(34.0, 500.0).family(SERIF)).color(pal.ink))
        .child(el().grow(1.0))
        .child(button("find-cancel", "Cancel", pal).on_press(|s: &mut State| s.close_finding()));
    let page = col().key("finding").w_full().grow(1.0).min_h(0.0).px(m).pl(m + left).pr(m + right).pt(top_pad).gap(18.0);
    let note = |words: String| text(words).font(ui(14.0, 400.0)).color(pal.muted).wrap();
    let candidates = match finding {
        Finding::Searching => {
            return page.child(header).child(note("Looking through your files. Nothing leaves this device.".into()));
        }
        Finding::Results(candidates) if candidates.is_empty() => {
            let words = match s.host {
                Host::Desktop => "No new books in Documents, Downloads, Desktop, Books or iCloud Drive.",
                Host::Ios => "No new books in the folders you allowed. Files still in iCloud need downloading first.",
            };
            return page.child(header).child(note(words.into())).children(folder_links(s, pal));
        }
        Finding::Results(candidates) => candidates,
    };
    let rows = candidates.iter().enumerate().map(|(at, c)| candidate_row(at, c, pal));
    let chosen = candidates.iter().filter(|c| c.chosen).count();
    let add_label = match chosen {
        0 => "Choose books to add".to_string(),
        1 => "Add 1 book".to_string(),
        n => format!("Add {n} books"),
    };
    let add = row()
        .key("find-add")
        .role("button")
        .label(add_label.as_str())
        .w_full()
        .justify_center()
        .py(13.0)
        .radius(999.0)
        .fill(if chosen > 0 { pal.ink } else { pal.track })
        .child(text(add_label.as_str()).font(ui(15.0, 600.0)).color(if chosen > 0 { pal.page } else { pal.muted }));
    let add = if chosen > 0 { add.focusable(true).cursor(Cursor::Pointer).press_scale(0.98).on_press(|s: &mut State| s.add_chosen()) } else { add };
    let found = match candidates.len() {
        1 => "Found 1 book you don’t have yet.".to_string(),
        n => format!("Found {n} books you don’t have yet."),
    };
    page.pb(bottom + 20.0)
        .child(header)
        .child(note(format!("{found} Choose the ones to add. PDFs start unchecked, since they’re often not books.")))
        .child(col().key("find-results").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).children(rows))
        .children(folder_links(s, pal))
        .child(add)
}

/// The iPhone searches only the folders the person allowed, so they can change which ones.
fn folder_links(s: &State, pal: &Palette) -> Option<El> {
    (s.host == Host::Ios).then(|| {
        row()
            .w_full()
            .justify_center()
            .gap(8.0)
            .child(button("find-add-folder", "Add a folder", pal).on_press(|s: &mut State| s.add_folder()))
            .child(button("find-forget-folders", "Forget folders", pal).on_press(|s: &mut State| s.forget_folders()))
    })
}

fn candidate_row(at: usize, c: &Candidate, pal: &Palette) -> El {
    let f = &c.found;
    let kind = match f.format {
        reader_core::Format::Epub => "EPUB",
        reader_core::Format::Pdf => "PDF",
    };
    let size = if f.bytes >= 1_000_000 { format!("{:.1} MB", f.bytes as f32 / 1e6) } else { format!("{} KB", (f.bytes / 1000).max(1)) };
    let mut detail: Vec<&str> = vec![kind];
    if let Some(author) = &f.author {
        detail.push(author);
    }
    if !f.folder.is_empty() {
        detail.push(&f.folder);
    }
    detail.push(&size);
    let mut mark = el().w(22.0).h(22.0).radius(6.0).center();
    mark = if c.chosen { mark.fill(pal.ink).child(text("\u{2713}").font(ui(14.0, 600.0)).color(pal.page)) } else { mark.border(1.5, pal.faint) };
    row()
        .key(format!("found-{at}"))
        .role("checkbox")
        .label(f.title.as_str())
        .focusable(true)
        .cursor(Cursor::Pointer)
        .w_full()
        .items_center()
        .gap(14.0)
        .py(12.0)
        .hover(pal.ink.alpha(0.03))
        .on_press(move |s: &mut State| s.toggle_candidate(at))
        .child(mark)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(text(f.title.as_str()).font(TextStyle::new(16.0, 500.0).family(SERIF).lh(21.0)).color(pal.ink).truncate())
                .child(text(detail.join(" · ")).font(ui(12.5, 400.0)).color(pal.muted).truncate()),
        )
}

fn import_status(import: &Import, pal: &Palette) -> Option<El> {
    let line = match import {
        Import::Idle => return None,
        Import::Parsing { name, queued } => {
            let more = if *queued > 0 { format!(" and {queued} more") } else { String::new() };
            row().key("import-status").child(text(format!("Importing {name}{more}…")).font(ui(14.0, 400.0)).color(pal.muted))
        }
        Import::Failed(message) => row()
            .key("import-error")
            .role("alert")
            .w_full()
            .gap(14.0)
            .items_start()
            .px(14.0)
            .py(11.0)
            .radius(10.0)
            .fill(pal.tile)
            .child(text(message.as_str()).font(ui(14.0, 400.0)).color(pal.ink).wrap().grow(1.0))
            .child(
                text("Dismiss")
                    .key("dismiss")
                    .role("button")
                    .cursor(Cursor::Pointer)
                    .font(ui(14.0, 500.0))
                    .color(pal.muted)
                    .on_press(|s: &mut State| s.import = Import::Idle),
            ),
    };
    Some(line.transition(Transition::opacity()))
}

/// A soft tint per book, from its id, at the page theme's tile lightness.
fn tint(pal: &Palette, entry: &Entry) -> Color {
    let hash = entry.id.0.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let hue = hash % 360;
    let [l, ..] = pal.tile.to_oklab();
    blend(pal.tile, rune::oklch(l, if pal.dark { 0.02 } else { 0.022 }, hue as f32), 0.7)
}

fn cover(e: &Entry, image: Option<&Image>, w: f32, pal: &Palette) -> El {
    let id = e.id.clone();
    let title_size = (w * 0.115).clamp(15.0, 20.0).round();
    let progress = e.progress();
    let picture = image.filter(|i| i.is_ready() && i.error().is_none());
    let base = col().w(w).h((w * 1.5).round()).radius(5.0).border(1.0, pal.ink.alpha(0.05)).shadow(1.0, 3.0, pal.shadow.alpha(pal.shadow.a * 0.5));
    let mut tile = match picture {
        Some(image) => base.key("cover-image").clip_rounded(5.0).fill(pal.tile).pixels_fit(image.pixels(), SurfaceFit::Fill).justify_end().padding(8.0),
        None => base.key("cover-title").fill(tint(pal, e)).padding(14.0).justify_between().child(
            text(e.title.as_str()).font(TextStyle::new(title_size, 500.0).family(SERIF).lh((title_size * 1.25).round())).color(pal.ink).wrap().max_lines(6),
        ),
    };
    if progress > 0.0 {
        let (track, done) = if picture.is_some() { (Color::BLACK.alpha(0.35), Color::WHITE.alpha(0.9)) } else { (pal.hairline, pal.muted) };
        tile = tile.child(el().key("progress").w_full().h(2.0).radius(1.0).fill(track).child(el().h(2.0).w_frac(progress.max(0.02)).radius(1.0).fill(done)));
    }
    let author = e.author.as_deref().unwrap_or(match e.format {
        reader_core::Format::Epub => "EPUB",
        reader_core::Format::Pdf => "PDF",
    });
    col()
        .key(format!("book-{}", e.id.0))
        .role("button")
        .label(e.title.as_str())
        .focusable(true)
        .cursor(Cursor::Pointer)
        .w(w)
        .gap(10.0)
        .press_scale(0.98)
        .on_press(move |s: &mut State| s.open_book(&id))
        .child(tile)
        .child(text(author).font(ui(12.5, 400.0)).color(pal.muted).truncate())
}

fn reader(s: &State, r: &Reading, cx: &mut Cx<State>, pal: &Palette, layers: &mut Vec<El>) -> El {
    let ty = Type::of(&s.settings);
    let [top, _, bottom, _] = s.insets;
    let content = match &r.flow {
        Flow::Scroll(sc) => scrolled(s, r, sc, &ty, pal),
        Flow::Paged(p) => paged(s, r, p, &ty, pal),
    };
    let progress = match &r.flow {
        Flow::Scroll(sc) => {
            let placeholder = progress_text(&r.chapter_label(), r.flow.fraction());
            text(placeholder).live_text(&sc.progress)
        }
        Flow::Paged(p) => text(format!("{} · Page {} of {}", r.chapter_label(), p.page + 1, p.pages.len().max(1))),
    };
    let progress = row()
        .key("progress-line")
        .pin(Some(0.0), None, Some(0.0), Some(0.0))
        .h(BOTTOM_LINE + bottom)
        .pb(bottom)
        .center()
        .inert()
        .fill(pal.page)
        .child(fade(pal, false).absolute(0.0, -FADE))
        .child(progress.font(ui(12.0, 400.0).features("'tnum' 1")).color(pal.faint).w(320.0).align(Align::Center));

    let bar = top_bar(r, cx, pal, top);
    if r.panel == Some(Panel::Appearance) {
        layers.push(appearance(s, pal));
    }
    let mut screen = el().key("reader").w_full().h_full().child(content).child(progress);
    if top > 0.0 {
        screen = screen.child(
            el().key("status-guard").pin(Some(0.0), Some(0.0), Some(0.0), None).h(top).fill(pal.page).inert().child(fade(pal, true).absolute(0.0, top)),
        );
    }
    screen.child(bar)
}

fn top_bar(r: &Reading, cx: &mut Cx<State>, pal: &Palette, top: f32) -> El {
    let lead = cx.titlebar_inset();
    let back = row()
        .key("back")
        .role("button")
        .label("Back to Library")
        .focusable(true)
        .cursor(Cursor::Pointer)
        .gap(6.0)
        .items_center()
        .px(10.0)
        .py(6.0)
        .radius(8.0)
        .hover(pal.ink.alpha(0.05))
        .on_press(|s: &mut State| s.close_book())
        .child(text("‹").font(ui(20.0, 400.0).lh(20.0)).color(pal.muted))
        .child(text("Library").font(ui(14.0, 500.0)).color(pal.muted));
    let aa = row()
        .key("aa")
        .role("button")
        .label("Appearance")
        .focusable(true)
        .cursor(Cursor::Pointer)
        .px(10.0)
        .py(4.0)
        .radius(8.0)
        .fill(if r.panel.is_some() { pal.ink.alpha(0.07) } else { Color::TRANSPARENT })
        .hover(pal.ink.alpha(0.05))
        .on_press(|s: &mut State| {
            let open = s.reading().and_then(|r| r.panel);
            s.set_panel(if open.is_some() { None } else { Some(Panel::Appearance) });
        })
        .child(text("Aa").font(TextStyle::new(18.0, 500.0).family(SERIF)).color(pal.ink));
    let title = text(r.book.title.as_str()).font(ui(13.0, 500.0)).color(pal.faint).truncate().align(Align::Center);
    let shown = r.chrome || r.panel.is_some();
    let mut bar = row()
        .key("top-bar")
        .pin(Some(0.0), Some(0.0), Some(0.0), None)
        .h(TOP_BAR + top)
        .pt(top)
        .pl(10.0 + lead)
        .pr(10.0)
        .items_center()
        .gap(8.0)
        .fill(pal.page)
        .child(fade(pal, true).absolute(0.0, TOP_BAR + top))
        .opacity(if shown { 1.0 } else { 0.0 })
        .opacity_motion(if shown { motion::SETTLE } else { motion::REFLOW })
        .child(back)
        .child(el().grow(1.0).min_w(0.0).px(12.0).child(title.w_full()))
        .child(aa);
    if !shown {
        bar = bar.inert();
    }
    bar
}

const FADE: f32 = 20.0;

/// A short fade from the page colour into the text, under the top bar or above the progress line.
fn fade(pal: &Palette, down: bool) -> El {
    let (a, b) = if down { (pal.page, pal.page.alpha(0.0)) } else { (pal.page.alpha(0.0), pal.page) };
    el().w_full().h(FADE).gradient(a, b).inert()
}

fn block(ty: &Type, pal: &Palette, kind: Kind, t: &str) -> El {
    let (style, inset) = ty.block(kind);
    let color = match kind {
        Kind::Break => pal.faint,
        _ => pal.ink,
    };
    let mut t = text(t).font(style).color(color).wrap();
    t = if kind == Kind::Break { t.w_full().align(Align::Center) } else { t.w_full() };
    if inset > 0.0 { el().w_full().pl(inset).child(t) } else { t }
}

fn scrolled(s: &State, r: &Reading, sc: &crate::Scroll, ty: &Type, pal: &Palette) -> El {
    let [top, _, bottom, _] = s.insets;
    let col_w = column_width(ty, s.window[0]);
    let chapter = &r.book.chapters[r.chapter];
    let mut column = col().key("column").w(col_w).gap(ty.gap).on_press(|s: &mut State| s.toggle_chrome());
    if r.chapter > 0 {
        let prev = r.chapter - 1;
        let label = r.book.chapters[prev].title.as_deref().map(display_title).unwrap_or_else(|| format!("Chapter {}", prev + 1));
        column = column.child(
            text(format!("‹ {label}"))
                .key("prev-chapter")
                .role("link")
                .cursor(Cursor::Pointer)
                .font(ui(13.0, 400.0))
                .color(pal.faint)
                .mb(ty.gap)
                .on_press(move |s: &mut State| s.go_to_chapter(prev, true)),
        );
    }
    if let Some(title) = &chapter.title {
        column = column.child(block(ty, pal, Kind::Title, &display_title(title)).mb(ty.gap));
    }
    column = column.children(chapter.blocks.iter().map(|b| {
        let (kind, t) = Kind::of(b);
        block(ty, pal, kind, t)
    }));
    if r.chapter + 1 < r.book.chapters.len() {
        let next = r.chapter + 1;
        let label = r.book.chapters[next].title.as_deref().map(display_title).unwrap_or_else(|| format!("Chapter {}", next + 1));
        column = column.child(
            col()
                .key("next-chapter")
                .role("button")
                .label(format!("Next: {label}"))
                .cursor(Cursor::Pointer)
                .w_full()
                .mt(ty.gap * 2.0)
                .pt(24.0)
                .pb(8.0)
                .gap(6.0)
                .items_center()
                .border_sides([1.0, 0.0, 0.0, 0.0], pal.hairline)
                .on_press(move |s: &mut State| s.go_to_chapter(next, false))
                .child(text("Next").font(ui(12.0, 500.0)).color(pal.faint))
                .child(text(label).font(TextStyle::new(20.0, 500.0).family(SERIF)).color(pal.ink)),
        );
    } else {
        column = column.child(text("The end").font(TextStyle::new(16.0, 400.0).family(SERIF).italic()).color(pal.faint).w_full().align(Align::Center).mt(ty.gap * 2.0));
    }
    let at = sc.at.clone();
    let progress = sc.progress.clone();
    let label = r.chapter_label();
    col()
        .key(format!("chapter-{}", r.chapter))
        .w_full()
        .h_full()
        .child(
            col()
                .key(SCROLL_KEY)
                .w_full()
                .h_full()
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_anchor()
                .on_scroll_offset(move |pos: ScrollPos| {
                    at.set(Some(pos));
                    let f = if pos.max[1] > 0.0 { pos.offset[1] / pos.max[1] } else { 1.0 };
                    progress.set(progress_text(&label, f.clamp(0.0, 1.0)));
                })
                .child(col().w_full().items_center().pt(top + TOP_BAR + 28.0).pb(bottom + BOTTOM_LINE + 48.0).child(column)),
        )
}

/// How far a swipe must travel to turn the page, in points.
const SWIPE_TURNS: f32 = 40.0;
/// How much of a swipe the page follows when there's nothing further to turn to.
const PAST_THE_END: f32 = 0.25;

fn paged(s: &State, r: &Reading, p: &crate::Paged, ty: &Type, pal: &Palette) -> El {
    let [top, _, bottom, _] = s.insets;
    let win_w = s.window[0];
    let col_w = column_width(ty, win_w);
    let left = ((win_w - col_w) / 2.0).floor();
    let along = p.heading as f32 * win_w;
    let chapter_slide = Transition::asymmetric(Transition::offset(along, 0.0), Transition::offset(-along, 0.0));
    let pages = (p.page.saturating_sub(1)..(p.page + 2).min(p.pages.len())).map(|k| {
        let x = left + (k as f32 - p.page as f32) * win_w + p.drag;
        col()
            .key(format!("page-{}-{k}", r.chapter))
            .w(col_w)
            .absolute(x.round(), 0.0)
            .gap(ty.gap)
            .motion(motion::GLIDE)
            .transition(chapter_slide)
            .children(p.pages[k].iter().map(|Piece { kind, text }| block(ty, pal, *kind, text)))
    });
    let page_box = el()
        .key("page-box")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .clip_rounded(0.0)
        .on_layout(|s: &mut State, rect| {
            if let Screen::Reading(r) = &mut s.screen {
                r.page_box = [rect[2], rect[3]];
            }
        })
        .on_drag(|s: &mut State, d: &mut rune::Drag, tx: &mut Tx| match d.phase {
            Phase::Down => {}
            Phase::Move => {
                let by = if d.dx < 0.0 { 1 } else { -1 };
                let travel = if s.can_turn(by) { d.dx } else { d.dx * PAST_THE_END };
                if let Some(Flow::Paged(p)) = s.reading_flow_mut() {
                    p.drag = travel;
                    p.heading = by;
                    tx.instant();
                }
            }
            Phase::Up => {
                let tap = d.dx.abs() < 8.0 && d.dy.abs() < 8.0;
                if tap {
                    let third = d.start_x / d.w.max(1.0);
                    if third < 1.0 / 3.0 {
                        s.queue_turn(-1, tx);
                    } else if third > 2.0 / 3.0 {
                        s.queue_turn(1, tx);
                    } else {
                        s.settle_page(tx);
                        s.toggle_chrome();
                    }
                } else if d.dx < -SWIPE_TURNS && s.can_turn(1) {
                    s.queue_turn(1, tx);
                } else if d.dx > SWIPE_TURNS && s.can_turn(-1) {
                    s.queue_turn(-1, tx);
                } else {
                    s.settle_page(tx);
                }
            }
        })
        .children(pages);
    col().key(format!("paged-{}", r.chapter)).w_full().h_full().pt(top + TOP_BAR + 12.0).pb(bottom + BOTTOM_LINE).child(page_box)
}

impl State {
    fn settle_page(&mut self, tx: &mut Tx) {
        let Some(Flow::Paged(p)) = self.reading_flow_mut() else { return };
        p.release = Some(0);
        tx.wake_after(std::time::Duration::ZERO);
    }

    fn reading_flow_mut(&mut self) -> Option<&mut Flow> {
        match &mut self.screen {
            Screen::Reading(r) => Some(&mut r.flow),
            _ => None,
        }
    }
}

fn appearance(s: &State, pal: &Palette) -> El {
    let narrow = s.window[0] < NARROW;
    let set = &s.settings;
    let section = |label: &str, control: El| col().w_full().gap(8.0).child(text(label).font(ui(11.5, 600.0).tracking(0.4)).color(pal.faint)).child(control);

    let size_step = |key: &str, label: &str, glyph: f32, steps: i32, enabled: bool| {
        row()
            .key(key)
            .role("button")
            .label(label)
            .focusable(true)
            .cursor(Cursor::Pointer)
            .w(52.0)
            .h(36.0)
            .center()
            .radius(9.0)
            .fill(pal.track)
            .hover(pal.ink.alpha(0.05))
            .press(pal.ink.alpha(0.09))
            .opacity(if enabled { 1.0 } else { 0.4 })
            .on_press(move |s: &mut State| s.change(|c| c.step_font(steps)))
            .child(text("A").font(TextStyle::new(glyph, 500.0).family(SERIF)).color(pal.ink))
    };
    let (lo, hi) = (*Settings::FONT_SIZES.start(), *Settings::FONT_SIZES.end());
    let size = row()
        .w_full()
        .items_center()
        .gap(10.0)
        .child(size_step("smaller", "Smaller text", 13.0, -1, set.font_size > lo))
        .child(text(format!("{}", set.font_size as i32)).key("font-size").font(ui(15.0, 500.0).features("'tnum' 1")).color(pal.ink).grow(1.0).align(Align::Center))
        .child(size_step("larger", "Larger text", 20.0, 1, set.font_size < hi));

    let swatches = row().key("themes").role("radiogroup").w_full().justify_between().children(PageTheme::ALL.map(|theme| {
        let them = Palette::of(theme);
        let current = theme == set.theme;
        let mut dot = el()
            .key(format!("theme-{}", theme.spec().name.to_lowercase()))
            .role("radio")
            .label(theme.spec().name)
            .checked(current)
            .focusable(true)
            .cursor(Cursor::Pointer)
            .size(34.0, 34.0)
            .radius(17.0)
            .fill(them.page)
            .border(1.0, blend(them.page, them.ink, 0.28))
            .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                tx.color_motion(motion::CALM);
                s.change(|c| c.theme = theme);
            })
            .child(el().size(10.0, 10.0).radius(5.0).absolute(12.0, 12.0).fill(them.ink));
        if current {
            dot = dot.outline(2.0, 3.0, pal.ink);
        }
        dot
    }));

    let fonts = segmented(
        "fonts",
        pal,
        FontChoice::ALL.map(|f| (f, text(f.spec().label).font(TextStyle::new(15.0, 500.0).family(label_family(f))))),
        set.font,
        |c, f| c.font = f,
    );
    let spacing = segmented(
        "spacing",
        pal,
        LineSpacing::ALL.map(|sp| {
            let gap = (sp.spec().factor - 1.0) * 7.0;
            let bars = col().gap(gap.round()).label(sp.spec().label).children((0..3).map(|_| el().w(18.0).h(2.0).radius(1.0).fill(pal.ink)));
            (sp, bars)
        }),
        set.spacing,
        |c, sp| c.spacing = sp,
    );
    let modes = segmented("modes", pal, PageMode::ALL.map(|m| (m, text(m.label()).font(ui(14.0, 500.0)))), set.mode, |c, m| c.mode = m);

    let panel = col()
        .key("appearance")
        .role("dialog")
        .label("Appearance")
        .gap(18.0)
        .padding(20.0)
        .fill(pal.raised)
        .border(1.0, pal.hairline)
        .modal(true, |s: &mut State| s.set_panel(None))
        .child(section("TEXT SIZE", size))
        .child(section("PAGE", swatches))
        .child(section("FONT", fonts))
        .child(section("LINE SPACING", spacing))
        .child(section("TURN PAGES BY", modes));
    if narrow {
        panel
            .pin(Some(0.0), None, Some(0.0), Some(0.0))
            .pb(20.0 + s.insets[2])
            .corners([18.0, 18.0, 0.0, 0.0])
            .shadow(-4.0, 30.0, pal.shadow)
            .transition(Transition::offset(0.0, 80.0))
    } else {
        panel
            .w(320.0)
            .radius(16.0)
            .shadow(12.0, 40.0, pal.shadow)
            .anchored("aa", Side::Bottom, Align::End, 8.0)
            .transition(Transition::scale(0.96))
    }
}

/// A row of choices, the current one raised. Each choice's label is its own element, so a
/// font's name can be set in that font.
fn segmented<T: Copy + PartialEq + 'static, const N: usize>(
    key: &str,
    pal: &Palette,
    options: [(T, El); N],
    current: T,
    apply: fn(&mut Settings, T),
) -> El {
    row().key(key).role("radiogroup").focus_group().w_full().h(40.0).padding(3.0).gap(3.0).radius(11.0).fill(pal.track).children(options.into_iter().enumerate().map(|(i, (value, label))| {
        let on = value == current;
        row()
            .key(format!("{key}-{i}"))
            .role("radio")
            .checked(on)
            .focusable(true)
            .cursor(Cursor::Pointer)
            .grow(1.0)
            .h_full()
            .center()
            .radius(8.0)
            .fill(if on { pal.selected } else { Color::TRANSPARENT })
            .shadow(if on { 1.0 } else { 0.0 }, if on { 3.0 } else { 0.0 }, if on { pal.shadow } else { Color::TRANSPARENT })
            .hover(pal.ink.alpha(0.04))
            .motion(motion::SNAP)
            .on_press(move |s: &mut State| s.change(|c| apply(c, value)))
            .child(label.color(if on { pal.ink } else { pal.muted }))
    }))
}
