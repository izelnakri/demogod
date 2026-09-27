//! Everything `Set` can change, with the value it has when nothing does.

use std::path::PathBuf;
use std::time::Duration;

use crate::{Rgb, Theme};

/// How the recording is timed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Timing {
    /// Every step lasts as long as the tape says, however long it really took: `Type` is its
    /// length times the typing speed, `Wait` is `WaitDuration`, a keypress is instant. The same
    /// tape gives the same film on a fast machine and a slow one, twice in a row.
    #[default]
    Tape,
    /// Every step lasts as long as it really took, the way VHS records.
    Real,
}

/// The title bar drawn above the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowBar {
    /// No bar: the terminal starts at the top edge.
    #[default]
    None,
    /// Red, yellow and green dots on the left, like macOS.
    Colorful,
    /// The same dots, on the right.
    ColorfulRight,
    /// Three outlined dots on the left.
    Rings,
    /// Three outlined dots on the right.
    RingsRight,
}

/// A width, either in pixels or as a share of the whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// This many pixels.
    Pixels(u32),
    /// This percentage of the width available.
    Percent(f32),
}

impl Length {
    /// The width this comes to, out of `total`.
    pub fn of(self, total: u32) -> u32 {
        match self {
            Length::Pixels(pixels) => pixels.min(total),
            Length::Percent(percent) => (total as f32 * percent / 100.0).round() as u32,
        }
    }
}

/// Where a GIF starts, as VHS writes it: a frame number, a percentage, or a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopOffset {
    /// This many frames in, at the film's framerate: `Set LoopOffset 5`.
    Frames(u32),
    /// This far through: `Set LoopOffset 50%`.
    Percent(f32),
    /// This long in: `Set LoopOffset 2.5s`.
    Time(Duration),
}

impl LoopOffset {
    /// How far into a film of `length`, at `framerate`, this is.
    pub fn within(self, length: Duration, framerate: u32) -> Duration {
        let offset = match self {
            LoopOffset::Frames(frames) => Duration::from_secs(frames as u64).div_f64(framerate.max(1) as f64),
            LoopOffset::Percent(percent) => length.mul_f64(percent as f64 / 100.0),
            LoopOffset::Time(time) => time,
        };
        offset.min(length)
    }
}

/// Every setting a tape can change with `Set`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Settings {
    /// `Set Shell "zsh"`: the shell the terminal runs. `bash`, `zsh`, `fish` and `sh` get a clean
    /// prompt and no rc files; anything else is run as given.
    pub shell: String,
    /// `Set Directory "../app"`: where the shell starts, relative to the tape.
    pub directory: PathBuf,
    /// `Set Width 1200`: the whole film's width in pixels.
    pub width: u32,
    /// `Set Height 600`: the whole film's height in pixels.
    pub height: u32,
    /// `Set Columns 80`: the terminal is this many columns wide, and the film as wide as that
    /// takes, whatever `Width` says.
    pub columns: Option<u16>,
    /// `Set Rows 24`: the terminal is this many rows high, and the film as high as that takes.
    pub rows: Option<u16>,
    /// `Set FontSize 16`.
    pub font_size: f32,
    /// `Set FontFamily "Fira Code"`: an installed font by name, a `.ttf`/`.otf` path, or a
    /// comma-separated list to fall back through. JetBrains Mono is built in and always last.
    pub font_family: String,
    /// `Set LineHeight 1.2`: rows are this many times the font size apart.
    pub line_height: f32,
    /// `Set LetterSpacing 0`: extra pixels between columns.
    pub letter_spacing: f32,
    /// `Set Theme "Tokyo Night"`, or a JSON object of colors.
    pub theme: Theme,
    /// `Set Padding 20`: pixels between the window's edge and the text inside it.
    pub padding: u32,
    /// `Set Margin 0`: pixels between the film's edge and the windows.
    pub margin: u32,
    /// `Set MarginFill "#6b50ff"`: the color of the margin.
    pub margin_fill: Rgb,
    /// `Set WindowBar Colorful`.
    pub window_bar: WindowBar,
    /// `Set WindowBarSize 40`: the bar's height in pixels; twice the font size unless set.
    pub window_bar_size: Option<u32>,
    /// `Set BorderRadius 8`: rounded window corners, in pixels.
    pub border_radius: u32,
    /// `Set TypingSpeed 50ms`: the time between two typed characters. Can change mid-tape.
    pub typing_speed: Duration,
    /// `Set Framerate 50`: the most frames a second the film has. Frames are only written when
    /// something changes, so a still screen costs nothing.
    pub framerate: u32,
    /// `Set PlaybackSpeed 1.0`: 2 plays twice as fast, 0.5 half as fast.
    pub playback_speed: f32,
    /// `Set CursorBlink true`.
    pub cursor_blink: bool,
    /// `Set WaitTimeout 15s`: how long a `Wait` or a `Click` looks before it fails the recording.
    /// Can change mid-tape.
    pub wait_timeout: Duration,
    /// `Set WaitPattern /done/`: what a bare `Wait` waits for, instead of the prompt coming back.
    /// Can change mid-tape.
    pub wait_pattern: Option<crate::Pattern>,
    /// `Set LoopOffset 50%`: where in the film a GIF starts, and loops back to.
    pub loop_offset: LoopOffset,
    /// `Set Timing "tape"` or `"real"`.
    pub timing: Timing,
    /// `Set WaitDuration 1s`: how long every `Wait` lasts in the film under tape timing.
    pub wait_duration: Duration,
    /// `Set PaneWidth 45%`: the browser or image pane's share of the width, when there is a
    /// terminal beside it.
    pub pane_width: Length,
    /// `Set Prompt "❯ "`: the prompt the shell shows.
    pub prompt: String,
    /// `Set BrowserZoom 0.8`: the page is laid out this many times wider than the pane and scaled
    /// down to fit it, so a desktop layout stays a desktop layout in a narrow pane.
    pub browser_zoom: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shell: default_shell().into(),
            directory: PathBuf::from("."),
            width: 1200,
            height: 600,
            columns: None,
            rows: None,
            font_size: 16.0,
            font_family: String::new(),
            line_height: 1.25,
            letter_spacing: 0.0,
            theme: Theme::default(),
            padding: 20,
            margin: 0,
            margin_fill: Rgb(0x6b, 0x50, 0xff),
            window_bar: WindowBar::None,
            window_bar_size: None,
            border_radius: 0,
            typing_speed: Duration::from_millis(50),
            framerate: 50,
            playback_speed: 1.0,
            cursor_blink: false,
            wait_timeout: Duration::from_secs(15),
            wait_pattern: None,
            loop_offset: LoopOffset::Frames(0),
            timing: Timing::Tape,
            wait_duration: Duration::from_secs(1),
            pane_width: Length::Percent(45.0),
            prompt: "❯ ".into(),
            browser_zoom: 1.0,
        }
    }
}

