//! EPUB 2 and 3: container.xml → OPF (metadata, manifest, spine) → XHTML → blocks, with chapters
//! split where the nav (or NCX) points, which is often in the middle of a spine file.
//!
//! Content documents go through quick-xml with end-name checks off, and entities resolve through
//! the HTML table, so `&nbsp;` and unclosed `<br>` don't stop a book from opening.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::book::{Block, Chapter};

#[derive(Debug)]
pub struct Parsed {
    pub title: Option<String>,
    pub author: Option<String>,
    pub chapters: Vec<Chapter>,
}

pub fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    let mut zip = Zip::open(bytes)?;
    let container = zip.read_text("META-INF/container.xml").ok_or("it has no META-INF/container.xml")?;
    let opf_path = first_attr(&container, b"rootfile", b"full-path").ok_or("its container.xml names no package file")?;
    let opf = zip.read_text(&opf_path).ok_or_else(|| format!("its package file {opf_path} is missing"))?;
    let package = Package::parse(&opf, &dir_of(&opf_path));

    let targets = package
        .nav
        .as_ref()
        .and_then(|nav| zip.read_text(nav).map(|src| nav_targets(&src, &dir_of(nav))))
        .filter(|t| !t.is_empty())
        .or_else(|| package.ncx.as_ref().and_then(|ncx| zip.read_text(ncx).map(|src| ncx_targets(&src, &dir_of(ncx)))))
        .unwrap_or_default();

    let mut walker = Walker::default();
    for doc in &package.spine {
        let Some(src) = zip.read_text(doc) else { continue };
        let here: HashMap<&str, &str> = targets
            .iter()
            .filter(|t| &t.file == doc)
            .map(|t| (t.fragment.as_deref().unwrap_or(""), t.label.as_str()))
            .collect();
        if targets.is_empty() {
            walker.start_chapter(None);
        } else if let Some(label) = here.get("") {
            walker.start_chapter(Some(label));
        }
        walker.walk(&src, &here);
    }
    let chapters = walker.finish();
    if chapters.is_empty() {
        return Err("it has no readable text".into());
    }
    Ok(Parsed { title: package.title, author: package.author, chapters })
}

struct Zip<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
}

