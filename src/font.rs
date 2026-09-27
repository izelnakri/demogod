//! Fonts: finding them, and turning characters into coverage masks, once each.
//!
//! JetBrains Mono is built in, with DejaVu Sans Mono behind it for symbols, so a tape renders the
//! same everywhere without any font installed. `Set FontFamily` puts others in front of them, and a
//! character any of them lacks falls through to the next. Box-drawing, block and braille characters are drawn rather than looked up, the way
//! terminals draw them, so lines join across rows whatever the line height.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMonoNL-Regular.ttf");
const JETBRAINS_MONO_BOLD: &[u8] = include_bytes!("../assets/fonts/JetBrainsMonoNL-Bold.ttf");
/// For the symbols JetBrains Mono does not have and command lines print anyway: ✔ ✖ ★ ↵ ♥ ☑.
const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

/// Anti-aliasing is kept to this many levels, which is invisible at text sizes and keeps a
/// terminal's colors well inside a GIF's 256.
const COVERAGE_LEVELS: u32 = 8;

/// A typeface, in its regular and (when there is one) bold weights.
struct Face {
    regular: fontdue::Font,
    bold: Option<fontdue::Font>,
}

/// The fonts to draw with, in the order to try them. Parsed once, drawn at any size.
#[derive(Clone)]
pub(crate) struct Faces {
    faces: Arc<Vec<Face>>,
    /// The families asked for that are not installed.
    missing: Vec<String>,
}

impl Faces {
    /// `Set FontFamily`'s value: comma-separated families or font files, then the built-in fonts.
    ///
    /// A family that is not installed is skipped — a tape written on one machine still records
    /// on another — and listed by [`Faces::missing`]. A font file that is not there is an error.
    pub fn load(family: &str) -> Result<Faces> {
        let mut faces = Vec::new();
        let mut missing = Vec::new();
        for name in family.split(',').map(str::trim).filter(|name| !name.is_empty()) {
            if name.eq_ignore_ascii_case("JetBrains Mono") {
                continue;
            }
            let is_file = is_font_file(Path::new(name));
            match find(name) {
                Some((regular, bold)) => {
                    faces.push(Face { regular: parse(&regular)?, bold: bold.as_deref().map(parse).transpose()? })
                }
                None if is_file => return Err(Error::new(format!("no such font file: {name}"))),
                None => missing.push(name.to_string()),
            }
        }
        faces.push(Face {
            regular: fontdue::Font::from_bytes(JETBRAINS_MONO, fontdue::FontSettings::default()).expect("built in"),
            bold: Some(
                fontdue::Font::from_bytes(JETBRAINS_MONO_BOLD, fontdue::FontSettings::default()).expect("built in"),
            ),
        });
        faces.push(Face {
            regular: fontdue::Font::from_bytes(DEJAVU_SANS_MONO, fontdue::FontSettings::default()).expect("built in"),
            bold: None,
        });

        Ok(Faces { faces: Arc::new(faces), missing })
    }

    /// The families asked for that are not installed.
    pub fn missing(&self) -> &[String] {
        &self.missing
    }
}

fn parse(path: &Path) -> Result<fontdue::Font> {
    let bytes = std::fs::read(path).map_err(|error| Error::io(path, error))?;
    fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
        .map_err(|error| Error::new(format!("{}: not a font: {error}", path.display())))
}

/// A character drawn as a coverage mask, placed relative to its cell's top left corner.
pub(crate) struct Glyph {
    pub mask: Vec<u8>,
    pub width: usize,
    pub left: i32,
    pub top: i32,
}

/// The fonts at one size, with the cell every character is drawn in.
pub(crate) struct Font {
    faces: Faces,
    size: f32,
    /// One column, in pixels.
    pub cell_width: u32,
    /// One row, in pixels.
    pub cell_height: u32,
    /// From a cell's top to the baseline its text sits on.
    baseline: i32,
    glyphs: RwLock<HashMap<(char, bool, u8), Arc<Glyph>>>,
}

