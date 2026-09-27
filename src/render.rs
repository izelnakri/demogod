//! One frame of the film, drawn: the caption strip, the terminal, the pane, and the pointer.
//!
//! Everything here is a function of what is on screen at one moment — a [`Moment`] — so frames
//! can be drawn in any order, on as many threads as there are.

use std::sync::Arc;

use crate::font::{Faces, Font};
use crate::image::{Image, Rect};
use crate::terminal::{Cell, Color, Screen};
use crate::{Caption, Rgb, Settings, Theme, WindowBar};

/// Where everything goes in the film.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub width: u32,
    pub height: u32,
    pub caption: Option<Rect>,
    pub terminal: Option<Window>,
    pub pane: Option<Window>,
    /// Whether the pane shows web pages, and so has an address bar.
    pub pane_is_browser: bool,
}

/// A window: its outline, the bar across its top if it has one, and what is under the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Window {
    pub frame: Rect,
    pub bar: Option<Rect>,
    pub body: Rect,
}

impl Window {
    fn new(frame: Rect, bar_height: u32) -> Window {
        let bar = (bar_height > 0).then(|| Rect::new(frame.x, frame.y, frame.width, bar_height));
        let body =
            Rect::new(frame.x, frame.y + bar_height as i32, frame.width, frame.height.saturating_sub(bar_height));
        Window { frame, bar, body }
    }
}

impl Layout {
    /// Splits the film between what the tape uses: a caption strip when there are captions, a
    /// terminal when anything is typed, a pane when anything is opened or shown.
    pub fn new(
        settings: &Settings,
        has_caption: bool,
        has_terminal: bool,
        has_pane: bool,
        pane_is_browser: bool,
    ) -> Layout {
        let margin = settings.margin;
        let inner = Rect::new(0, 0, settings.width, settings.height).inset(margin);
        let gap = margin / 2;
        let caption_height = if has_caption { (settings.font_size * 2.75).round() as u32 } else { 0 };
        let caption = has_caption.then(|| Rect::new(inner.x, inner.y, inner.width, caption_height));
        let below = if has_caption { caption_height + gap } else { 0 };
        let area = Rect::new(inner.x, inner.y + below as i32, inner.width, inner.height.saturating_sub(below));
        let bar_height = settings.window_bar_size.unwrap_or((settings.font_size * 2.0).round() as u32);
        let terminal_bar = if settings.window_bar == WindowBar::None { 0 } else { bar_height };
        let pane_bar = if pane_is_browser { bar_height } else { 0 };

        let (terminal, pane) = match (has_terminal || !has_pane, has_pane) {
            (true, true) => {
                let pane_width = settings.pane_width.of(area.width).min(area.width.saturating_sub(gap + 64));
                let terminal_width = area.width.saturating_sub(pane_width + gap);
                let terminal = Rect::new(area.x, area.y, terminal_width, area.height);
                let pane = Rect::new(area.right() - pane_width as i32, area.y, pane_width, area.height);
                (Some(Window::new(terminal, terminal_bar)), Some(Window::new(pane, pane_bar)))
            }
            (_, true) => (None, Some(Window::new(area, pane_bar))),
            _ => (Some(Window::new(area, terminal_bar)), None),
        };

        Layout { width: settings.width, height: settings.height, caption, terminal, pane, pane_is_browser }
    }

    /// Why nothing could be drawn in this layout, if that is so: a window without room inside it.
    pub fn problem(&self, font: &Font, padding: u32) -> Option<String> {
        let windows = [("terminal", self.terminal, padding), ("pane", self.pane, 0)];
        windows.into_iter().find_map(|(name, window, inset)| {
            let body = window?.body.inset(inset);
            (body.width < font.cell_width * 10 || body.height < font.cell_height * 2).then(|| {
                format!(
                    "the {name} has no room: {}×{} pixels inside, after the margin, padding and bars",
                    body.width, body.height
                )
            })
        })
    }

