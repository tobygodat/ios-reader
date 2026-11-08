//! Text PDFs: each page's text layer, with running headers, footers and page numbers dropped,
//! lines joined back into paragraphs and hyphenated line breaks rejoined. Chapters follow the
//! PDF's outline when it has one, else runs of pages.

use std::collections::HashMap;

use crate::book::{Block, Chapter};

#[derive(Debug)]
pub struct Parsed {
    pub title: Option<String>,
    pub author: Option<String>,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PdfError {
    Unreadable(String),
    NoText,
}

/// Pages grouped into one section when the PDF has no outline.
const PAGES_PER_SECTION: usize = 10;

pub fn parse(bytes: &[u8]) -> Result<Parsed, PdfError> {
    let doc = pdf_extract::Document::load_mem(bytes).map_err(|e| PdfError::Unreadable(e.to_string()))?;
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes))
        .map_err(|_| PdfError::Unreadable("its text couldn't be read".into()))?
        .map_err(|e| PdfError::Unreadable(e.to_string()))?;
    let ink: usize = pages.iter().map(|p| p.chars().filter(|c| !c.is_whitespace()).count()).sum();
    if pages.is_empty() || ink < 40 * pages.len().min(5) {
        return Err(PdfError::NoText);
    }
    let pages = strip_furniture(&pages);
    let outline = outline(&doc, pages.len());
    let sections: Vec<(Option<String>, std::ops::Range<usize>)> = if outline.is_empty() {
        (0..pages.len())
            .step_by(PAGES_PER_SECTION)
            .map(|start| {
                let end = (start + PAGES_PER_SECTION).min(pages.len());
                let title = (pages.len() > PAGES_PER_SECTION).then(|| format!("Pages {}–{}", start + 1, end));
                (title, start..end)
            })
            .collect()
    } else {
        outline
    };
    let chapters = sections
        .into_iter()
        .filter_map(|(title, range)| {
            let blocks = paragraphs(&pages[range]);
            (!blocks.is_empty()).then_some(Chapter { title, blocks })
        })
        .collect::<Vec<_>>();
    if chapters.is_empty() {
        return Err(PdfError::NoText);
    }
    Ok(Parsed { title: info(&doc, b"Title"), author: info(&doc, b"Author"), chapters })
}

fn info(doc: &pdf_extract::Document, key: &[u8]) -> Option<String> {
    let info = doc.trailer.get(b"Info").ok()?;
    let dict = match info {
        pdf_extract::Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        pdf_extract::Object::Dictionary(d) => d,
        _ => return None,
    };
    let value = dict.get(key).ok()?;
    let text = pdf_extract::decode_text_string(value).ok()?;
    let text = crate::epub::collapse(&text);
    (!text.is_empty()).then_some(text)
}

/// Top-level outline entries as page ranges (0-based), when they are in page order.
fn outline(doc: &pdf_extract::Document, pages: usize) -> Vec<(Option<String>, std::ops::Range<usize>)> {
    let Ok(toc) = doc.get_toc() else { return Vec::new() };
    let top = toc.toc.iter().map(|e| e.level).min().unwrap_or(1);
    let mut starts: Vec<(String, usize)> = Vec::new();
    for e in toc.toc.iter().filter(|e| e.level == top) {
        let page = e.page.saturating_sub(1).min(pages.saturating_sub(1));
        match starts.last() {
            Some((_, prev)) if page < *prev => return Vec::new(),
            Some((_, prev)) if page == *prev => {}
            _ => starts.push((crate::epub::collapse(&e.title), page)),
        }
    }
    if starts.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if starts[0].1 > 0 {
        out.push((None, 0..starts[0].1));
    }
    for (i, (title, start)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(pages, |(_, s)| *s);
        out.push((Some(title.clone()).filter(|t| !t.is_empty()), *start..end));
    }
    out
}

/// A line with each run of digits blanked and spacing collapsed, so "Page 3" and "Page 14" match.
fn shape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c.is_ascii_digit() {
            if !out.ends_with('#') {
                out.push('#');
            }
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    crate::epub::collapse(&out)
}

fn is_page_number(line: &str) -> bool {
    let t = line.trim().trim_matches(|c: char| matches!(c, '-' | '–' | '—' | '[' | ']' | '(' | ')') || c.is_whitespace());
    let t = t.strip_prefix("Page").or_else(|| t.strip_prefix("page")).unwrap_or(t).trim();
    if t.is_empty() {
        return false;
    }
    let digits = t.len() <= 4 && t.chars().all(|c| c.is_ascii_digit());
    let roman = t.len() <= 6 && t.chars().all(|c| matches!(c.to_ascii_lowercase(), 'i' | 'v' | 'x' | 'l' | 'c'));
    let of = t.split_once(" of ").is_some_and(|(a, b)| a.trim().chars().all(|c| c.is_ascii_digit()) && b.trim().chars().all(|c| c.is_ascii_digit()));
    digits || roman || of
}

