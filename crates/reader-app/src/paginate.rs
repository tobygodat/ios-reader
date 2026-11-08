//! Breaks a chapter into pages that each fit the page box, measuring every block with the style
//! it draws in. A paragraph that doesn't fit is split between words at the last line that does,
//! so no line is ever cut in half. Rune wraps greedily, so the words of a paragraph's first `n`
//! lines are exactly the longest prefix that measures `n` lines tall.

use reader_core::Chapter;

use crate::look::{Kind, Type};

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub kind: Kind,
    pub text: String,
}

pub type Page = Vec<Piece>;

/// What a pagination was computed for; a different key means pages must be computed again.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub chapter: usize,
    pub ty: Type,
    pub size: [u32; 2],
}

fn height(ty: &Type, kind: Kind, text: &str, width: f32) -> f32 {
    let (style, inset) = ty.block(kind);
    rune::text::measure(text, &[], style, Some((width - inset).max(1.0))).1
}

pub fn paginate(chapter: &Chapter, title: Option<&str>, ty: &Type, width: f32, height_limit: f32) -> Vec<Page> {
    // Half a point of slack for rounding between measurement and layout.
    let limit = height_limit - 0.5;
    let mut queue: Vec<Piece> = Vec::new();
    if let Some(t) = title {
        queue.push(Piece { kind: Kind::Title, text: t.to_string() });
    }
    queue.extend(chapter.blocks.iter().map(|b| {
        let (kind, text) = Kind::of(b);
        Piece { kind, text: text.to_string() }
    }));
    let body_line = ty.body.line_px();

    let mut pages: Vec<Page> = Vec::new();
    let mut page: Page = Vec::new();
    let mut used = 0.0f32;
    let mut i = 0;
    while i < queue.len() {
        let piece = &queue[i];
        let gap = if page.is_empty() { 0.0 } else { ty.gap };
        let h = height(ty, piece.kind, &piece.text, width);
        let room = limit - used - gap;
        // A heading keeps at least two lines of what follows it on its page.
        let needs = if matches!(piece.kind, Kind::Heading | Kind::Title) && i + 1 < queue.len() { h + ty.gap + 2.0 * body_line } else { h };
        if needs <= room || (page.is_empty() && !piece.kind.splits()) {
            used += gap + h;
            page.push(piece.clone());
            i += 1;
            continue;
        }
        if piece.kind.splits() {
            let words: Vec<&str> = piece.text.split(' ').collect();
            let fits = |k: usize| height(ty, piece.kind, &words[..k].join(" "), width) <= room;
            let (mut lo, mut hi) = (0usize, words.len());
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                if fits(mid) { lo = mid } else { hi = mid - 1 }
            }
            // A page that is otherwise empty takes at least a word, so a paragraph taller than
            // the box still moves on.
            let take = if lo == 0 && page.is_empty() { 1 } else { lo };
            if take > 0 {
                let kind = piece.kind;
                page.push(Piece { kind, text: words[..take].join(" ") });
                let rest = words[take..].join(" ");
                if rest.is_empty() {
                    i += 1;
                } else {
                    queue[i] = Piece { kind, text: rest };
                }
            }
        }
        pages.push(std::mem::take(&mut page));
        used = 0.0;
    }
    if !page.is_empty() || pages.is_empty() {
        pages.push(page);
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_core::{Block, Settings};

    fn fonts() -> rune::text::TextSystem {
        rune::text::TextSystem::new(crate::FONTS, crate::FAMILIES)
    }

    fn chapter() -> Chapter {
        let para = |n: usize| Block::Paragraph(format!("Paragraph {n}. ") + &"Words that wrap across the line and keep on going for a while. ".repeat(6));
        Chapter { title: Some("One".into()), blocks: (0..20).map(para).collect() }
    }

    #[test]
    fn every_page_fits_and_no_word_is_lost() {
        let _fonts = fonts();
        let ty = Type::of(&Settings::default());
        let (w, h) = (360.0, 520.0);
        let c = chapter();
        let pages = paginate(&c, Some("One"), &ty, w, h);
        assert!(pages.len() > 4, "{} pages", pages.len());
        for (n, page) in pages.iter().enumerate() {
            let total: f32 = page.iter().map(|p| height(&ty, p.kind, &p.text, w)).sum::<f32>() + ty.gap * (page.len().saturating_sub(1)) as f32;
            assert!(total <= h, "page {n} is {total} tall");
        }
        let words = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let original: Vec<String> = c.blocks.iter().flat_map(|b| words(Kind::of(b).1)).collect();
        let paged: Vec<String> = pages.iter().flatten().filter(|p| p.kind != Kind::Title).flat_map(|p| words(&p.text)).collect();
        assert_eq!(paged, original);
    }

    #[test]
    fn a_split_paragraph_fills_its_page_to_the_last_whole_line() {
        let _fonts = fonts();
        let ty = Type::of(&Settings::default());
        let pages = paginate(&chapter(), None, &ty, 360.0, 520.0);
        let line = ty.body.line_px();
        let first = &pages[0];
        let used: f32 = first.iter().map(|p| height(&ty, p.kind, &p.text, 360.0)).sum::<f32>() + ty.gap * (first.len() - 1) as f32;
        assert!(520.0 - used < line + ty.gap, "only {} of 520 used", used);
    }

    #[test]
    fn a_heading_never_ends_a_page() {
        let _fonts = fonts();
        let ty = Type::of(&Settings::default());
        let mut c = chapter();
        for i in (2..20).step_by(3) {
            c.blocks.insert(i, Block::Heading { level: 2, text: "A heading".into() });
        }
        for page in paginate(&c, None, &ty, 360.0, 400.0) {
            assert_ne!(page.last().map(|p| p.kind), Some(Kind::Heading));
        }
    }
}