    /// How many rows and columns of text fit in the terminal.
    pub fn grid(&self, font: &Font, padding: u32) -> (u16, u16) {
        let Some(terminal) = self.terminal else { return (24, 80) };
        let body = terminal.body.inset(padding);
        let rows = (body.height / font.cell_height).clamp(2, 500) as u16;
        let columns = (body.width / font.cell_width).clamp(10, 1000) as u16;

        (rows, columns)
    }

    /// The pane's page area in pixels, which is the size the browser is asked for.
    pub fn page_size(&self) -> (u32, u32) {
        self.pane.map_or((800, 600), |pane| (pane.body.width, pane.body.height))
    }
}

/// Where the pointer is over the page, in page pixels, and whether it is pressing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Pointer {
    pub x: f32,
    pub y: f32,
    pub pressed: bool,
}

/// A picture in the pane, and the address the toolbar shows above it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaneFrame {
    pub image: Arc<Image>,
    pub url: Option<String>,
}

/// What the film shows at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Moment<'a> {
    pub screen: Option<&'a Screen>,
    pub pane: Option<&'a PaneFrame>,
    pub caption: Option<(usize, &'a Caption)>,
    pub pointer: Option<Pointer>,
    pub cursor_on: bool,
}

/// The colors of everything around the terminal, shaded from its theme.
struct Palette {
    background: Rgb,
    foreground: Rgb,
    strip: Rgb,
    bar: Rgb,
    muted: Rgb,
    faint: Rgb,
    accent: Rgb,
    accent_done: Rgb,
    track: Rgb,
}

impl Palette {
    fn of(theme: &Theme) -> Palette {
        let (background, foreground) = (theme.background, theme.foreground);
        let accent = theme.ansi[4];
        Palette {
            background,
            foreground,
            strip: background.mix(Rgb(0, 0, 0), if theme.is_light() { 14 } else { 56 }),
            bar: background.mix(foreground, 16),
            muted: foreground.mix(background, 110),
            faint: foreground.mix(background, 170),
            accent,
            accent_done: accent.mix(background, 120),
            track: background.mix(foreground, 30),
        }
    }
}

/// Draws moments.
pub(crate) struct Renderer {
    pub layout: Layout,
    pub font: Font,
    caption_font: Font,
    theme: Theme,
    palette: Palette,
    padding: u32,
    margin: u32,
    radius: u32,
    margin_fill: Rgb,
    window_bar: WindowBar,
    scenes: usize,
    /// The page's CSS pixels to the pane's, for placing the pointer.
    pub zoom: f32,
}

impl Renderer {
    pub fn new(settings: &Settings, faces: Faces, layout: Layout, scenes: usize) -> Renderer {
        let font = Font::new(faces.clone(), settings.font_size, settings.line_height, settings.letter_spacing);
        let caption_font = Font::new(faces, (settings.font_size * 0.95).round(), 1.0, 0.0);
        let radius = if settings.margin > 0 { settings.border_radius } else { 0 };

        Renderer {
            layout,
            font,
            caption_font,
            palette: Palette::of(&settings.theme),
            theme: settings.theme.clone(),
            padding: settings.padding,
            margin: settings.margin,
            radius,
            margin_fill: settings.margin_fill,
            window_bar: settings.window_bar,
            scenes,
            zoom: settings.browser_zoom,
        }
    }