impl<'a> Zip<'a> {
    fn open(bytes: &'a [u8]) -> Result<Self, String> {
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "it isn't a valid EPUB (not a zip archive)".to_string())?;
        Ok(Zip { archive })
    }

    fn read_text(&mut self, path: &str) -> Option<String> {
        let name = match self.archive.index_for_name(path) {
            Some(i) => i,
            None => (0..self.archive.len()).find(|&i| self.archive.name_for_index(i).is_some_and(|n| n.as_ref().is_ok_and(|n| n.eq_ignore_ascii_case(path))))?,
        };
        let mut file = self.archive.by_index(name).ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[derive(Default)]
struct Package {
    title: Option<String>,
    author: Option<String>,
    spine: Vec<String>,
    nav: Option<String>,
    ncx: Option<String>,
}

impl Package {
    fn parse(src: &str, base: &str) -> Package {
        let mut pkg = Package::default();
        let mut manifest: HashMap<String, (String, String, String)> = HashMap::new();
        let mut spine_ids = Vec::new();
        let mut toc_id = None;
        let mut capture: Option<&'static str> = None;
        let mut buf = String::new();
        for ev in events(src) {
            match ev {
                Xml::Open(tag, attrs) | Xml::Empty(tag, attrs) if tag == "item" => {
                    let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
                    manifest.insert(get("id"), (get("href"), get("media-type"), get("properties")));
                }
                Xml::Open(tag, attrs) | Xml::Empty(tag, attrs) if tag == "itemref" => {
                    if let Some((_, id)) = attrs.iter().find(|(n, _)| n == "idref") {
                        spine_ids.push(id.clone());
                    }
                }
                Xml::Open(tag, attrs) if tag == "spine" => {
                    toc_id = attrs.iter().find(|(n, _)| n == "toc").map(|(_, v)| v.clone());
                }
                Xml::Open(tag, _) if (tag == "title" && pkg.title.is_none()) || (tag == "creator" && pkg.author.is_none()) => {
                    capture = Some(if tag == "title" { "title" } else { "creator" });
                    buf.clear();
                }
                Xml::Text(t) if capture.is_some() => buf.push_str(&t),
                Xml::Close(tag) if capture == Some(if tag == "title" { "title" } else { "creator" }) => {
                    let value = collapse(&buf);
                    if !value.is_empty() {
                        if tag == "title" { pkg.title = Some(value) } else { pkg.author = Some(value) }
                    }
                    capture = None;
                }
                _ => {}
            }
        }
        pkg.spine = spine_ids.iter().filter_map(|id| manifest.get(id)).map(|(href, _, _)| resolve(base, href).0).collect();
        pkg.nav = manifest.values().find(|(_, _, props)| props.split_whitespace().any(|p| p == "nav")).map(|(href, _, _)| resolve(base, href).0);
        pkg.ncx = toc_id
            .and_then(|id| manifest.get(&id))
            .or_else(|| manifest.values().find(|(_, mt, _)| mt == "application/x-dtbncx+xml"))
            .map(|(href, _, _)| resolve(base, href).0);
        pkg
    }
}

/// Where a table-of-contents entry points: a spine file and, usually, an element id in it.
#[derive(Debug)]
struct Target {
    file: String,
    fragment: Option<String>,
    label: String,
}

fn push_target(out: &mut Vec<Target>, base: &str, href: &str, label: &str) {
    let (file, fragment) = resolve(base, href);
    if !out.iter().any(|t| t.file == file && t.fragment == fragment) {
        out.push(Target { file, fragment, label: collapse(label) });
    }
}

/// EPUB 3: every link inside `<nav epub:type="toc">`, nested lists flattened in reading order.
fn nav_targets(src: &str, base: &str) -> Vec<Target> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_toc = false;
    let mut link: Option<(String, String)> = None;
    for ev in events(src) {
        match ev {
            Xml::Open(tag, attrs) if tag == "nav" => {
                depth += 1;
                if depth == 1 {
                    in_toc = attrs.iter().any(|(n, v)| n == "type" && v.split_whitespace().any(|t| t == "toc"));
                }
            }
            Xml::Close(tag) if tag == "nav" => {
                depth = depth.saturating_sub(1);
                if depth == 0 && in_toc {
                    break;
                }
            }
            Xml::Open(tag, attrs) if tag == "a" && in_toc => {
                link = attrs.iter().find(|(n, _)| n == "href").map(|(_, v)| (v.clone(), String::new()));
            }
            Xml::Text(t) if in_toc => {
                if let Some((_, label)) = &mut link {
                    label.push_str(&t);
                }
            }
            Xml::Close(tag) if tag == "a" => {
                if let Some((href, label)) = link.take() {
                    push_target(&mut out, base, &href, &label);
                }
            }
            _ => {}
        }
    }
    out
}

/// EPUB 2: `navPoint`s in document order, each a `navLabel/text` and a `content src`.
fn ncx_targets(src: &str, base: &str) -> Vec<Target> {
    let mut out = Vec::new();
    let mut label = String::new();
    let mut in_text = false;
    for ev in events(src) {
        match ev {
            Xml::Open(tag, _) if tag == "navPoint" => label.clear(),
            Xml::Open(tag, _) if tag == "text" => in_text = true,
            Xml::Close(tag) if tag == "text" => in_text = false,
            Xml::Text(t) if in_text => label.push_str(&t),
            Xml::Open(tag, attrs) | Xml::Empty(tag, attrs) if tag == "content" => {
                if let Some((_, href)) = attrs.iter().find(|(n, _)| n == "src") {
                    push_target(&mut out, base, href, &label);
                }
            }
            _ => {}
        }
    }
    out
}

/// One open element in a content document, and what it changed on the way in.
struct Open {
    tag: String,
    skip: bool,
    heading: Option<u8>,
    quote: bool,
    link: bool,
}

#[derive(Default)]
struct Walker {
    chapters: Vec<(Option<String>, Vec<Block>)>,
    stack: Vec<Open>,
    text: String,
    skipping: usize,
    quoting: usize,
    linking: usize,
    /// How much of `text` sits inside links, to spot contents lists.
    link_chars: usize,
}

const LINE: char = '\u{2028}';

impl Walker {
    fn start_chapter(&mut self, label: Option<&str>) {
        self.flush();
        self.chapters.push((label.map(str::to_owned), Vec::new()));
    }

    fn walk(&mut self, src: &str, targets: &HashMap<&str, &str>) {
        self.stack.clear();
        self.skipping = 0;
        self.quoting = 0;
        self.linking = 0;
        for ev in events(src) {
            match ev {
                Xml::Open(tag, attrs) => {
                    self.check_target(&attrs, targets);
                    if is_void(&tag) {
                        self.empty(&tag, &attrs);
                    } else {
                        self.open(tag, &attrs);
                    }
                }
                Xml::Empty(tag, attrs) => {
                    self.check_target(&attrs, targets);
                    self.empty(&tag, &attrs);
                }
                Xml::Close(tag) => self.close(&tag),
                Xml::Text(t) if self.skipping == 0 => {
                    if self.linking > 0 {
                        self.link_chars += t.trim().chars().count();
                    }
                    self.text.push_str(&t);
                }
                Xml::Text(_) => {}
            }
        }
        self.flush();
    }

