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
    let state = State::open(data, Host::Desktop);
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