    /// One frame.
    pub fn draw(&self, moment: &Moment) -> Image {
        let behind = if self.margin > 0 { self.margin_fill } else { self.palette.background };
        let mut image = Image::new(self.layout.width, self.layout.height, behind);

        if let Some(strip) = self.layout.caption {
            self.draw_caption(&mut image, strip, moment.caption);
        }
        if let Some(window) = self.layout.terminal {
            self.draw_window(&mut image, window);
            if let Some(bar) = window.bar {
                self.draw_dots(&mut image, bar);
            }
            if let Some(screen) = moment.screen {
                self.draw_screen(&mut image, window.body.inset(self.padding), screen, moment.cursor_on);
            }
        }
        if let Some(window) = self.layout.pane {
            self.draw_window(&mut image, window);
            let frame = moment.pane;
            if let Some(bar) = window.bar {
                self.draw_toolbar(&mut image, bar, frame.and_then(|frame| frame.url.as_deref()));
            }
            if let Some(frame) = frame {
                let picture = if (frame.image.width, frame.image.height) == (window.body.width, window.body.height) {
                    frame.image.clone()
                } else {
                    Arc::new(frame.image.fit(window.body.width, window.body.height, self.palette.background))
                };
                image.draw(&picture, window.body.x, window.body.y);
            }
            if let Some(pointer) = moment.pointer {
                let x = window.body.x as f32 + pointer.x * self.zoom;
                let y = window.body.y as f32 + pointer.y * self.zoom;
                draw_pointer(
                    &mut image,
                    x,
                    y,
                    pointer.pressed,
                    self.font.cell_height as f32 / 20.0,
                    self.palette.accent,
                );
            }
            if self.radius > 0 {
                image.round_corners(window.frame, self.radius, self.margin_fill);
            }
        }
        if let (Some(window), true) = (self.layout.terminal, self.radius > 0) {
            image.round_corners(window.frame, self.radius, self.margin_fill);
        }
        if let (Some(terminal), Some(pane), 0) = (self.layout.terminal, self.layout.pane, self.margin) {
            image.fill(Rect::new(pane.frame.x, terminal.frame.y, 1, terminal.frame.height), self.palette.track);
        }

        image
    }

    fn draw_window(&self, image: &mut Image, window: Window) {
        image.fill(window.frame, self.palette.background);
        if let Some(bar) = window.bar {
            image.fill(bar, self.palette.bar);
        }
    }

    /// The three dots of a window bar, on the side and in the style `Set WindowBar` says.
    fn draw_dots(&self, image: &mut Image, bar: Rect) -> i32 {
        let radius = bar.height as f32 * 0.19;
        let spacing = radius * 3.3;
        let right = matches!(self.window_bar, WindowBar::ColorfulRight | WindowBar::RingsRight);
        let first = if right { bar.right() as f32 - radius * 2.6 - spacing * 2.0 } else { bar.x as f32 + radius * 2.6 };
        let centre_y = bar.y as f32 + bar.height as f32 / 2.0;
        let colors = [Rgb(0xff, 0x5f, 0x57), Rgb(0xfe, 0xbc, 0x2e), Rgb(0x28, 0xc8, 0x40)];
        for (index, color) in colors.iter().enumerate() {
            let centre_x = first + spacing * index as f32;
            match self.window_bar {
                WindowBar::Rings | WindowBar::RingsRight => {
                    image.stroke_circle(centre_x, centre_y, radius, 1.5, self.palette.faint)
                }
                _ => image.fill_circle(centre_x, centre_y, radius, *color),
            }
        }

        if right { bar.x } else { (first + spacing * 2.0 + radius * 2.6) as i32 }
    }

    /// A browser's toolbar: the window's dots, and the address in a rounded field.
    fn draw_toolbar(&self, image: &mut Image, bar: Rect, url: Option<&str>) {
        let start =
            if self.window_bar == WindowBar::None { bar.x + bar.height as i32 / 3 } else { self.draw_dots(image, bar) };
        let font = &self.caption_font;
        let height = (bar.height as f32 * 0.62) as u32;
        let field = Rect::new(
            start,
            bar.y + (bar.height - height) as i32 / 2,
            (bar.right() - start - bar.height as i32 / 3).max(0) as u32,
            height,
        );
        image.fill_rounded(field, height / 2, self.palette.background);
        let Some(url) = url else { return };
        // A file is shown by its name: its full path says nothing about the demo.
        let shown = match url.strip_prefix("file://") {
            Some(path) => percent_decoded(path.rsplit('/').next().unwrap_or(path)),
            None => url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string(),
        };
        let room = (field.width.saturating_sub(height) / font.cell_width) as usize;
        let text: String = if shown.chars().count() > room {
            shown.chars().take(room.saturating_sub(1)).chain(['…']).collect()
        } else {
            shown
        };
        let y = field.y + (field.height as i32 - font.cell_height as i32) / 2;
        font.draw_text(image, field.x + height as i32 / 2, y, &text, self.palette.muted, false);
    }