impl Font {
    pub fn new(faces: Faces, size: f32, line_height: f32, letter_spacing: f32) -> Font {
        let primary = &faces.faces[0].regular;
        let advance = primary.metrics('M', size).advance_width;
        let metrics = primary.horizontal_line_metrics(size).expect("a text font has horizontal metrics");
        let cell_width = (advance + letter_spacing).round().max(1.0) as u32;
        let cell_height = (size * line_height).round().max(1.0) as u32;
        let text_height = metrics.ascent - metrics.descent;
        let baseline = ((cell_height as f32 - text_height) / 2.0 + metrics.ascent).round() as i32;

        Font { faces, size, cell_width, cell_height, baseline, glyphs: RwLock::default() }
    }

    /// A character's glyph, `columns` cells wide, drawn once and kept.
    pub fn glyph(&self, character: char, bold: bool, columns: u8) -> Arc<Glyph> {
        let key = (character, bold, columns);
        if let Some(glyph) = self.glyphs.read().expect("never poisoned").get(&key) {
            return glyph.clone();
        }
        let glyph = Arc::new(
            draw_box(character, self.cell_width as usize * columns as usize, self.cell_height as usize, bold)
                .unwrap_or_else(|| self.rasterize(character, bold, columns)),
        );
        self.glyphs.write().expect("never poisoned").insert(key, glyph.clone());

        glyph
    }

    fn rasterize(&self, character: char, bold: bool, columns: u8) -> Glyph {
        let face = self.faces.faces.iter().find(|face| face.regular.lookup_glyph_index(character) != 0);
        let Some(face) = face else {
            return self.missing(columns);
        };
        let (font, synthetic_bold) = match (&face.bold, bold) {
            (Some(bold_font), true) if bold_font.lookup_glyph_index(character) != 0 => (bold_font, false),
            (_, bold) => (&face.regular, bold),
        };
        let (metrics, mut mask) = font.rasterize(character, self.size);
        let mut width = metrics.width;
        if synthetic_bold && width > 0 {
            // No bold weight: thicken the regular one by a pixel, the way terminals fake it.
            let height = metrics.height;
            let mut thick = vec![0u8; (width + 1) * height];
            for row in 0..height {
                for column in 0..=width {
                    let here = if column < width { mask[row * width + column] } else { 0 };
                    let left = if column > 0 { mask[row * width + column - 1] } else { 0 };
                    thick[row * (width + 1) + column] = here.max(left);
                }
            }
            (mask, width) = (thick, width + 1);
        }
        for alpha in &mut mask {
            *alpha = quantize(*alpha);
        }
        // Centred in its cells, so a glyph from a wider fallback font does not lean right.
        let span = (self.cell_width * columns as u32) as f32;
        let left = ((span - metrics.advance_width) / 2.0).round() as i32 + metrics.xmin;

        Glyph { mask, width, left, top: self.baseline - metrics.height as i32 - metrics.ymin }
    }

    /// The box drawn for a character no font has.
    fn missing(&self, columns: u8) -> Glyph {
        let (width, height) = ((self.cell_width * columns as u32) as usize, self.cell_height as usize);
        let inset = (width.min(height) / 6).max(1);
        let mut mask = vec![0u8; width * height];
        for y in inset..height.saturating_sub(inset) {
            for x in inset..width.saturating_sub(inset) {
                let edge = x == inset || y == inset || x + inset + 1 == width || y + inset + 1 == height;
                mask[y * width + x] = if edge { 160 } else { 0 };
            }
        }

        Glyph { mask, width, left: 0, top: 0 }
    }

    /// Draws a line of text with its top at `y`, one cell per character, and returns where it
    /// ended.
    pub fn draw_text(
        &self,
        image: &mut crate::image::Image,
        x: i32,
        y: i32,
        text: &str,
        color: crate::Rgb,
        bold: bool,
    ) -> i32 {
        let mut x = x;
        for character in text.chars() {
            let columns = crate::render::columns(character);
            let glyph = self.glyph(character, bold, columns);
            image.paint_mask(x + glyph.left, y + glyph.top, &glyph.mask, glyph.width, color);
            x += (self.cell_width * columns as u32) as i32;
        }

        x
    }
}

fn quantize(alpha: u8) -> u8 {
    let level = (alpha as u32 * (COVERAGE_LEVELS - 1) + 127) / 255;
    (level * 255 / (COVERAGE_LEVELS - 1)) as u8
}

// ── box drawing ─────────────────────────────────────────────────────────────────────────────

