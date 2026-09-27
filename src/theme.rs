//! Terminal color schemes: the sixteen ANSI colors and the four around them.

use crate::Rgb;

/// A terminal color scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// What `Set Theme` calls it.
    pub name: String,
    /// The terminal's background, and what the caption strip and window bar are shaded from.
    pub background: Rgb,
    /// Text in the default color.
    pub foreground: Rgb,
    /// The block cursor.
    pub cursor: Rgb,
    /// Selected text's background.
    pub selection: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then the same eight bright.
    pub ansi: [Rgb; 16],
}

/// The ANSI color names, in palette order, as a theme file spells them.
const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
];

/// Every built-in theme as `(name, [background, foreground, cursor, selection, ansi × 16])`.
#[rustfmt::skip]
const BUILT_IN: &[(&str, [u32; 20])] = &[
    ("Dracula", [
        0x282a36, 0xf8f8f2, 0xf8f8f2, 0x44475a, //
        0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, //
        0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
    ]),
    ("Catppuccin Mocha", [
        0x1e1e2e, 0xcdd6f4, 0xf5e0dc, 0x585b70, //
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
        0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ]),
    ("Catppuccin Latte", [
        0xeff1f5, 0x4c4f69, 0xdc8a78, 0xacb0be, //
        0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be, //
        0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
    ]),
    ("Tokyo Night", [
        0x1a1b26, 0xc0caf5, 0xc0caf5, 0x33467c, //
        0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
        0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
    ]),
    ("Nord", [
        0x2e3440, 0xd8dee9, 0xd8dee9, 0x434c5e, //
        0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, //
        0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
    ]),
    ("Gruvbox Dark", [
        0x282828, 0xebdbb2, 0xebdbb2, 0x504945, //
        0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, //
        0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
    ]),
    ("One Dark", [
        0x282c34, 0xabb2bf, 0x528bff, 0x3e4451, //
        0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf, //
        0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
    ]),
    ("Solarized Dark", [
        0x002b36, 0x839496, 0x93a1a1, 0x073642, //
        0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, //
        0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
    ]),
    ("Solarized Light", [
        0xfdf6e3, 0x657b83, 0x586e75, 0xeee8d5, //
        0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, //
        0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
    ]),
    ("GitHub Dark", [
        0x0d1117, 0xe6edf3, 0x2f81f7, 0x264f78, //
        0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4, //
        0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
    ]),
    ("GitHub Light", [
        0xffffff, 0x1f2328, 0x0969da, 0xb6e3ff, //
        0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, //
        0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
    ]),
    ("Monokai", [
        0x272822, 0xf8f8f2, 0xf8f8f0, 0x49483e, //
        0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2, //
        0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
    ]),
    ("Rose Pine", [
        0x191724, 0xe0def4, 0x524f67, 0x403d52, //
        0x26233a, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4, //
        0x6e6a86, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
    ]),
];

impl Default for Theme {
    fn default() -> Self {
        Theme::named("Dracula").expect("Dracula is built in")
    }
}

impl Theme {
    /// A built-in theme. Case, spaces, dashes and underscores are ignored, so `catppuccin-mocha`
    /// finds `Catppuccin Mocha`.
    ///
    /// ```
    /// use demogod::Theme;
    ///
    /// assert_eq!(Theme::named("tokyo_night").unwrap().name, "Tokyo Night");
    /// assert!(Theme::named("Nope").is_none());
    /// ```
    pub fn named(name: &str) -> Option<Theme> {
        let wanted = normalize(name);
        let (name, colors) = BUILT_IN.iter().find(|(name, _)| normalize(name) == wanted)?;
        let rgb = |index: usize| {
            let value = colors[index];
            Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
        };

        Some(Theme {
            name: name.to_string(),
            background: rgb(0),
            foreground: rgb(1),
            cursor: rgb(2),
            selection: rgb(3),
            ansi: std::array::from_fn(|index| rgb(4 + index)),
        })
    }