    /// The strip along the top: which scene of how many, its title and detail, and a bar of
    /// segments filling in as the scenes go by.
    fn draw_caption(&self, image: &mut Image, strip: Rect, caption: Option<(usize, &Caption)>) {
        image.fill(strip, self.palette.strip);
        let font = &self.caption_font;
        let track_height = (strip.height / 14).max(3);
        let text_y = strip.y + (strip.height as i32 - track_height as i32 - font.cell_height as i32) / 2;
        let mut x = strip.x + (font.cell_width * 2) as i32;
        let current = caption.map(|(index, _)| index);

        if let Some((index, caption)) = caption {
            let counter = format!("{}/{}", index + 1, self.scenes);
            x = font.draw_text(image, x, text_y, &counter, self.palette.faint, false) + font.cell_width as i32 * 2;
            x = font.draw_text(image, x, text_y, &caption.title, self.palette.foreground, true)
                + font.cell_width as i32 * 2;
            let room = ((strip.right() - x).max(0) as u32 / font.cell_width) as usize;
            let detail: String = if caption.detail.chars().count() > room {
                caption.detail.chars().take(room.saturating_sub(2)).chain(['…']).collect()
            } else {
                caption.detail.clone()
            };
            font.draw_text(image, x, text_y, &detail, self.palette.muted, false);
        }

        let segments = self.scenes.max(1) as u32;
        let gap = 3;
        let track_y = strip.bottom() - track_height as i32;
        let segment_width = (strip.width.saturating_sub(gap * (segments - 1))) as f32 / segments as f32;
        for segment in 0..segments {
            let left = strip.x + (segment as f32 * (segment_width + gap as f32)).round() as i32;
            let right = strip.x + ((segment + 1) as f32 * (segment_width + gap as f32) - gap as f32).round() as i32;
            let color = match current {
                Some(now) if (segment as usize) < now => self.palette.accent_done,
                Some(now) if segment as usize == now => self.palette.accent,
                _ => self.palette.track,
            };
            image.fill(Rect::new(left, track_y, (right - left).max(1) as u32, track_height), color);
        }
    }

    fn draw_screen(&self, image: &mut Image, area: Rect, screen: &Screen, cursor_on: bool) {
        let (cell_width, cell_height) = (self.font.cell_width as i32, self.font.cell_height as i32);
        let cursor = screen.cursor.filter(|_| cursor_on);
        let rows = screen.rows.min((area.height / self.font.cell_height) as u16);
        let columns = screen.columns.min((area.width / self.font.cell_width) as u16);

        // Backgrounds first, then text, so a glyph that reaches into the next row is not painted over.
        for row in 0..rows {
            for column in 0..columns {
                let cell = screen.cell(row, column);
                let (_, background) = self.colors(cell, cursor == Some((row, column)));
                if background != self.palette.background {
                    let span = if cell.has(Cell::WIDE) { 2 } else { 1 };
                    let (x, y) = (area.x + column as i32 * cell_width, area.y + row as i32 * cell_height);
                    image.fill(Rect::new(x, y, (cell_width * span) as u32, cell_height as u32), background);
                }
            }
        }
        for row in 0..rows {
            for column in 0..columns {
                let cell = screen.cell(row, column);
                if cell.has(Cell::WIDE_TAIL) {
                    continue;
                }
                let (foreground, _) = self.colors(cell, cursor == Some((row, column)));
                let (x, y) = (area.x + column as i32 * cell_width, area.y + row as i32 * cell_height);
                if cell.character != ' ' {
                    let span = if cell.has(Cell::WIDE) { 2 } else { 1 };
                    let glyph = self.font.glyph(cell.character, cell.has(Cell::BOLD), span);
                    image.paint_mask(x + glyph.left, y + glyph.top, &glyph.mask, glyph.width, foreground);
                }
                if cell.has(Cell::UNDERLINE) {
                    let thickness = (cell_height / 16).max(1);
                    image.fill(
                        Rect::new(x, y + cell_height - thickness * 2, cell_width as u32, thickness as u32),
                        foreground,
                    );
                }
            }
        }
    }