/// U+2500 to U+257F as the weight of each arm — left, right, up, down — where 1 is light, 2 heavy,
/// 3 double, and `-` marks the diagonals, which are left to the font.
const BOX_ARMS: [&str; 128] = [
    "1100", "2200", "0011", "0022", "1100", "2200", "0011", "0022", "1100", "2200", "0011",
    "0022", // ─━│┃┄┅┆┇┈┉┊┋
    "0101", "0201", "0102", "0202", "1001", "2001", "1002", "2002", // ┌┍┎┏┐┑┒┓
    "0110", "0210", "0120", "0220", "1010", "2010", "1020", "2020", // └┕┖┗┘┙┚┛
    "0111", "0211", "0121", "0112", "0122", "0221", "0212", "0222", // ├┝┞┟┠┡┢┣
    "1011", "2011", "1021", "1012", "1022", "2021", "2012", "2022", // ┤┥┦┧┨┩┪┫
    "1101", "2101", "1201", "2201", "1102", "2102", "1202", "2202", // ┬┭┮┯┰┱┲┳
    "1110", "2110", "1210", "2210", "1120", "2120", "1220", "2220", // ┴┵┶┷┸┹┺┻
    "1111", "2111", "1211", "2211", "1121", "1112", "1122", "2121", // ┼┽┾┿╀╁╂╃
    "1221", "2112", "1212", "2221", "2212", "2122", "1222", "2222", // ╄╅╆╇╈╉╊╋
    "1100", "2200", "0011", "0022", // ╌╍╎╏
    "3300", "0033", "0301", "0103", "0303", "3001", "1003", "3003", // ═║╒╓╔╕╖╗
    "0310", "0130", "0330", "3010", "1030", "3030", "0311", "0133", // ╘╙╚╛╜╝╞╟
    "0333", "3011", "1033", "3033", "3301", "1103", "3303", "3310", // ╠╡╢╣╤╥╦╧
    "1130", "3330", "3311", "1133", "3333", // ╨╩╪╫╬
    "0101", "1001", "1010", "0110", // ╭╮╯╰ (drawn as arcs)
    "----", "----", "----", // ╱╲╳
    "1000", "0010", "0100", "0001", "2000", "0020", "0200", "0002", "1200", "0012", "2100",
    "0021", // ╴╵╶╷╸╹╺╻╼╽╾╿
];

/// A box-drawing, block or braille character, drawn to fill its cell exactly. `None` for anything
/// else, which the font draws.
fn draw_box(character: char, width: usize, height: usize, bold: bool) -> Option<Glyph> {
    let code = character as u32;
    let mut mask = vec![0u8; width * height];
    let mut fill = |left: f32, top: f32, right: f32, bottom: f32, alpha: u8| {
        let (x0, x1) = ((left * width as f32).round() as usize, (right * width as f32).round() as usize);
        let (y0, y1) = ((top * height as f32).round() as usize, (bottom * height as f32).round() as usize);
        for y in y0..y1.min(height) {
            for x in x0..x1.min(width) {
                mask[y * width + x] = mask[y * width + x].max(alpha);
            }
        }
    };

    match code {
        0x2500..=0x257f => {
            let arms = BOX_ARMS[(code - 0x2500) as usize].as_bytes();
            if arms[0] == b'-' {
                return None;
            }
            if (0x256d..=0x2570).contains(&code) {
                return Some(draw_arc(code, width, height, bold));
            }
            draw_lines(&mut mask, width, height, arms, bold);
        }
        0x2580 => fill(0.0, 0.0, 1.0, 0.5, 255),
        0x2581..=0x2588 => fill(0.0, 1.0 - (code - 0x2580) as f32 / 8.0, 1.0, 1.0, 255),
        0x2589..=0x258f => fill(0.0, 0.0, (0x2590 - code) as f32 / 8.0, 1.0, 255),
        0x2590 => fill(0.5, 0.0, 1.0, 1.0, 255),
        0x2591..=0x2593 => fill(0.0, 0.0, 1.0, 1.0, quantize(((code - 0x2590) * 64) as u8)),
        0x2594 => fill(0.0, 0.0, 1.0, 0.125, 255),
        0x2595 => fill(0.875, 0.0, 1.0, 1.0, 255),
        0x2596..=0x259f => {
            // Quadrants as bits: upper left, upper right, lower left, lower right.
            let quadrants = [0b0010, 0b0001, 0b1000, 0b1011, 0b1001, 0b1110, 0b1101, 0b0100, 0b0110, 0b0111];
            let bits = quadrants[(code - 0x2596) as usize];
            for (bit, (left, top)) in [(8, (0.0, 0.0)), (4, (0.5, 0.0)), (2, (0.0, 0.5)), (1, (0.5, 0.5))] {
                if bits & bit != 0 {
                    fill(left, top, left + 0.5, top + 0.5, 255);
                }
            }
        }
        0x2800..=0x28ff => {
            // Braille: dots 1-3 and 7 down the left, 4-6 and 8 down the right.
            let dots = code - 0x2800;
            let positions = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
            let radius = (width.min(height / 2) as f32 / 5.0).max(1.0);
            for (bit, (column, row)) in positions.iter().enumerate() {
                if dots & (1 << bit) == 0 {
                    continue;
                }
                let centre_x = width as f32 * (0.3 + 0.4 * *column as f32);
                let centre_y = height as f32 * (0.15 + 0.233 * *row as f32);
                for y in 0..height {
                    for x in 0..width {
                        let (dx, dy) = (x as f32 + 0.5 - centre_x, y as f32 + 0.5 - centre_y);
                        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                        mask[y * width + x] = mask[y * width + x].max(quantize((coverage * 255.0) as u8));
                    }
                }
            }
        }
        _ => return None,
    }

    Some(Glyph { mask, width, left: 0, top: 0 })
}

