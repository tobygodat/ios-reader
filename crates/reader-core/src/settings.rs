use serde::{Deserialize, Serialize};

/// How the page looks. One value for the whole app, saved between launches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
    pub const FONT_STEP: f32 = 1.0;

    /// One step larger or smaller (`steps` may be negative), clamped to `FONT_SIZES`.
    pub fn step_font(&mut self, steps: i32) {
        let size = (self.font_size + steps as f32 * Self::FONT_STEP).round();
        self.font_size = size.clamp(*Self::FONT_SIZES.start(), *Self::FONT_SIZES.end());
    }
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

/// A page theme's two authored colours, as sRGB hex. Everything else (muted text, hairlines,
/// tiles) is derived from these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeSpec {
    pub name: &'static str,
    pub page: &'static str,
    pub ink: &'static str,
    pub dark: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageTheme {
    Paper,
    Sepia,
    Sage,
    Dusk,
    Night,
}

const THEMES: [ThemeSpec; 5] = [
    ThemeSpec { name: "Paper", page: "#FBFAF7", ink: "#1F1D1A", dark: false },
    ThemeSpec { name: "Sepia", page: "#F4ECD8", ink: "#4A3F2E", dark: false },
    ThemeSpec { name: "Sage", page: "#E8EDE4", ink: "#2D3A2E", dark: false },
    ThemeSpec { name: "Dusk", page: "#2B2D33", ink: "#D9D6CF", dark: true },
    ThemeSpec { name: "Night", page: "#000000", ink: "#B8B4AC", dark: true },
];

impl PageTheme {
    pub const ALL: [PageTheme; 5] = [PageTheme::Paper, PageTheme::Sepia, PageTheme::Sage, PageTheme::Dusk, PageTheme::Night];

    pub fn spec(self) -> &'static ThemeSpec {
        &THEMES[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontSpec {
    pub label: &'static str,
    /// The family name embedded in the bundled font file.
    pub family: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    Serif,
    Sans,
    Mono,
}

const FONTS: [FontSpec; 3] = [
    FontSpec { label: "Serif", family: "Literata" },
    FontSpec { label: "Sans", family: "Inter" },
    FontSpec { label: "Mono", family: "JetBrains Mono" },
];

impl FontChoice {
    pub const ALL: [FontChoice; 3] = [FontChoice::Serif, FontChoice::Sans, FontChoice::Mono];

    pub fn spec(self) -> &'static FontSpec {
        &FONTS[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingSpec {
    pub label: &'static str,
    /// Line height as a multiple of the body size.
    pub factor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineSpacing {
    Compact,
    Normal,
    Relaxed,
}

const SPACINGS: [SpacingSpec; 3] = [
    SpacingSpec { label: "Compact", factor: 1.35 },
    SpacingSpec { label: "Normal", factor: 1.55 },
    SpacingSpec { label: "Relaxed", factor: 1.8 },
];

impl LineSpacing {
    pub const ALL: [LineSpacing; 3] = [LineSpacing::Compact, LineSpacing::Normal, LineSpacing::Relaxed];

    pub fn spec(self) -> &'static SpacingSpec {
        &SPACINGS[self as usize]
    }
}

/// How the reader moves through a book: one continuous scroll, or a screen at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageMode {
    Scroll,
    Paged,
}

impl PageMode {
    pub const ALL: [PageMode; 2] = [PageMode::Scroll, PageMode::Paged];

    pub fn label(self) -> &'static str {
        match self {
            PageMode::Scroll => "Scroll",
            PageMode::Paged => "Pages",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_steps_stay_in_range() {
        let mut s = Settings::default();
        for _ in 0..40 {
            s.step_font(1);
        }
        assert_eq!(s.font_size, 30.0);
        for _ in 0..40 {
            s.step_font(-1);
        }
        assert_eq!(s.font_size, 14.0);
    }

    #[test]
    fn every_enum_has_a_table_row() {
        assert_eq!(PageTheme::ALL.map(|t| t.spec().name), ["Paper", "Sepia", "Sage", "Dusk", "Night"]);
        assert_eq!(FontChoice::Mono.spec().family, "JetBrains Mono");
        assert!(LineSpacing::Relaxed.spec().factor > LineSpacing::Compact.spec().factor);
    }
}
