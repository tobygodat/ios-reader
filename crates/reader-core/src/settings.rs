use serde::{Deserialize, Serialize};

/// How the page looks. One value for the whole app, saved between launches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Body text size in points, kept within `FONT_SIZES`.
    pub font_size: f32,
    pub theme: PageTheme,
    pub font: FontChoice,
    pub spacing: LineSpacing,
    pub mode: PageMode,
}

impl Settings {
    pub const FONT_SIZES: std::ops::RangeInclusive<f32> = 14.0..=30.0;
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            font_size: 19.0,
            theme: PageTheme::Paper,
            font: FontChoice::Serif,
            spacing: LineSpacing::Normal,
            mode: PageMode::Scroll,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageTheme {
    Paper,
    Sepia,
    Sage,
    Dusk,
    Night,
}

impl PageTheme {
    pub const ALL: [PageTheme; 5] = [PageTheme::Paper, PageTheme::Sepia, PageTheme::Sage, PageTheme::Dusk, PageTheme::Night];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    Serif,
    Sans,
    Mono,
}

impl FontChoice {
    pub const ALL: [FontChoice; 3] = [FontChoice::Serif, FontChoice::Sans, FontChoice::Mono];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineSpacing {
    Compact,
    Normal,
    Relaxed,
}

impl LineSpacing {
    pub const ALL: [LineSpacing; 3] = [LineSpacing::Compact, LineSpacing::Normal, LineSpacing::Relaxed];
}

/// How the reader moves through a book: one continuous scroll, or a screen at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageMode {
    Scroll,
    Paged,
}