/// Settings that shape each step as it is read, and so may change between two of them.
pub(crate) const STEP_SETTINGS: &[&str] = &["TypingSpeed", "WaitTimeout", "WaitPattern"];

/// Every name `Set` knows, for "did you mean" when it is given one it does not.
pub(crate) const NAMES: &[&str] = &[
    "Shell",
    "Directory",
    "Width",
    "Height",
    "Columns",
    "Rows",
    "FontSize",
    "FontFamily",
    "LineHeight",
    "LetterSpacing",
    "Theme",
    "Padding",
    "Margin",
    "MarginFill",
    "WindowBar",
    "WindowBarSize",
    "BorderRadius",
    "TypingSpeed",
    "Framerate",
    "PlaybackSpeed",
    "CursorBlink",
    "WaitTimeout",
    "WaitPattern",
    "LoopOffset",
    "Timing",
    "WaitDuration",
    "PaneWidth",
    "Prompt",
    "BrowserZoom",
];

impl Settings {
    /// Applies one `Set <name> <value>`, or says why it cannot.
    ///
    /// ```
    /// use demogod::Settings;
    ///
    /// let mut settings = Settings::default();
    /// settings.set("FontSize", "22").unwrap();
    /// settings.set("Theme", "Nord").unwrap();
    /// assert_eq!(settings.font_size, 22.0);
    /// assert_eq!(settings.theme.name, "Nord");
    /// assert!(settings.set("FontSize", "huge").is_err());
    /// ```
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        match name {
            "Shell" => self.shell = non_empty(value)?,
            "Directory" => self.directory = PathBuf::from(non_empty(value)?),
            "Width" => self.width = pixels(value, 16)?,
            "Height" => self.height = pixels(value, 16)?,
            "Columns" => self.columns = Some(number(value, 10.0, 1000.0)? as u16),
            "Rows" => self.rows = Some(number(value, 2.0, 500.0)? as u16),
            "FontSize" => self.font_size = number(value, 4.0, 200.0)?,
            "FontFamily" => self.font_family = value.to_string(),
            "LineHeight" => self.line_height = number(value, 0.5, 4.0)?,
            "LetterSpacing" => self.letter_spacing = number(value, -10.0, 100.0)?,
            "Theme" => self.theme = theme(value)?,
            "Padding" => self.padding = pixels(value, 0)?,
            "Margin" => self.margin = pixels(value, 0)?,
            "MarginFill" => self.margin_fill = color(value)?,
            "WindowBar" => self.window_bar = window_bar(value)?,
            "WindowBarSize" => self.window_bar_size = Some(pixels(value, 0)?),
            "BorderRadius" => self.border_radius = pixels(value, 0)?,
            "TypingSpeed" => self.typing_speed = duration(value)?,
            "Framerate" => self.framerate = number(value, 1.0, 100.0)? as u32,
            "PlaybackSpeed" => self.playback_speed = number(value, 0.01, 100.0)?,
            "CursorBlink" => self.cursor_blink = boolean(value)?,
            "WaitTimeout" => self.wait_timeout = duration(value)?,
            "WaitPattern" => self.wait_pattern = Some(crate::tape::pattern_from(value)?),
            "LoopOffset" => self.loop_offset = loop_offset(value)?,
            "Timing" => self.timing = timing(value)?,
            "WaitDuration" => self.wait_duration = duration(value)?,
            "PaneWidth" => self.pane_width = length(value)?,
            "Prompt" => self.prompt = value.to_string(),
            "BrowserZoom" => self.browser_zoom = number(value, 0.1, 4.0)?,
            _ => return Err(format!("no such setting: {name}")),
        }

