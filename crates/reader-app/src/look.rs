//! What the settings look like on screen: colours derived from the page theme's two authored
//! colours, and the text styles every block is set in. The reader view and the paginator both
//! read styles from here, so a measured page is laid out exactly as it draws.

use reader_core::{Block, FontChoice, PageTheme, Settings};
use rune::{Color, Family, TextStyle};

pub const SERIF: Family = Family::Named("Literata");
pub const SANS: Family = Family::Named("Inter");

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub page: Color,
    pub ink: Color,
    /// Secondary text: the book title in the bar, authors.
    pub muted: Color,
    /// The quietest text: progress, hints.
    pub faint: Color,
    pub hairline: Color,
    /// A cover tile or a control's track, a step off the page.
    pub tile: Color,
    /// The appearance panel, lifted off the page.
    pub raised: Color,
    pub shadow: Color,
    pub dark: bool,
}

/// `t` of the way from `a` to `b` in OKLab, so steps look even on light and dark pages.
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    let (x, y) = (a.to_oklab(), b.to_oklab());
    Color::from_oklab(std::array::from_fn(|i| x[i] + (y[i] - x[i]) * t))
}

impl Palette {
    pub fn of(theme: PageTheme) -> Palette {
        let spec = theme.spec();
        let page = rune::color::hex(spec.page).expect("theme table holds valid hex");
        let ink = rune::color::hex(spec.ink).expect("theme table holds valid hex");
        Palette {
            page,
            ink,
            muted: blend(page, ink, 0.66),
            faint: blend(page, ink, 0.55),
            hairline: blend(page, ink, 0.14),
            tile: blend(page, ink, if spec.dark { 0.08 } else { 0.055 }),
            raised: if spec.dark { blend(page, ink, 0.07) } else { blend(page, Color::WHITE, 0.6) },
            shadow: Color::BLACK.alpha(if spec.dark { 0.45 } else { 0.12 }),
            dark: spec.dark,
        }
    }
}

/// Every text style the reader sets, for one settings value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub body: TextStyle,
    pub quote: TextStyle,
    pub heading: TextStyle,
    pub title: TextStyle,
    pub rule: TextStyle,
    /// Space between blocks.
    pub gap: f32,
    /// A quote's left indent.
    pub indent: f32,
    /// The reading column's widest measure.
    pub measure: f32,
}

impl Type {
    pub fn of(s: &Settings) -> Type {
        let size = s.font_size;
        let family = Family::Named(s.font.spec().family);
        let lh = (size * s.spacing.spec().factor).round();
        let body = TextStyle::new(size, 400.0).family(family).lh(lh);
        let heading_size = (size * 1.2).round();
        let title_size = (size * 1.65).round();
        Type {
            body,
            quote: body.italic(),
            heading: TextStyle::new(heading_size, 600.0).family(family).lh((heading_size * 1.3).round()),
            title: TextStyle::new(title_size, 500.0).family(SERIF).lh((title_size * 1.25).round()),
            rule: body,
            gap: (lh * 0.6).round(),
            indent: (size * 1.4).round(),
            measure: (size * 34.0).round(),
        }
    }

    /// The style a block is set in, and how far it is inset from the column's left edge.
    pub fn block(&self, kind: Kind) -> (TextStyle, f32) {
        match kind {
            Kind::Title => (self.title, 0.0),
            Kind::Heading => (self.heading, 0.0),
            Kind::Paragraph => (self.body, 0.0),
            Kind::Quote => (self.quote, self.indent),
            Kind::Break => (self.rule, 0.0),
        }
    }
}

/// What a piece of a page is, for its style. A chapter's title is set apart from headings
/// inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Title,
    Heading,
    Paragraph,
    Quote,
    Break,
}

pub const RULE: &str = "*\u{2003}*\u{2003}*";

impl Kind {
    pub fn of(block: &Block) -> (Kind, &str) {
        match block {
            Block::Heading { text, .. } => (Kind::Heading, text),
            Block::Paragraph(t) => (Kind::Paragraph, t),
            Block::Quote(t) => (Kind::Quote, t),
            Block::Break => (Kind::Break, RULE),
        }
    }

    pub fn splits(self) -> bool {
        matches!(self, Kind::Paragraph | Kind::Quote)
    }
}

/// A chapter title as the reader shows it: "CHAPTER II." becomes "Chapter II".
pub fn display_title(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('.').trim();
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    let shouting = letters.len() > 3 && letters.iter().all(|c| c.is_uppercase());
    if !shouting {
        return t.to_string();
    }
    t.split(' ')
        .map(|w| {
            let roman = !w.is_empty() && w.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M' | '.' | ','));
            if roman {
                return w.to_string();
            }
            let mut cs = w.chars();
            match cs.next() {
                Some(first) => first.to_uppercase().chain(cs.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The family a font choice's own label is set in, for the segmented control.
pub fn label_family(font: FontChoice) -> Family {
    Family::Named(font.spec().family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shouting_titles_are_calmed() {
        assert_eq!(display_title("CHAPTER II."), "Chapter II");
        assert_eq!(display_title("Chapter I."), "Chapter I");
        assert_eq!(display_title("THE FULL LICENSE"), "The Full License");
        assert_eq!(display_title("Pages 1–10"), "Pages 1–10");
    }

    #[test]
    fn muted_text_sits_between_page_and_ink() {
        for theme in PageTheme::ALL {
            let p = Palette::of(theme);
            let l = |c: Color| c.to_oklab()[0];
            let (page, ink, muted) = (l(p.page), l(p.ink), l(p.muted));
            assert!((page - muted).abs() < (page - ink).abs() && (muted - ink).abs() < (page - ink).abs(), "{theme:?}");
        }
    }
}