    /// A cell's text and background colors, after bold-is-bright, dim, inverse and the cursor.
    fn colors(&self, cell: &Cell, under_cursor: bool) -> (Rgb, Rgb) {
        let resolve = |color: Color, default: Rgb| match color {
            Color::Default => default,
            Color::Indexed(index) if index < 8 && cell.has(Cell::BOLD) && default == self.theme.foreground => {
                self.theme.indexed(index + 8)
            }
            Color::Indexed(index) => self.theme.indexed(index),
            Color::Rgb(red, green, blue) => Rgb(red, green, blue),
        };
        let mut foreground = resolve(cell.foreground, self.theme.foreground);
        let mut background = resolve(cell.background, self.theme.background);
        if cell.has(Cell::INVERSE) {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.has(Cell::DIM) {
            foreground = foreground.mix(background, 110);
        }
        if under_cursor { (self.theme.background, self.theme.cursor) } else { (foreground, background) }
    }
}

/// The arrow pointer, as rows of pixels: `X` outline, `.` fill. Scaled to the text size.
const ARROW: [&str; 17] = [
    "X",
    "XX",
    "X.X",
    "X..X",
    "X...X",
    "X....X",
    "X.....X",
    "X......X",
    "X.......X",
    "X........X",
    "X.....XXXXX",
    "X..X..X",
    "X.X X..X",
    "XX  X..X",
    "X    X..X",
    "     X..X",
    "      XX",
];

fn draw_pointer(image: &mut Image, x: f32, y: f32, pressed: bool, scale: f32, accent: Rgb) {
    let scale = scale.max(1.0);
    if pressed {
        image.fill_circle(x, y, 14.0 * scale, Rgb(0, 0, 0).mix(accent, 200));
        image.stroke_circle(x, y, 14.0 * scale, 2.0 * scale, accent);
    }
    let size = scale.round().max(1.0) as i32;
    for (row, line) in ARROW.iter().enumerate() {
        for (column, pixel) in line.chars().enumerate() {
            let color = match pixel {
                'X' => Rgb(0, 0, 0),
                '.' => Rgb(255, 255, 255),
                _ => continue,
            };
            let (left, top) = (x as i32 + column as i32 * size, y as i32 + row as i32 * size);
            image.fill(Rect::new(left, top, size as u32, size as u32), color);
        }
    }
}

/// `my%20page.html` as `my page.html`.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// How many columns a character takes: 2 for wide ones, 1 for the rest.
pub(crate) fn columns(character: char) -> u8 {
    unicode_width::UnicodeWidthChar::width(character).unwrap_or(1).clamp(1, 2) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Length, Theme};

    fn settings() -> Settings {
        Settings { width: 800, height: 400, font_size: 16.0, ..Settings::default() }
    }

    fn renderer(settings: &Settings, caption: bool, terminal: bool, pane: bool) -> Renderer {
        let layout = Layout::new(settings, caption, terminal, pane, pane);
        Renderer::new(settings, Faces::load("").unwrap(), layout, 3)
    }

    fn screen(text: &str) -> Screen {
        let mut parser = vt100::Parser::new(4, 20, 0);
        parser.process(text.as_bytes());
        Screen::capture(parser.screen())
    }

    #[test]
    fn a_terminal_alone_fills_the_film() {
        let layout = Layout::new(&settings(), false, true, false, false);
        assert_eq!(layout.terminal.unwrap().frame, Rect::new(0, 0, 800, 400));
        assert_eq!(layout.pane, None);
        assert_eq!(layout.caption, None);
    }