        Ok(())
    }
}

/// The shell a tape gets when it does not say: PowerShell on Windows, bash elsewhere.
fn default_shell() -> &'static str {
    if cfg!(windows) { "powershell" } else { "bash" }
}

/// `500ms`, `2s`, `1.5s`, `1m`, or a bare number of seconds the way VHS reads one.
pub(crate) fn duration(value: &str) -> Result<Duration, String> {
    let split = value.find(|character: char| character.is_ascii_alphabetic()).unwrap_or(value.len());
    let (amount, unit) = value.split_at(split);
    let amount: f64 = amount.parse().map_err(|_| format!("not a duration: {value}"))?;
    let seconds = match unit {
        "ms" => amount / 1000.0,
        "s" | "" => amount,
        "m" => amount * 60.0,
        _ => return Err(format!("not a duration: {value}")),
    };
    if !(0.0..=86_400.0).contains(&seconds) {
        return Err(format!("not a duration: {value}"));
    }

    Ok(Duration::from_secs_f64(seconds))
}

fn non_empty(value: &str) -> Result<String, String> {
    if value.trim().is_empty() { Err("expected a value".into()) } else { Ok(value.to_string()) }
}

fn number(value: &str, min: f32, max: f32) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(number) if (min..=max).contains(&number) => Ok(number),
        Ok(_) => Err(format!("{value} is out of range: {min} to {max}")),
        Err(_) => Err(format!("not a number: {value}")),
    }
}

fn pixels(value: &str, min: u32) -> Result<u32, String> {
    let value = value.strip_suffix("px").unwrap_or(value);
    match value.parse::<u32>() {
        Ok(pixels) if (min..=8192).contains(&pixels) => Ok(pixels),
        Ok(_) => Err(format!("{value} is out of range: {min} to 8192")),
        Err(_) => Err(format!("not a whole number of pixels: {value}")),
    }
}

fn length(value: &str) -> Result<Length, String> {
    match value.strip_suffix('%') {
        Some(percent) => Ok(Length::Percent(number(percent, 1.0, 99.0)?)),
        None => Ok(Length::Pixels(pixels(value, 16)?)),
    }
}

fn loop_offset(value: &str) -> Result<LoopOffset, String> {
    if let Some(percent) = value.strip_suffix('%') {
        return Ok(LoopOffset::Percent(number(percent, 0.0, 100.0)?));
    }
    if let Ok(frames) = value.parse::<u32>() {
        return Ok(LoopOffset::Frames(frames));
    }
    duration(value).map(LoopOffset::Time).map_err(|_| format!("not a loop offset: {value} (try 5, 50% or 2s)"))
}

fn boolean(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, not {value}")),
    }
}

fn color(value: &str) -> Result<Rgb, String> {
    Rgb::parse(value).ok_or_else(|| format!("not a color: {value} (try \"#6b50ff\")"))
}

fn theme(value: &str) -> Result<Theme, String> {
    if value.trim_start().starts_with('{') {
        return Theme::from_json(value, Theme::default());
    }

    Theme::named(value)
        .ok_or_else(|| format!("no such theme: {value} (one of: {})", Theme::names().collect::<Vec<_>>().join(", ")))
}