    /// The names of every built-in theme, for `demogod themes`.
    pub fn names() -> impl Iterator<Item = &'static str> {
        BUILT_IN.iter().map(|(name, _)| *name)
    }

    /// A theme written out as JSON, the shape VHS and xterm.js use:
    /// `{"background": "#282a36", "foreground": "#f8f8f2", "red": "#ff5555", …}`.
    ///
    /// Anything left out is taken from `base`, so a theme can be a single color changed.
    /// `purple` is read as `magenta`, and `selectionBackground` as `selection`.
    ///
    /// ```
    /// use demogod::{Rgb, Theme};
    ///
    /// let theme = Theme::from_json(r##"{"background": "#000000", "purple": "#ff00ff"}"##, Theme::default()).unwrap();
    /// assert_eq!(theme.background, Rgb(0, 0, 0));
    /// assert_eq!(theme.ansi[5], Rgb(255, 0, 255));
    /// ```
    pub fn from_json(json: &str, base: Theme) -> Result<Theme, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|error| format!("not a theme: {error}"))?;
        let object = value.as_object().ok_or("a theme is a JSON object of colors")?;
        let mut theme = base;
        if let Some(name) = object.get("name").and_then(|name| name.as_str()) {
            theme.name = name.to_string();
        } else {
            theme.name = "custom".into();
        }

        for (key, value) in object {
            if key == "name" {
                continue;
            }
            let text = value.as_str().ok_or_else(|| format!("{key} is not a string"))?;
            let color = Rgb::parse(text).ok_or_else(|| format!("{key}: not a color: {text}"))?;
            let slot = match key.as_str() {
                "background" => &mut theme.background,
                "foreground" => &mut theme.foreground,
                "cursor" | "cursorColor" | "cursorAccent" => &mut theme.cursor,
                "selection" | "selectionBackground" => &mut theme.selection,
                "purple" => &mut theme.ansi[5],
                "brightPurple" => &mut theme.ansi[13],
                other => match ANSI_NAMES.iter().position(|name| *name == other) {
                    Some(index) => &mut theme.ansi[index],
                    None => return Err(format!("no such theme color: {other}")),
                },
            };
            *slot = color;
        }

        Ok(theme)
    }

    /// Whether this is a light theme, which decides how the chrome around it is shaded.
    pub fn is_light(&self) -> bool {
        self.background.luma() > 128
    }

    /// The color an xterm-256 palette index stands for: the theme's sixteen, then the 6×6×6 cube,
    /// then the grey ramp.
    ///
    /// ```
    /// use demogod::{Rgb, Theme};
    ///
    /// let theme = Theme::default();
    /// assert_eq!(theme.indexed(1), theme.ansi[1]);
    /// assert_eq!(theme.indexed(196), Rgb(255, 0, 0));
    /// assert_eq!(theme.indexed(232), Rgb(8, 8, 8));
    /// ```
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let cube = index - 16;
                let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
                Rgb(level(cube / 36), level(cube / 6 % 6), level(cube % 6))
            }
            _ => {
                let grey = 8 + (index - 232) * 10;
                Rgb(grey, grey, grey)
            }
        }
    }
}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|character| !matches!(character, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace('é', "e")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_theme_loads_by_its_own_name() {
        for name in Theme::names() {
            assert_eq!(Theme::named(name).unwrap().name, name);
        }
    }

    #[test]
    fn light_and_dark_are_told_apart() {
        assert!(Theme::named("GitHub Light").unwrap().is_light());
        assert!(Theme::named("Catppuccin Latte").unwrap().is_light());
        assert!(!Theme::named("Dracula").unwrap().is_light());
    }

    #[test]
    fn rose_pine_can_be_spelled_with_its_accent() {
        assert_eq!(Theme::named("Rosé Pine").unwrap().name, "Rose Pine");
    }

    #[test]
    fn json_reads_every_ansi_name() {
        let json = ANSI_NAMES
            .iter()
            .enumerate()
            .map(|(index, name)| format!(r##""{name}": "#{index:02x}0000""##))
            .collect::<Vec<_>>()
            .join(",");
        let theme = Theme::from_json(&format!("{{{json}}}"), Theme::default()).unwrap();

        for (index, color) in theme.ansi.iter().enumerate() {
            assert_eq!(*color, Rgb(index as u8, 0, 0));
        }
        assert_eq!(theme.name, "custom");
    }

    #[test]
    fn json_errors_say_which_key() {
        let error = |json: &str| Theme::from_json(json, Theme::default()).unwrap_err();

        assert!(error(r##"{"red": "#12"}"##).contains("red: not a color"));
        assert!(error(r##"{"crimson": "#ff0000"}"##).contains("no such theme color: crimson"));
        assert!(error(r#"{"red": 12}"#).contains("red is not a string"));
        assert!(error("[1]").contains("JSON object"));
        assert!(error("{").contains("not a theme"));
    }

    #[test]
    fn the_color_cube_ends_at_white() {
        assert_eq!(Theme::default().indexed(231), Rgb(255, 255, 255));
        assert_eq!(Theme::default().indexed(16), Rgb(0, 0, 0));
        assert_eq!(Theme::default().indexed(255), Rgb(238, 238, 238));
    }
}