    fn check_target(&mut self, attrs: &[(String, String)], targets: &HashMap<&str, &str>) {
        if self.skipping > 0 {
            return;
        }
        let label = attrs.iter().filter(|(n, _)| n == "id" || n == "name").find_map(|(_, v)| targets.get(v.as_str()));
        if let Some(label) = label {
            let heading = self.stack.iter().rev().find_map(|o| o.heading);
            self.start_chapter(Some(label));
            // An anchor inside a heading starts the chapter at that heading.
            if heading.is_some() {
                self.text.clear();
            }
        }
    }

    fn open(&mut self, tag: String, attrs: &[(String, String)]) {
        let skip = self.skipping > 0 || skipped(&tag, attrs);
        let heading = heading_level(&tag);
        let quote = tag == "blockquote";
        let link = tag == "a" && attrs.iter().any(|(n, _)| n == "href");
        if !skip && (is_block(&tag) || heading.is_some()) {
            self.flush();
        }
        if !skip && matches!(tag.as_str(), "td" | "th") {
            self.text.push(' ');
        }
        if skip {
            self.skipping += 1;
        }
        if quote {
            self.quoting += 1;
        }
        if link {
            self.linking += 1;
        }
        self.stack.push(Open { tag, skip, heading, quote, link });
    }

    fn close(&mut self, tag: &str) {
        let Some(at) = self.stack.iter().rposition(|o| o.tag == tag) else { return };
        while self.stack.len() > at {
            let top = self.stack.last().expect("stack is longer than `at`");
            if !top.skip && (is_block(&top.tag) || top.heading.is_some()) {
                self.flush();
            }
            let open = self.stack.pop().expect("stack is longer than `at`");
            if open.skip {
                self.skipping -= 1;
            }
            if open.quote {
                self.quoting -= 1;
            }
            if open.link {
                self.linking -= 1;
            }
        }
    }

    fn empty(&mut self, tag: &str, attrs: &[(String, String)]) {
        if self.skipping > 0 || skipped(tag, attrs) {
            return;
        }
        match tag {
            "br" => self.text.push(LINE),
            "hr" => {
                self.flush();
                self.push(Block::Break);
            }
            // A drop cap is often an image whose alt is the letter. Other images' alt text
            // describes a picture the reader doesn't show.
            "img" | "image" => {
                if let Some((_, alt)) = attrs.iter().find(|(n, v)| n == "alt" && (1..=2).contains(&v.trim().chars().count())) {
                    self.text.push_str(alt);
                }
            }
            _ => {}
        }
    }

    fn flush(&mut self) {
        let raw = std::mem::take(&mut self.text);
        let link_chars = std::mem::take(&mut self.link_chars);
        let text = collapse_lines(&raw);
        let chars = text.chars().filter(|c| !c.is_whitespace()).count();
        // A block that is mostly links is the book's own contents list.
        if text.is_empty() || link_chars * 10 >= chars * 8 {
            return;
        }
        let heading = self.stack.iter().rev().find_map(|o| o.heading);
        let block = if let Some(level) = heading {
            Block::Heading { level, text }
        } else if is_scene_break(&text) {
            Block::Break
        } else if self.quoting > 0 {
            Block::Quote(text)
        } else {
            Block::Paragraph(text)
        };
        self.push(block);
    }

    fn push(&mut self, block: Block) {
        if self.chapters.is_empty() {
            self.chapters.push((None, Vec::new()));
        }
        let blocks = &mut self.chapters.last_mut().expect("a chapter was just ensured").1;
        if block == Block::Break && matches!(blocks.last(), None | Some(Block::Break)) {
            return;
        }
        blocks.push(block);
    }

    fn finish(mut self) -> Vec<Chapter> {
        self.flush();
        self.chapters
            .into_iter()
            .filter_map(|(label, mut blocks)| {
                while blocks.last() == Some(&Block::Break) {
                    blocks.pop();
                }
                // The chapter's own heading becomes its title, so the page doesn't show it twice.
                let title = match blocks.first() {
                    Some(Block::Heading { text, .. }) => {
                        let t = text.replace('\n', " ");
                        blocks.remove(0);
                        Some(t)
                    }
                    _ => label.filter(|l| !l.is_empty()),
                };
                (!blocks.is_empty()).then_some(Chapter { title, blocks })
            })
            .collect()
    }
}