    #[test]
    fn the_pane_takes_its_share_on_the_right() {
        let settings = Settings { pane_width: Length::Percent(25.0), ..settings() };
        let layout = Layout::new(&settings, true, true, true, true);

        let pane = layout.pane.unwrap();
        assert_eq!(pane.frame.width, 200);
        assert_eq!(pane.frame.right(), 800);
        assert_eq!(layout.terminal.unwrap().frame.width, 600);
        assert_eq!(layout.caption.unwrap().height, 44);
        assert_eq!(pane.frame.y, 44);
        assert_eq!(pane.body.y, 44 + 32, "a browser pane has an address bar");
    }

    #[test]
    fn a_margin_surrounds_everything() {
        let settings = Settings { margin: 20, window_bar: WindowBar::Colorful, ..settings() };
        let layout = Layout::new(&settings, false, true, false, false);
        let terminal = layout.terminal.unwrap();

        assert_eq!(terminal.frame, Rect::new(20, 20, 760, 360));
        assert_eq!(terminal.bar.unwrap().height, 32);
        assert_eq!(terminal.body.y, 52);
    }

    #[test]
    fn a_browser_alone_fills_the_film() {
        let layout = Layout::new(&settings(), false, false, true, true);
        assert!(layout.terminal.is_none());
        assert_eq!(layout.pane.unwrap().frame.width, 800);
    }

    #[test]
    fn the_grid_is_what_fits_inside_the_padding() {
        let settings = settings();
        let renderer = renderer(&settings, false, true, false);
        assert_eq!(renderer.layout.grid(&renderer.font, 20), ((400 - 40) / 20, (800 - 40) / 10));
    }

    #[test]
    fn text_is_drawn_in_its_colors() {
        let settings = settings();
        let renderer = renderer(&settings, false, true, false);
        let image = renderer.draw(&Moment { screen: Some(&screen("\x1b[41m  \x1b[0m")), ..Moment::default() });

        assert_eq!(image.get(25, 25), settings.theme.ansi[1], "a red background");
        assert_eq!(image.get(5, 5), settings.theme.background);
    }

    #[test]
    fn the_cursor_is_a_block_that_can_blink_off() {
        let settings = settings();
        let renderer = renderer(&settings, false, true, false);
        let screen = screen("");
        let on = renderer.draw(&Moment { screen: Some(&screen), cursor_on: true, ..Moment::default() });
        let off = renderer.draw(&Moment { screen: Some(&screen), cursor_on: false, ..Moment::default() });

        assert_eq!(on.get(25, 30), settings.theme.cursor);
        assert_eq!(off.get(25, 30), settings.theme.background);
    }

    #[test]
    fn inverse_swaps_and_bold_brightens() {
        let settings = settings();
        let renderer = renderer(&settings, false, true, false);
        let theme = &settings.theme;
        let cell = |foreground, style| Cell { character: 'x', foreground, background: Color::Default, style };

        assert_eq!(renderer.colors(&cell(Color::Indexed(1), Cell::BOLD), false).0, theme.ansi[9]);
        assert_eq!(renderer.colors(&cell(Color::Indexed(1), Cell::INVERSE), false), (theme.background, theme.ansi[1]));
        assert_eq!(renderer.colors(&cell(Color::Rgb(1, 2, 3), 0), false).0, Rgb(1, 2, 3));
        assert_eq!(renderer.colors(&cell(Color::Default, 0), true), (theme.background, theme.cursor));
    }

    #[test]
    fn the_caption_strip_shows_progress() {
        let settings = settings();
        let renderer = renderer(&settings, true, true, false);
        let caption = Caption { title: "Run it".into(), detail: "fast".into() };
        let image = renderer.draw(&Moment { caption: Some((1, &caption)), ..Moment::default() });
        let strip = renderer.layout.caption.unwrap();
        let track_y = strip.bottom() as u32 - 1;
        let palette = Palette::of(&settings.theme);

        assert_eq!(image.get(10, track_y), palette.accent_done, "the first scene is done");
        assert_eq!(image.get(400, track_y), palette.accent, "the second is now");
        assert_eq!(image.get(790, track_y), palette.track, "the third is to come");
    }