/// How thick a light line is in a cell this wide; a heavy one is twice that.
fn line_thickness(width: usize, bold: bool) -> usize {
    (width / 8).max(1) + bold as usize
}

fn draw_lines(mask: &mut [u8], width: usize, height: usize, arms: &[u8], bold: bool) {
    let light = line_thickness(width, bold);
    let (centre_x, centre_y) = (width / 2, height / 2);
    let mut paint = |x0: usize, x1: usize, y0: usize, y1: usize| {
        for y in y0..y1.min(height) {
            for x in x0..x1.min(width) {
                mask[y * width + x] = 255;
            }
        }
    };
    // Each arm runs from the far side of the centre line to the edge, so arms of different
    // weights still meet without a notch.
    let across =
        |centre: usize, thickness: usize| (centre.saturating_sub(thickness / 2), centre + thickness.div_ceil(2));
    let heaviest_vertical = light * arms[2].max(arms[3]).saturating_sub(b'0').min(2) as usize;
    let heaviest_horizontal = light * arms[0].max(arms[1]).saturating_sub(b'0').min(2) as usize;

    for (index, weight) in arms.iter().enumerate() {
        let weight = (weight - b'0') as usize;
        if weight == 0 {
            continue;
        }
        let horizontal = index < 2;
        let (centre, other_centre) = if horizontal { (centre_y, centre_x) } else { (centre_x, centre_y) };
        let reach = if horizontal { heaviest_vertical } else { heaviest_horizontal }.div_ceil(2);
        let lines: Vec<(usize, usize)> = if weight == 3 {
            let gap = light.max(2);
            vec![across(centre - gap, light), across(centre + gap, light)]
        } else {
            vec![across(centre, light * weight)]
        };
        let extent = if horizontal { width } else { height };
        let (start, end) = match index % 2 {
            0 => (0, other_centre + reach.max(light.div_ceil(2))),
            _ => (other_centre.saturating_sub(reach.max(light / 2)), extent),
        };
        for (near, far) in lines {
            if horizontal { paint(start, end, near, far) } else { paint(near, far, start, end) }
        }
    }
}