fn heading_level(tag: &str) -> Option<u8> {
    match tag.as_bytes() {
        [b'h', d @ b'1'..=b'6'] => Some(d - b'0'),
        _ => None,
    }
}

fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div" | "blockquote" | "li" | "ul" | "ol" | "dd" | "dt" | "dl" | "pre" | "section" | "article" | "header"
            | "footer" | "aside" | "main" | "tr" | "table" | "body" | "address" | "center"
    )
}

fn is_void(tag: &str) -> bool {
    matches!(tag, "br" | "hr" | "img" | "meta" | "link" | "input" | "col" | "wbr" | "source")
}

/// Things that aren't the book's running text: page furniture, image captions, the Project
/// Gutenberg licence boilerplate.
fn skipped(tag: &str, attrs: &[(String, String)]) -> bool {
    if matches!(tag, "head" | "script" | "style" | "nav" | "figure" | "figcaption" | "svg" | "math" | "title" | "rt") {
        return true;
    }
    attrs.iter().any(|(name, value)| match name.as_str() {
        "class" => value.split_whitespace().any(|c| {
            matches!(c, "caption" | "pg-boilerplate" | "x-ebookmaker-pageno" | "pagenum" | "pageno" | "figcenter" | "figleft" | "figright" | "footnote-ref")
        }),
        "type" => value.split_whitespace().any(|t| t == "pagebreak"),
        "role" => matches!(value.as_str(), "figure" | "doc-pagebreak"),
        "id" => matches!(value.as_str(), "pg-header" | "pg-footer"),
        _ => false,
    })
}

fn is_scene_break(text: &str) -> bool {
    text.chars().all(|c| matches!(c, '*' | '⁂' | '·' | '•' | '~' | '#') || c.is_whitespace()) && text.chars().any(|c| !c.is_whitespace())
}

/// Collapses runs of whitespace to one space, keeping `<br>`s as newlines.
fn collapse_lines(raw: &str) -> String {
    raw.split(LINE).map(collapse).collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
}