    #[test]
    fn the_pane_shows_its_picture_and_the_pointer() {
        let settings = settings();
        let renderer = renderer(&settings, false, true, true);
        let pane = renderer.layout.pane.unwrap();
        let picture = Arc::new(Image::new(pane.body.width, pane.body.height, Rgb(1, 2, 3)));
        let frame = PaneFrame { image: picture, url: Some("https://example.com/".into()) };
        let pointer = Pointer { x: 10.0, y: 10.0, pressed: false };
        let image = renderer.draw(&Moment { pane: Some(&frame), pointer: Some(pointer), ..Moment::default() });

        assert_eq!(image.get(pane.body.right() as u32 - 5, pane.body.bottom() as u32 - 5), Rgb(1, 2, 3));
        assert_eq!(image.get((pane.body.x + 10) as u32, (pane.body.y + 10) as u32), Rgb(0, 0, 0), "the pointer's tip");
    }

    #[test]
    fn rounded_windows_show_the_margin_in_their_corners() {
        let settings = Settings { margin: 20, border_radius: 10, margin_fill: Rgb(9, 9, 9), ..settings() };
        let renderer = renderer(&settings, false, true, false);
        let image = renderer.draw(&Moment::default());

        assert_eq!(image.get(20, 20), Rgb(9, 9, 9));
        assert_eq!(image.get(40, 40), settings.theme.background);
    }

    #[test]
    fn a_light_theme_still_has_readable_chrome() {
        let settings = Settings { theme: Theme::named("GitHub Light").unwrap(), ..settings() };
        let palette = Palette::of(&settings.theme);
        assert!(palette.muted.luma() < palette.background.luma());
    }

    #[test]
    fn no_setting_in_range_makes_drawing_panic() {
        let faces = Faces::load("").unwrap();
        let screen = screen("\u{1f680} wide \u{10ffff}");
        for (width, height, margin, padding, font_size, letter_spacing, bar) in [
            (16, 16, 0, 0, 4.0, 0.0, None),
            (100, 40, 45, 20, 200.0, 100.0, Some(100)),
            (8192, 16, 8192, 0, 16.0, -10.0, Some(0)),
            (300, 60, 10, 50, 16.0, 100.0, Some(60)),
        ] {
            let settings = Settings {
                width,
                height,
                margin,
                padding,
                font_size,
                letter_spacing,
                window_bar: WindowBar::Colorful,
                window_bar_size: bar,
                border_radius: 20,
                ..Settings::default()
            };
            for (caption, terminal, pane) in [(true, true, true), (false, false, true), (true, true, false)] {
                let layout = Layout::new(&settings, caption, terminal, pane, pane);
                let renderer = Renderer::new(&settings, faces.clone(), layout, 2);
                let _ = renderer.layout.problem(&renderer.font, padding);
                let picture =
                    PaneFrame { image: Arc::new(Image::new(7, 3, Rgb(1, 2, 3))), url: Some("file:///a/b.html".into()) };
                let caption = Caption { title: "A title".into(), detail: "x".repeat(500) };
                renderer.draw(&Moment {
                    screen: Some(&screen),
                    pane: Some(&picture),
                    caption: Some((1, &caption)),
                    pointer: Some(Pointer { x: -50.0, y: 1e6, pressed: true }),
                    cursor_on: true,
                });
            }
        }
    }

    #[test]
    fn percent_encoding_is_undone_for_the_address_bar() {
        assert_eq!(percent_decoded("my%20page%23.html"), "my page#.html");
        assert_eq!(percent_decoded("100%"), "100%");
    }

    #[test]
    fn wide_characters_take_two_columns() {
        assert_eq!(columns('界'), 2);
        assert_eq!(columns('a'), 1);
        assert_eq!(columns('\u{0301}'), 1);
    }
}