/// Each page's lines without its running header and footer: lines among a page's first or last
/// two that recur (digits aside) on many pages, and bare page numbers.
fn strip_furniture(pages: &[String]) -> Vec<Vec<String>> {
    const EDGE: usize = 2;
    let lines: Vec<Vec<&str>> = pages.iter().map(|p| p.lines().map(str::trim_end).collect()).collect();
    let edges = |page: &Vec<&str>| -> Vec<usize> {
        let filled: Vec<usize> = (0..page.len()).filter(|&i| !page[i].trim().is_empty()).collect();
        let mut picked: Vec<usize> = filled.iter().take(EDGE).chain(filled.iter().rev().take(EDGE)).copied().collect();
        picked.sort_unstable();
        picked.dedup();
        picked
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for page in &lines {
        let mut seen: Vec<String> = edges(page).into_iter().map(|i| shape(page[i])).collect();
        seen.sort();
        seen.dedup();
        for s in seen {
            *counts.entry(s).or_default() += 1;
        }
    }
    let threshold = (pages.len() / 2).max(3);
    lines
        .iter()
        .map(|page| {
            let edge = edges(page);
            page.iter()
                .enumerate()
                .filter(|(i, l)| {
                    let at_edge = edge.contains(i);
                    !(at_edge && (is_page_number(l) || counts.get(&shape(l)).is_some_and(|&n| pages.len() >= 3 && n >= threshold)))
                })
                .map(|(_, l)| l.to_string())
                .collect()
        })
        .collect()
}

/// Joins wrapped lines into paragraphs. A paragraph ends at a blank line, or at a line that ends
/// a sentence well short of the usual line length.
fn paragraphs(pages: &[Vec<String>]) -> Vec<Block> {
    let lines: Vec<&str> = pages.iter().flat_map(|p| p.iter().map(String::as_str)).collect();
    let mut lengths: Vec<usize> = lines.iter().map(|l| l.trim().chars().count()).filter(|&n| n > 0).collect();
    lengths.sort_unstable();
    let typical = lengths.get(lengths.len() * 3 / 4).copied().unwrap_or(60);

    let mut blocks = Vec::new();
    let mut para = String::new();
    let flush = |para: &mut String, blocks: &mut Vec<Block>| {
        let text = crate::epub::collapse(para);
        para.clear();
        if !text.is_empty() {
            blocks.push(Block::Paragraph(text));
        }
    };
    for line in lines {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut para, &mut blocks);
            continue;
        }
        let hyphenated = para.ends_with('-') && t.starts_with(|c: char| c.is_lowercase()) && para[..para.len() - 1].ends_with(|c: char| c.is_alphabetic());
        if hyphenated || para.ends_with('\u{ad}') {
            para.pop();
        } else if !para.is_empty() {
            para.push(' ');
        }
        para.push_str(t);
        let ends_sentence = t.ends_with(['.', '!', '?', ':', '"', '”', '’', ')']);
        if ends_sentence && t.chars().count() * 10 < typical * 8 {
            flush(&mut para, &mut blocks);
        }
    }
    flush(&mut para, &mut blocks);
    blocks
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pdf_extract::{Dictionary, Document, Object, Stream, content::Content, content::Operation, dictionary};

    /// A text PDF with one Helvetica text line per entry of each page.
    pub(crate) fn build(pages: &[Vec<&str>]) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let mut kids = Vec::new();
        for lines in pages {
            let mut ops = vec![Operation::new("BT", vec![]), Operation::new("Tf", vec!["F1".into(), 11.into()]), Operation::new("TL", vec![14.into()])];
            ops.push(Operation::new("Td", vec![72.into(), 760.into()]));
            for line in lines {
                ops.push(Operation::new("Tj", vec![Object::string_literal(*line)]));
                ops.push(Operation::new("T*", vec![]));
            }
            ops.push(Operation::new("ET", vec![]));
            let content = Content { operations: ops };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id });
            kids.push(page_id.into());
        }
        let count = kids.len() as i64;
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        let info_id = doc.add_object(dictionary! { "Title" => Object::string_literal("Generated Paper"), "Author" => Object::string_literal("P. Writer") });
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set("Info", info_id);
        let _ = Dictionary::new();
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn rebuilds_paragraphs_without_headers_footers_or_hyphens() {
        let words = ["apple", "birch", "cedar"];
        let lines: Vec<Vec<String>> = (0..3)
            .map(|n| {
                let w = words[n];
                vec![
                    "A Journal of Things".to_string(),
                    format!("The {w} line of a long paragraph that wraps across"),
                    "several lines of the page and keeps on going with a hyph-".to_string(),
                    format!("enated word in the middle of the {w}."),
                    String::new(),
                    format!("A second {w} paragraph begins here and runs for a line or"),
                    format!("two before the {w} ends."),
                    (n + 1).to_string(),
                ]
            })
            .collect();
        let page = |n: usize| lines[n].iter().map(String::as_str).collect::<Vec<_>>();
        let pdf = build(&[page(0), page(1), page(2)]);
        let p = parse(&pdf).unwrap();
        assert_eq!(p.title.as_deref(), Some("Generated Paper"));
        assert_eq!(p.author.as_deref(), Some("P. Writer"));
        let blocks = &p.chapters[0].blocks;
        assert_eq!(
            blocks[0],
            Block::Paragraph("The apple line of a long paragraph that wraps across several lines of the page and keeps on going with a hyphenated word in the middle of the apple.".into())
        );
        assert_eq!(blocks[1], Block::Paragraph("A second apple paragraph begins here and runs for a line or two before the apple ends.".into()));
        assert_eq!(blocks.len(), 6, "{blocks:#?}");
        assert!(blocks.iter().all(|b| !matches!(b, Block::Paragraph(t) if t.contains("Journal"))));
    }

    #[test]
    fn a_pdf_without_text_is_refused() {
        let pdf = build(&[vec![], vec![]]);
        assert_eq!(parse(&pdf).err(), Some(PdfError::NoText));
    }

    #[test]
    fn page_numbers_are_recognised() {
        for s in ["12", " - 7 - ", "Page 3", "xiv", "[4]", "3 of 10"] {
            assert!(is_page_number(s), "{s}");
        }
        for s in ["1984 was a year", "Chapter 2"] {
            assert!(!is_page_number(s), "{s}");
        }
    }
}