pub(crate) fn collapse(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for word in raw.split(|c: char| c.is_whitespace() && c != '\u{a0}') {
        if !word.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

fn dir_of(path: &str) -> String {
    path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
}

/// An href relative to `base`, as a normalized archive path and its fragment.
fn resolve(base: &str, href: &str) -> (String, Option<String>) {
    let (path, fragment) = match href.split_once('#') {
        Some((p, f)) => (p, Some(percent_decode(f))),
        None => (href, None),
    };
    let path = percent_decode(path);
    let mut parts: Vec<&str> = if path.starts_with('/') { Vec::new() } else { base.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    (parts.join("/"), fragment.filter(|f| !f.is_empty()))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A forgiving XML event: names without namespace prefixes, text with every entity resolved.
enum Xml {
    Open(String, Vec<(String, String)>),
    Empty(String, Vec<(String, String)>),
    Close(String),
    Text(String),
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .map(|a| (local(a.key.as_ref()), html_escape::decode_html_entities(&*a.value).into_owned()))
        .collect()
}

/// Every event in `src`, stopping quietly at the first error so a damaged file keeps what read.
fn events(src: &str) -> Vec<Xml> {
    let mut reader = Reader::from_str(src);
    let config = reader.config_mut();
    config.check_end_names = false;
    config.allow_unmatched_ends = true;
    config.trim_text(false);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => out.push(Xml::Open(local(e.name().as_ref()), attrs(&e))),
            Ok(Event::Empty(e)) => out.push(Xml::Empty(local(e.name().as_ref()), attrs(&e))),
            Ok(Event::End(e)) => out.push(Xml::Close(local(e.name().as_ref()))),
            Ok(Event::Text(t)) => out.push(Xml::Text(t.to_string())),
            Ok(Event::CData(t)) => out.push(Xml::Text(t.to_string())),
            Ok(Event::GeneralRef(r)) => {
                let name = r.to_string();
                out.push(Xml::Text(html_escape::decode_html_entities(&format!("&{name};")).into_owned()));
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

fn first_attr(src: &str, tag: &[u8], attr: &[u8]) -> Option<String> {
    let tag = String::from_utf8_lossy(tag);
    let attr = String::from_utf8_lossy(attr);
    events(src).into_iter().find_map(|ev| match ev {
        Xml::Open(t, a) | Xml::Empty(t, a) if t == tag => a.into_iter().find(|(n, _)| *n == attr).map(|(_, v)| v),
        _ => None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A small EPUB built in memory: `files` are (path, contents) besides the mimetype.
    pub(crate) fn build(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("mimetype", opts.compression_method(zip::CompressionMethod::Stored)).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        for (path, body) in files {
            zip.start_file(*path, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    pub(crate) fn sample() -> Vec<u8> {
        build(&[
            (
                "META-INF/container.xml",
                r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#,
            ),
            (
                "OPS/book.opf",
                r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>A Small Book</dc:title><dc:creator>Ann Author</dc:creator></metadata>
                <manifest><item id="nav" href="nav.xhtml" properties="nav"/><item id="a" href="text/one.xhtml"/><item id="b" href="text/two%20b.xhtml"/></manifest>
                <spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#,
            ),
            (
                "OPS/nav.xhtml",
                r#"<html><body><nav epub:type="toc"><ol><li><a href="text/one.xhtml">Opening</a></li>
                <li><a href="text/one.xhtml#c2">Second</a><ol><li><a href="text/two%20b.xhtml#c3">Third</a></li></ol></li></ol></nav></body></html>"#,
            ),
            (
                "OPS/text/one.xhtml",
                r#"<html><head><title>x</title><style>p{}</style></head><body>
                <h1>Chapter&nbsp;One</h1><p>It was a  bright
                cold day&mdash;and the clocks<br>were striking.</p>
                <div class="caption"><p>An image caption</p></div>
                <p>* * *</p><blockquote><p>A quoted line.</p></blockquote>
                <h2 id="c2">Chapter Two</h2><p><img alt="T" src="t.png"/>he second &amp; last &#8220;part&#8221;.</p></body></html>"#,
            ),
            ("OPS/text/two b.xhtml", r#"<html><body><p>Before the anchor.</p><h2><a id="c3"/>Three</h2><p>Third text.</p><hr/></body></html>"#),
        ])
    }

    #[test]
    fn splits_chapters_at_nav_anchors_and_reads_blocks() {
        let p = parse(&sample()).unwrap();
        assert_eq!(p.title.as_deref(), Some("A Small Book"));
        assert_eq!(p.author.as_deref(), Some("Ann Author"));
        let titles: Vec<_> = p.chapters.iter().map(|c| c.title.as_deref().unwrap()).collect();
        assert_eq!(titles, ["Chapter\u{a0}One", "Chapter Two", "Three"]);
        let one = &p.chapters[0].blocks;
        assert_eq!(one[0], Block::Paragraph("It was a bright cold day—and the clocks\nwere striking.".into()));
        assert_eq!(one[1], Block::Break);
        assert_eq!(one[2], Block::Quote("A quoted line.".into()));
        assert_eq!(p.chapters[1].blocks, [Block::Paragraph("The second & last “part”.".into()), Block::Paragraph("Before the anchor.".into())]);
        assert_eq!(p.chapters[2].blocks, [Block::Paragraph("Third text.".into())]);
    }

    #[test]
    fn falls_back_to_one_chapter_per_spine_file_without_a_toc() {
        let epub = build(&[
            ("META-INF/container.xml", r#"<container><rootfiles><rootfile full-path="c.opf"/></rootfiles></container>"#),
            ("c.opf", r#"<package><manifest><item id="a" href="a.html"/><item id="b" href="b.html"/></manifest><spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#),
            ("a.html", "<html><body><h1>First</h1><p>One &hellip; &copy;</body></html>"),
            ("b.html", "<html><body><p>Two</p></body></html>"),
        ]);
        let p = parse(&epub).unwrap();
        assert_eq!(p.title, None);
        assert_eq!(p.chapters.len(), 2);
        assert_eq!(p.chapters[0].title.as_deref(), Some("First"));
        assert_eq!(p.chapters[0].blocks, [Block::Paragraph("One … ©".into())]);
        assert_eq!(p.chapters[1].title, None);
    }

    #[test]
    fn rejects_files_that_are_not_epubs() {
        assert!(parse(b"not a zip").is_err());
        assert!(parse(&build(&[("x.txt", "hi")])).unwrap_err().contains("container.xml"));
    }

    #[test]
    fn resolves_relative_hrefs() {
        assert_eq!(resolve("OPS/text", "../img/a%20b.png#x"), ("OPS/img/a b.png".into(), Some("x".into())));
        assert_eq!(resolve("", "a.html"), ("a.html".into(), None));
    }
}
