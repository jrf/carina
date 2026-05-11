use ratatui::style::Color;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub text: Color,
    pub text_dim: Color,
    pub text_muted: Color,
    pub author: Color,
    pub highlight: Color,
    pub link: Color,
    pub date: Color,
    pub border: Color,
    pub selection: Color,
    pub popup_bg: Color,
    pub popup_border: Color,
    pub normal_bg: Color,
    pub insert_bg: Color,
    pub status_fg: Color,
}

fn default_theme() -> Theme {
    Theme {
        text: Color::Rgb(200, 211, 245),       // #c8d3f5 — fg
        text_dim: Color::Rgb(130, 139, 184),   // #828bb8 — fg_dark
        text_muted: Color::Rgb(59, 66, 97),    // #3b4261 — fg_gutter
        author: Color::Rgb(192, 153, 255),      // #c099ff — magenta
        highlight: Color::Rgb(255, 199, 119),  // #ffc777 — yellow
        link: Color::Rgb(130, 170, 255),       // #82aaff — blue
        date: Color::Rgb(195, 232, 141),       // #c3e88d — green
        border: Color::Rgb(59, 66, 97),        // #3b4261 — fg_gutter
        selection: Color::Rgb(47, 51, 77),     // #2f334d — bg_highlight
        popup_bg: Color::Rgb(30, 32, 48),      // #1e2030 — ui.popup bg
        popup_border: Color::Rgb(88, 158, 215), // #589ed7 — border_highlight
        normal_bg: Color::Rgb(130, 170, 255),  // #82aaff — blue
        insert_bg: Color::Rgb(195, 232, 141),  // #c3e88d — green
        status_fg: Color::Rgb(27, 29, 43),     // #1b1d2b — black
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThemeConfig {
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    #[serde(default)]
    pub ui: Option<UiConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UiConfig {
    pub text: Option<String>,
    pub text_dim: Option<String>,
    pub text_muted: Option<String>,
    pub author: Option<String>,
    pub highlight: Option<String>,
    pub link: Option<String>,
    pub date: Option<String>,
    pub border: Option<String>,
    pub selection: Option<String>,
    pub popup_bg: Option<String>,
    pub popup_border: Option<String>,
    pub normal_bg: Option<String>,
    pub insert_bg: Option<String>,
    pub status_fg: Option<String>,
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn resolve_color(name: &str, palette: &BTreeMap<String, String>) -> Option<Color> {
    palette.get(name).and_then(|hex| parse_hex(hex))
}

impl ThemeConfig {
    pub fn resolve(&self, base: &Theme) -> Theme {
        let p = &self.colors;
        let ui = self.ui.as_ref();

        let r = |field: Option<&Option<String>>, fallback: Color| -> Color {
            field
                .and_then(|opt| opt.as_ref())
                .and_then(|name| resolve_color(name, p))
                .unwrap_or(fallback)
        };

        Theme {
            text: r(ui.map(|u| &u.text), base.text),
            text_dim: r(ui.map(|u| &u.text_dim), base.text_dim),
            text_muted: r(ui.map(|u| &u.text_muted), base.text_muted),
            author: r(ui.map(|u| &u.author), base.author),
            highlight: r(ui.map(|u| &u.highlight), base.highlight),
            link: r(ui.map(|u| &u.link), base.link),
            date: r(ui.map(|u| &u.date), base.date),
            border: r(ui.map(|u| &u.border), base.border),
            selection: r(ui.map(|u| &u.selection), base.selection),
            popup_bg: r(ui.map(|u| &u.popup_bg), base.popup_bg),
            popup_border: r(ui.map(|u| &u.popup_border), base.popup_border),
            normal_bg: r(ui.map(|u| &u.normal_bg), base.normal_bg),
            insert_bg: r(ui.map(|u| &u.insert_bg), base.insert_bg),
            status_fg: r(ui.map(|u| &u.status_fg), base.status_fg),
        }
    }
}

pub fn load_theme(config_theme: Option<&str>) -> Theme {
    let name = config_theme.unwrap_or("tokyo-night-moon");
    let base = default_theme();

    let theme_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("grimoire")
        .join("themes");

    let theme_file = theme_dir.join(format!("{}.toml", name));
    if let Ok(contents) = std::fs::read_to_string(&theme_file)
        && let Ok(cfg) = toml::from_str::<ThemeConfig>(&contents)
    {
        return cfg.resolve(&base);
    }

    base
}
