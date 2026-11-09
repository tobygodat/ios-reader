use std::io::Write;
use std::path::{Path, PathBuf};

use reader_core::{FontChoice, PageMode, PageTheme};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::*;

/// A two-chapter EPUB with enough text to fill several pages.
fn write_epub(dir: &Path) -> PathBuf {
    let para = |n: usize| format!("<p>Paragraph {n}. {}</p>", "It was the kind of evening that stays with a person, long and blue and quiet. ".repeat(5));
    let chapter = |title: &str| format!("<html><body><h1>{title}</h1>{}</body></html>", (0..14).map(para).collect::<String>());
    let files = [
        ("META-INF/container.xml".to_string(), r#"<container><rootfiles><rootfile full-path="b.opf"/></rootfiles></container>"#.to_string()),
        (
            "b.opf".to_string(),
            r#"<package xmlns:dc="dc"><metadata><dc:title>An Evening</dc:title><dc:creator>A. Writer</dc:creator></metadata><manifest><item id="1" href="1.html"/><item id="2" href="2.html"/></manifest><spine><itemref idref="1"/><itemref idref="2"/></spine></package>"#.to_string(),
        ),
        ("1.html".to_string(), chapter("The First")),
        ("2.html".to_string(), chapter("The Second")),
    ];
    let path = dir.join("evening.epub");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, body) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

fn runtime(data: &Path) -> Runtime<State> {
    runtime_on(data, Host::Desktop)
}

fn runtime_on(data: &Path, host: Host) -> Runtime<State> {
    let state = State::open(data, host);
    let mut rt = Runtime::with_text(state, view::view, (900.0, 700.0), Env::new(false, 2.0), rune::text::TextSystem::new(FONTS, FAMILIES));
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn settle(rt: &mut Runtime<State>, frames: usize) {
    for _ in 0..frames {
        rt.frame(1.0 / 60.0);
    }
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt, 3);
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent { key, mods: Mods::default() });
    settle(rt, 3);
}

fn wait_for_import(rt: &mut Runtime<State>) {
    for _ in 0..600 {
        if rt.state.import == Import::Idle && !rt.state.library.entries().is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        rt.frame(1.0 / 60.0);
    }
    panic!("import never finished: {:?}", rt.state.import);
}

#[test]
fn import_open_restyle_and_return_to_the_library() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let book = write_epub(tmp.path());
    let mut rt = runtime(&data);

    click(&mut rt, "import-empty");
    assert!(rt.file_dialog().is_some(), "Import opens the file dialog");
    rt.resolve_file_dialog(Some(book.clone())).expect("the dialog is open");
    settle(&mut rt, 2);
    wait_for_import(&mut rt);
    let id = rt.state.library.entries()[0].id.clone();
    assert_eq!(rt.state.library.entries()[0].title, "An Evening");
    assert!(data.join("library.json").exists() && data.join("books").join(format!("{}.json", id.0)).exists());

    click(&mut rt, &format!("book-{}", id.0));
    assert!(matches!(rt.state.screen, Screen::Reading(_)), "a tap on the cover opens the book");

    click(&mut rt, "aa");
    assert_eq!(rt.state.reading().unwrap().panel, Some(Panel::Appearance));
    click(&mut rt, "larger");
    click(&mut rt, "larger");
    assert_eq!(rt.state.settings.font_size, 21.0);
    click(&mut rt, "theme-dusk");
    assert_eq!(rt.state.settings.theme, PageTheme::Dusk);
    click(&mut rt, "fonts-1");
    assert_eq!(rt.state.settings.font, FontChoice::Sans);
    click(&mut rt, "modes-1");
    assert_eq!(rt.state.settings.mode, PageMode::Paged);
    let saved: reader_core::Settings = serde_json::from_slice(&std::fs::read(data.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved, rt.state.settings, "every change is saved");

    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.reading().unwrap().panel, None, "Escape closes the panel first");
    settle(&mut rt, 30);
    let pages = match &rt.state.reading().unwrap().flow {
        Flow::Paged(p) => p.pages.len(),
        Flow::Scroll(_) => panic!("still scrolling"),
    };
    assert!(pages > 2, "the chapter is paginated: {pages} pages");
    key(&mut rt, Key::Right);
    key(&mut rt, Key::Right);
    let at = rt.state.reading().unwrap().position();
    assert_eq!(at.chapter, 0);
    assert!((at.fraction - 2.0 / pages as f32).abs() < 1e-4, "two pages in: {at:?}");
    settle(&mut rt, 40);
    assert!(rt.find("page-0-2").is_some(), "the third page shows");

    key(&mut rt, Key::Escape);
    assert!(matches!(rt.state.screen, Screen::Library), "Escape goes back to the library");
    let reopened = reader_core::Library::open(&data).unwrap();
    assert_eq!(reopened.entries()[0].position, at, "the position was saved");

    click(&mut rt, &format!("book-{}", id.0));
    match &rt.state.reading().unwrap().flow {
        Flow::Paged(p) => assert_eq!(p.page, 2, "reopening lands on the same page"),
        Flow::Scroll(_) => panic!("mode not kept"),
    }
}