/// `╭╮╯╰`: a quarter circle between two edge midpoints, with straight lines to the edges.
fn draw_arc(code: u32, width: usize, height: usize, bold: bool) -> Glyph {
    let thickness = line_thickness(width, bold) as f32;
    let (centre_x, centre_y) = (width as f32 / 2.0, height as f32 / 2.0);
    let radius = centre_x.min(centre_y);
    // Which way the arc opens: towards the right (╭╰) or left, and down (╭╮) or up.
    let (to_right, to_down) = match code {
        0x256d => (true, true),
        0x256e => (false, true),
        0x256f => (false, false),
        _ => (true, false),
    };
    let arc_x = if to_right { centre_x + radius } else { centre_x - radius };
    let arc_y = if to_down { centre_y + radius } else { centre_y - radius };
    let mut mask = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut inside = 0;
            for sample in 0..16 {
                let px = x as f32 + (sample % 4) as f32 / 4.0 + 0.125;
                let py = y as f32 + (sample / 4) as f32 / 4.0 + 0.125;
                let in_quadrant = (px <= arc_x) == to_right && (py <= arc_y) == to_down;
                let on = if in_quadrant {
                    let distance = ((px - arc_x).powi(2) + (py - arc_y).powi(2)).sqrt();
                    (distance - radius).abs() <= thickness / 2.0
                } else {
                    // Past the arc: the straight part, along the centre line to the edge.
                    let vertical = (px - centre_x).abs() <= thickness / 2.0 && (py > arc_y) == to_down;
                    let horizontal = (py - centre_y).abs() <= thickness / 2.0 && (px > arc_x) == to_right;
                    vertical || horizontal
                };
                inside += on as u32;
            }
            mask[y * width + x] = quantize((inside * 255 / 16) as u8);
        }
    }

    Glyph { mask, width, left: 0, top: 0 }
}

// ── finding installed fonts ─────────────────────────────────────────────────────────────────

/// A family's regular and bold files, or a single file given by path.
fn find(name: &str) -> Option<(PathBuf, Option<PathBuf>)> {
    let path = Path::new(name);
    if is_font_file(path) {
        return path.is_file().then(|| (path.to_path_buf(), None));
    }

    let family = normalize(name);
    let files = font_files();
    let with_style = |styles: &[&str]| {
        styles.iter().find_map(|style| {
            files.iter().find(|file| {
                file.file_stem().and_then(|stem| stem.to_str()).map(normalize) == Some(format!("{family}{style}"))
            })
        })
    };
    let regular = with_style(&["regular", "", "book", "normal", "medium"])?;

    Some((regular.clone(), with_style(&["bold"]).cloned()))
}

fn normalize(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|character| character.to_ascii_lowercase()).collect()
}

/// Whether a path names a font file: `.ttf`, `.otf` or `.ttc`, in any case. Of a collection, the
/// first font is used.
fn is_font_file(path: &Path) -> bool {
    let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(extension.as_str(), "ttf" | "otf" | "ttc")
}