fn window_bar(value: &str) -> Result<WindowBar, String> {
    Ok(match value {
        "None" | "" => WindowBar::None,
        "Colorful" => WindowBar::Colorful,
        "ColorfulRight" => WindowBar::ColorfulRight,
        "Rings" => WindowBar::Rings,
        "RingsRight" => WindowBar::RingsRight,
        _ => return Err(format!("no such window bar: {value} (Colorful, ColorfulRight, Rings, RingsRight or None)")),
    })
}

fn timing(value: &str) -> Result<Timing, String> {
    match value {
        "tape" => Ok(Timing::Tape),
        "real" => Ok(Timing::Real),
        _ => Err(format!("timing is \"tape\" or \"real\", not {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_name_is_settable() {
        let values = |name: &str| match name {
            "Theme" => "Nord",
            "MarginFill" => "#000",
            "WindowBar" => "Colorful",
            "CursorBlink" => "true",
            "Timing" => "real",
            "TypingSpeed" | "WaitTimeout" | "WaitDuration" => "1s",
            "PaneWidth" | "LoopOffset" => "40%",
            "Columns" | "Rows" => "20",
            "WaitPattern" => "/done/",
            "LineHeight" | "BrowserZoom" | "PlaybackSpeed" => "1.5",
            "Shell" | "Directory" | "FontFamily" | "Prompt" => "x",
            _ => "100",
        };
        let mut settings = Settings::default();
        for name in NAMES {
            assert_eq!(settings.set(name, values(name)), Ok(()), "{name}");
        }
    }

    #[test]
    fn unknown_names_and_bad_values_say_so() {
        let mut settings = Settings::default();
        assert_eq!(settings.set("Color", "red"), Err("no such setting: Color".into()));
        assert!(settings.set("Width", "wide").unwrap_err().contains("not a whole number"));
        assert!(settings.set("Width", "9000").unwrap_err().contains("out of range"));
        assert!(settings.set("Theme", "Neon").unwrap_err().contains("one of: Dracula"));
        assert!(settings.set("WindowBar", "Fancy").unwrap_err().contains("Colorful"));
        assert!(settings.set("CursorBlink", "yes").unwrap_err().contains("true or false"));
        assert!(settings.set("Timing", "fast").unwrap_err().contains("\"tape\" or \"real\""));
        assert!(settings.set("Shell", " ").is_err());
        assert!(settings.set("MarginFill", "blue").is_err());
    }

    #[test]
    fn widths_take_pixels_or_a_percentage() {
        let mut settings = Settings::default();
        settings.set("PaneWidth", "480px").unwrap();
        assert_eq!(settings.pane_width.of(1200), 480);
        settings.set("PaneWidth", "25%").unwrap();
        assert_eq!(settings.pane_width.of(1200), 300);
        assert_eq!(Length::Pixels(5000).of(1200), 1200);
    }

    #[test]
    fn a_loop_offset_is_frames_a_percentage_or_a_time() {
        let length = Duration::from_secs(10);
        let offset = |value: &str| loop_offset(value).unwrap().within(length, 50);

        assert_eq!(offset("25"), Duration::from_millis(500));
        assert_eq!(offset("50%"), Duration::from_secs(5));
        assert_eq!(offset("2.5s"), Duration::from_millis(2500));
        assert_eq!(offset("20s"), length, "never past the end");
        assert!(loop_offset("later").is_err());
    }

    #[test]
    fn a_wait_pattern_is_written_with_or_without_slashes() {
        let mut settings = Settings::default();
        settings.set("WaitPattern", "/ok \\d/").unwrap();
        assert!(settings.wait_pattern.as_ref().unwrap().matches("ok 1"));
        settings.set("WaitPattern", "done$").unwrap();
        assert!(settings.wait_pattern.unwrap().matches("all done"));
        assert!(Settings::default().set("WaitPattern", "/(/").is_err());
        assert!(Settings::default().set("WaitPattern", "/usr/bin").unwrap_err().contains("no such pattern flag: b"));
    }

    #[test]
    fn a_theme_can_be_json() {
        let mut settings = Settings::default();
        settings.set("Theme", r##"{"background": "#101010"}"##).unwrap();
        assert_eq!(settings.theme.background, Rgb(16, 16, 16));
    }

    #[test]
    fn durations_in_every_unit() {
        assert_eq!(duration("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(duration("1.5s"), Ok(Duration::from_millis(1500)));
        assert_eq!(duration("2"), Ok(Duration::from_secs(2)));
        assert_eq!(duration("1m"), Ok(Duration::from_secs(60)));
        assert!(duration("soon").is_err());
    }

    #[test]
    fn durations_reject_negatives_and_unknown_units() {
        assert!(duration("-1s").is_err());
        assert!(duration("5h").is_err());
        assert!(duration("").is_err());
    }
}
