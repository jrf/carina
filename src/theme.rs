use ratatui::style::Color;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub text: Color,
    pub text_dim: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub warm: Color,
    pub border: Color,
    pub selection: Color,
}

pub fn builtin(name: &str) -> Theme {
    match name {
        "catppuccin-mocha" => Theme {
            text: Color::Rgb(205, 214, 244),       // #cdd6f4
            text_dim: Color::Rgb(166, 173, 200),   // #a6adc8
            text_muted: Color::Rgb(88, 91, 112),   // #585b70
            accent: Color::Rgb(137, 180, 250),     // #89b4fa
            warm: Color::Rgb(249, 226, 175),       // #f9e2af
            border: Color::Rgb(69, 71, 90),        // #45475a
            selection: Color::Rgb(49, 50, 68),     // #313244
        },
        "catppuccin-frappe" => Theme {
            text: Color::Rgb(198, 208, 245),       // #c6d0f5
            text_dim: Color::Rgb(165, 173, 206),   // #a5adce
            text_muted: Color::Rgb(98, 104, 128),  // #626880
            accent: Color::Rgb(140, 170, 238),     // #8caaee
            warm: Color::Rgb(229, 200, 144),       // #e5c890
            border: Color::Rgb(65, 69, 89),        // #414559
            selection: Color::Rgb(48, 52, 70),     // #303446
        },
        "rose-pine" => Theme {
            text: Color::Rgb(224, 222, 244),       // #e0def4
            text_dim: Color::Rgb(144, 140, 170),   // #908caa
            text_muted: Color::Rgb(110, 106, 134), // #6e6a86
            accent: Color::Rgb(196, 167, 231),     // #c4a7e7
            warm: Color::Rgb(246, 193, 119),       // #f6c177
            border: Color::Rgb(57, 53, 82),        // #393552
            selection: Color::Rgb(38, 35, 58),     // #26233a
        },
        "gruvbox" => Theme {
            text: Color::Rgb(235, 219, 178),       // #ebdbb2
            text_dim: Color::Rgb(168, 153, 132),   // #a89984
            text_muted: Color::Rgb(102, 92, 84),   // #665c54
            accent: Color::Rgb(131, 165, 152),     // #83a598
            warm: Color::Rgb(250, 189, 47),        // #fabd2f
            border: Color::Rgb(80, 73, 69),        // #504945
            selection: Color::Rgb(60, 56, 54),     // #3c3836
        },
        "kanagawa" => Theme {
            text: Color::Rgb(220, 215, 186),       // #dcd7ba
            text_dim: Color::Rgb(149, 147, 140),   // #95938c
            text_muted: Color::Rgb(84, 84, 88),    // #545458
            accent: Color::Rgb(126, 156, 216),     // #7e9cd8
            warm: Color::Rgb(226, 194, 114),       // #e2c272
            border: Color::Rgb(54, 54, 59),        // #36363b
            selection: Color::Rgb(42, 42, 47),     // #2a2a2f
        },
        // tokyo-night-moon is the default
        _ => Theme {
            text: Color::Rgb(200, 211, 245),       // #c8d3f5
            text_dim: Color::Rgb(99, 109, 166),    // #636da6
            text_muted: Color::Rgb(59, 66, 97),    // #3b4261
            accent: Color::Rgb(130, 170, 255),     // #82aaff
            warm: Color::Rgb(255, 199, 119),       // #ffc777
            border: Color::Rgb(69, 71, 90),        // #45475a
            selection: Color::Rgb(47, 51, 77),     // #2f334d
        },
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
    pub accent: Option<String>,
    pub warm: Option<String>,
    pub border: Option<String>,
    pub selection: Option<String>,
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
            accent: r(ui.map(|u| &u.accent), base.accent),
            warm: r(ui.map(|u| &u.warm), base.warm),
            border: r(ui.map(|u| &u.border), base.border),
            selection: r(ui.map(|u| &u.selection), base.selection),
        }
    }
}

pub fn load_theme(config_theme: Option<&str>) -> Theme {
    let name = config_theme.unwrap_or("tokyo-night-moon");

    let theme_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("carina")
        .join("themes");

    let theme_file = theme_dir.join(format!("{}.toml", name));
    if theme_file.exists()
        && let Ok(contents) = std::fs::read_to_string(&theme_file)
        && let Ok(cfg) = toml::from_str::<ThemeConfig>(&contents)
    {
        let base = builtin(name);
        return cfg.resolve(&base);
    }

    builtin(name)
}