/// Every font file in the places fonts are installed, on any of the three systems.
fn font_files() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut roots = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        home.join(".local/share/fonts"),
        home.join(".fonts"),
        home.join(".nix-profile/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/X11/fonts"),
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        home.join("Library/Fonts"),
        PathBuf::from(r"C:\Windows\Fonts"),
    ];
    if let Some(user) = std::env::var_os("USER") {
        roots.push(Path::new("/etc/profiles/per-user").join(user).join("share/fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(Path::new(&local).join(r"Microsoft\Windows\Fonts"));
    }

    let mut files = Vec::new();
    let mut pending: Vec<(PathBuf, u8)> = roots.into_iter().map(|root| (root, 0)).collect();
    while let Some((directory, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 6 {
                pending.push((path, depth + 1));
            } else if is_font_file(&path) {
                files.push(path);
            }
        }
    }
    files.sort();

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Font {
        Font::new(Faces::load("").unwrap(), 16.0, 1.25, 0.0)
    }

    fn column(glyph: &Glyph, x: usize) -> Vec<u8> {
        glyph.mask.chunks(glyph.width).map(|row| row[x]).collect()
    }

    #[test]
    fn cells_are_whole_pixels_from_the_built_in_font() {
        let font = font();
        assert_eq!((font.cell_width, font.cell_height), (10, 20));
    }

    #[test]
    fn the_built_in_fonts_have_what_command_lines_print() {
        let font = font();
        for character in "❯✓✗✔✖✘●→←↑↓…•›»λ★☆♥↵⬆⬇☐☑↻".chars() {
            let found = font.faces.faces.iter().any(|face| face.regular.lookup_glyph_index(character) != 0);
            assert!(found, "{character} is missing");
        }
    }

    #[test]
    fn glyphs_are_cached() {
        let font = font();
        assert!(Arc::ptr_eq(&font.glyph('a', false, 1), &font.glyph('a', false, 1)));
        assert!(!Arc::ptr_eq(&font.glyph('a', false, 1), &font.glyph('a', true, 1)));
    }

    #[test]
    fn coverage_has_few_levels() {
        let font = font();
        let glyph = font.glyph('@', false, 1);
        let levels: std::collections::BTreeSet<_> = glyph.mask.iter().collect();
        assert!(levels.len() <= COVERAGE_LEVELS as usize, "{levels:?}");
    }

    #[test]
    fn a_vertical_line_spans_the_whole_cell() {
        let font = font();
        let line = font.glyph('│', false, 1);
        let middle = column(&line, line.width / 2);
        assert!(middle.iter().all(|alpha| *alpha == 255), "{middle:?}");
        assert!(column(&line, 0).iter().all(|alpha| *alpha == 0));
    }

    #[test]
    fn corners_and_tees_reach_the_edges_they_should() {
        let font = font();
        let corner = font.glyph('┌', false, 1);
        let (width, height) = (corner.width, corner.mask.len() / corner.width);
        assert_eq!(corner.mask[(height / 2) * width + width - 1], 255, "right arm reaches the right edge");
        assert_eq!(corner.mask[(height - 1) * width + width / 2], 255, "down arm reaches the bottom");
        assert_eq!(corner.mask[(height / 2) * width], 0, "no left arm");
        assert_eq!(corner.mask[width / 2], 0, "no up arm");
    }

    #[test]
    fn an_arc_reaches_both_edges_and_skips_the_corner() {
        let font = font();
        let arc = font.glyph('╭', false, 1);
        let (width, height) = (arc.width, arc.mask.len() / arc.width);
        assert!(arc.mask[(height - 1) * width + width / 2] > 0);
        assert!(arc.mask[(height / 2) * width + width - 1] > 0);
        assert_eq!(arc.mask[0], 0);
    }

    #[test]
    fn blocks_fill_their_share() {
        let font = font();
        let full = font.glyph('█', false, 1);
        assert!(full.mask.iter().all(|alpha| *alpha == 255));
        let lower = font.glyph('▄', false, 1);
        assert_eq!(lower.mask[0], 0);
        assert_eq!(*lower.mask.last().unwrap(), 255);
        let shade = font.glyph('▒', false, 1);
        assert!(shade.mask.iter().all(|alpha| (100..160).contains(alpha)));
    }

    #[test]
    fn braille_dots_are_where_their_bits_say() {
        let font = font();
        let one = font.glyph('⠁', false, 1);
        let (width, height) = (one.width, one.mask.len() / one.width);
        let top_left = &one.mask[..(height / 4) * width];
        assert!(top_left.iter().any(|alpha| *alpha > 0));
        assert!(one.mask[(height * 3 / 4) * width..].iter().all(|alpha| *alpha == 0));
        assert!(font.glyph('⠀', false, 1).mask.iter().all(|alpha| *alpha == 0));
    }

    #[test]
    fn a_missing_character_is_a_box_not_nothing() {
        let font = font();
        let missing = font.glyph('\u{10ffff}', false, 1);
        assert!(missing.mask.iter().any(|alpha| *alpha > 0));
    }

    #[test]
    fn bold_is_bolder() {
        let font = font();
        let ink = |bold| font.glyph('H', bold, 1).mask.iter().map(|alpha| *alpha as u32).sum::<u32>();
        assert!(ink(true) > ink(false));
    }

    #[test]
    fn a_family_not_installed_is_skipped_and_a_missing_file_is_an_error() {
        let faces = Faces::load("Definitely Not A Font 9000, JetBrains Mono").unwrap();
        assert_eq!(faces.missing(), ["Definitely Not A Font 9000"]);
        assert_eq!(faces.faces.len(), 2, "the built-in fonts");
        assert!(Faces::load("/nope/font.ttf").err().unwrap().message.contains("no such font file"));
    }

    #[test]
    fn a_font_file_can_be_given_by_path() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/JetBrainsMonoNL-Bold.ttf");
        let faces = Faces::load(path.to_str().unwrap()).unwrap();
        assert_eq!(faces.faces.len(), 3);
    }

    #[test]
    fn text_is_drawn_one_cell_per_character() {
        let font = font();
        let mut image = crate::image::Image::new(100, 30, crate::Rgb(0, 0, 0));
        let end = font.draw_text(&mut image, 5, 5, "ab界", crate::Rgb(255, 255, 255), false);
        assert_eq!(end, 5 + 40);
        assert!(image.pixels.iter().any(|pixel| *pixel != crate::Rgb(0, 0, 0)));
    }
}