#[test]
fn importing_the_same_file_twice_keeps_one_book() {
    let tmp = tempfile::tempdir().unwrap();
    let book = write_epub(tmp.path());
    let mut rt = runtime(&tmp.path().join("data"));
    import(&mut rt.state, &book);
    wait_for_import(&mut rt);
    import(&mut rt.state, &book);
    settle(&mut rt, 1);
    wait_for_import(&mut rt);
    assert_eq!(rt.state.library.entries().len(), 1);
}

#[test]
fn a_bad_file_shows_why_in_the_library() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("notes.txt");
    std::fs::write(&bad, "not a book").unwrap();
    let mut rt = runtime(&tmp.path().join("data"));
    import(&mut rt.state, &bad);
    for _ in 0..200 {
        if matches!(rt.state.import, Import::Failed(_)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(rt.state.import, Import::Failed("“notes.txt” isn’t an EPUB or a PDF.".into()));
    settle(&mut rt, 2);
    assert!(rt.find("import-error").is_some());
    click(&mut rt, "dismiss");
    assert_eq!(rt.state.import, Import::Idle);
}

#[test]
fn ios_import_asks_the_host() {
    let tmp = tempfile::tempdir().unwrap();
    let state = State::open(tmp.path(), Host::Ios);
    let mut rt = Runtime::with_text(state, view::view, (390.0, 844.0), Env::new(false, 3.0), rune::text::TextSystem::new(FONTS, FAMILIES));
    rt.poll = Some(poll);
    settle(&mut rt, 2);
    assert!(rt.find("import-empty").is_some());
    click(&mut rt, "import-empty");
    assert_eq!(take_request(&mut rt.state), Some(HostRequest::PickBook));
    assert_eq!(take_request(&mut rt.state), None);
    assert!(rt.file_dialog().is_none());
}

#[test]
fn every_scene_builds() {
    for (name, scene) in samples::SCENES {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = State::open(tmp.path(), Host::Desktop);
        scene(&mut state);
        let mut rt = Runtime::with_text(state, view::view, (1100.0, 760.0), Env::new(false, 2.0), rune::text::TextSystem::new(FONTS, FAMILIES));
        rt.poll = Some(poll);
        settle(&mut rt, 4);
        assert!(rt.find("root").is_some(), "{name}");
    }
}

/// A book open in paged mode on its first page, ready to be swiped.
fn paged_book(data: &Path, dir: &Path) -> Runtime<State> {
    let book = write_epub(dir);
    let mut rt = runtime(data);
    click(&mut rt, "import-empty");
    rt.resolve_file_dialog(Some(book)).expect("the dialog is open");
    settle(&mut rt, 2);
    wait_for_import(&mut rt);
    let id = rt.state.library.entries()[0].id.clone();
    click(&mut rt, &format!("book-{}", id.0));
    rt.update(|s, _| s.change(|c| c.mode = PageMode::Paged));
    settle(&mut rt, 30);
    rt
}

/// How far a page's center sits from where it comes to rest, each frame after a release.
fn travel(rt: &mut Runtime<State>, key: &str, frames: usize) -> Vec<f32> {
    let rest = |rt: &mut Runtime<State>| rt.find(key).map(|(x, _)| x);
    let mut xs = Vec::new();
    for _ in 0..frames {
        rt.frame(1.0 / 60.0);
        xs.extend(rest(rt));
    }
    let last = *xs.last().expect("the page is on screen");
    xs.iter().map(|x| x - last).collect()
}

#[test]
fn a_swiped_page_slides_home_instead_of_jumping() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rt = paged_book(&tmp.path().join("data"), tmp.path());
    let (x, y) = rt.find("page-0-0").expect("the first page shows");

    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x - 60.0, y);
    settle(&mut rt, 2);
    rt.pointer_move(x - 120.0, y);
    rt.pointer_up(x - 130.0, y);

    let from_rest = travel(&mut rt, "page-0-1", 60);
    assert!(matches!(&rt.state.reading().unwrap().flow, Flow::Paged(p) if p.page == 1), "the swipe turned the page");
    let moving = from_rest.iter().filter(|d| d.abs() > 4.0).count();
    assert!(moving >= 5, "the page should travel over several frames, got {moving}: {from_rest:?}");
    assert!(from_rest.windows(2).all(|w| w[1].abs() <= w[0].abs() + 0.5), "and head home without jumping back: {from_rest:?}");
}

#[test]
fn a_search_offers_books_and_adds_only_the_chosen_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let shelf = tmp.path().join("shelf");
    std::fs::create_dir_all(shelf.join("novels")).unwrap();
    write_epub(&shelf.join("novels"));
    std::fs::write(shelf.join("receipt.pdf"), b"%PDF-1.7 not really a book").unwrap();
    let mut rt = runtime(&data);

    rt.update(|s, _| s.search(vec![shelf.clone()]));
    assert!(matches!(rt.state.screen, Screen::Finding(Finding::Searching)));
    for _ in 0..600 {
        rt.frame(1.0 / 60.0);
        if matches!(rt.state.screen, Screen::Finding(Finding::Results(_))) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    settle(&mut rt, 2);
    let Screen::Finding(Finding::Results(found)) = &rt.state.screen else { panic!("the search never finished") };
    let summary: Vec<_> = found.iter().map(|c| (c.found.title.as_str(), c.chosen)).collect();
    assert_eq!(summary, [("An Evening", true), ("receipt", false)], "EPUBs start chosen, PDFs don't");
    assert!(rt.state.library.entries().is_empty(), "nothing is added until the reader asks");

    click(&mut rt, "found-1");
    click(&mut rt, "found-1");
    click(&mut rt, "find-add");
    assert!(matches!(rt.state.screen, Screen::Library));
    wait_for_import(&mut rt);
    let titles: Vec<_> = rt.state.library.entries().iter().map(|e| e.title.clone()).collect();
    assert_eq!(titles, ["An Evening"], "the unchosen PDF stays out");
}

#[test]
fn a_search_left_early_doesnt_pull_the_reader_back() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rt = runtime(&tmp.path().join("data"));
    rt.update(|s, _| s.search(vec![tmp.path().join("empty")]));
    rt.update(|s, _| s.close_finding());
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(rt.state.screen, Screen::Library));
}

#[test]
fn an_iphone_search_leaves_the_folders_to_the_host_and_lists_books_from_all_of_them() {
    let tmp = tempfile::tempdir().unwrap();
    let (first, second) = (tmp.path().join("first"), tmp.path().join("second"));
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    write_epub(&first);
    std::fs::write(second.join("paper.pdf"), b"%PDF-1.7 a paper").unwrap();
    let mut rt = runtime_on(&tmp.path().join("data"), Host::Ios);

    rt.update(|s, _| s.find_books());
    assert_eq!(take_request(&mut rt.state), Some(HostRequest::FindBooks));
    assert!(matches!(rt.state.screen, Screen::Library), "the host may need to ask first");

    rt.update(|s, _| search_folders(s, vec![first.clone(), second.clone()]));
    wait_until(&mut rt, "the search", |s| matches!(s.screen, Screen::Finding(Finding::Results(_))));
    let Screen::Finding(Finding::Results(found)) = &rt.state.screen else { unreachable!() };
    let titles: Vec<_> = found.iter().map(|c| c.found.title.as_str()).collect();
    assert_eq!(titles, ["An Evening", "paper"]);

    click(&mut rt, "find-add-folder");
    assert_eq!(take_request(&mut rt.state), Some(HostRequest::AddFolder));
    assert!(matches!(rt.state.screen, Screen::Finding(Finding::Results(_))), "the list stays up while the person picks");

    click(&mut rt, "find-forget-folders");
    assert_eq!(take_request(&mut rt.state), Some(HostRequest::ForgetFolders));
    assert!(matches!(rt.state.screen, Screen::Library));
}

#[test]
fn a_mac_search_has_no_folder_links() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rt = runtime(&tmp.path().join("data"));
    rt.update(|s, _| s.search(vec![tmp.path().join("empty")]));
    wait_until(&mut rt, "the search", |s| matches!(s.screen, Screen::Finding(Finding::Results(_))));
    assert!(rt.find("find-add-folder").is_none());
}

#[test]
fn the_mac_searches_the_usual_folders() {
    let folders = desktop_folders(Path::new("/Users/r"));
    assert!(folders.contains(&PathBuf::from("/Users/r/Downloads")));
    assert!(folders.iter().any(|f| f.ends_with("com~apple~CloudDocs")));
}

fn wait_until(rt: &mut Runtime<State>, what: &str, done: impl Fn(&State) -> bool) {
    for _ in 0..1000 {
        rt.frame(1.0 / 60.0);
        if done(&rt.state) {
            settle(rt, 2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("{what} never happened");
}

#[test]
fn nothing_syncs_until_someone_signs_in_and_a_failed_sign_in_explains_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let mut rt = runtime(&data);
    assert!(rt.state.account.session().is_none());
    settle(&mut rt, 10);
    assert_eq!(rt.state.account.status, SyncStatus::Idle, "signed out, no sync starts");
    assert!(!data.join("account.json").exists());

    rt.state.account.set_api("http://127.0.0.1:1");
    click(&mut rt, "account");
    assert!(matches!(rt.state.screen, Screen::Account(_)));
    rt.update(|s, _| s.submit_credentials("a@b.co".into(), "correct horse".into(), false));
    wait_until(&mut rt, "the failure", |s| matches!(&s.screen, Screen::Account(f) if f.error.is_some()));
    let Screen::Account(form) = &rt.state.screen else { panic!("left the account screen") };
    assert!(form.error.as_deref().unwrap().contains("Couldn’t reach"), "{:?}", form.error);
    assert!(!form.busy);
    assert!(rt.state.account.session().is_none());
    key(&mut rt, Key::Escape);
    assert!(matches!(rt.state.screen, Screen::Library));
}
