//! An RGB image, and the handful of ways the film is drawn onto one.

use std::path::Path;

use crate::{Error, Result, Rgb};

/// A rectangle of pixels, in whole pixels from the top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    /// The same rectangle, `by` pixels smaller on every side.
    pub fn inset(&self, by: u32) -> Rect {
        Rect::new(
            self.x + by as i32,
            self.y + by as i32,
            self.width.saturating_sub(2 * by),
            self.height.saturating_sub(2 * by),
        )
    }
}

/// An opaque RGB image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Rgb>,
}

impl Image {
    pub fn new(width: u32, height: u32, fill: Rgb) -> Image {
        Image { width, height, pixels: vec![fill; width as usize * height as usize] }
    }

    pub fn get(&self, x: u32, y: u32) -> Rgb {
        self.pixels[(y * self.width + x) as usize]
    }

    /// Blends `color` into one pixel by `alpha` out of 255, if the pixel is on the image.
    pub fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: u8) {
        if alpha == 0 || x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let pixel = &mut self.pixels[(y as u32 * self.width + x as u32) as usize];
        *pixel = if alpha == 255 { color } else { pixel.mix(color, alpha) };
    }

    /// The part of `rect` that is on the image, as ranges of x and y.
    fn clip(&self, rect: Rect) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
        let clamp = |value: i32, max: u32| value.clamp(0, max as i32) as u32;
        (
            clamp(rect.x, self.width)..clamp(rect.right(), self.width),
            clamp(rect.y, self.height)..clamp(rect.bottom(), self.height),
        )
    }

    pub fn fill(&mut self, rect: Rect, color: Rgb) {
        let (columns, rows) = self.clip(rect);
        for y in rows {
            let start = (y * self.width) as usize;
            self.pixels[start + columns.start as usize..start + columns.end as usize].fill(color);
        }
    }

    /// A filled rectangle with its corners rounded to `radius`, anti-aliased.
    pub fn fill_rounded(&mut self, rect: Rect, radius: u32, color: Rgb) {
        let radius = radius.min(rect.width / 2).min(rect.height / 2);
        if radius == 0 {
            return self.fill(rect, color);
        }
        let r = radius as i32;
        self.fill(Rect::new(rect.x, rect.y + r, rect.width, rect.height - 2 * radius), color);
        self.fill(Rect::new(rect.x + r, rect.y, rect.width - 2 * radius, radius), color);
        self.fill(Rect::new(rect.x + r, rect.bottom() - r, rect.width - 2 * radius, radius), color);
        let corners = [
            (rect.x, rect.y, rect.x + r, rect.y + r),
            (rect.right() - r, rect.y, rect.right() - r, rect.y + r),
            (rect.x, rect.bottom() - r, rect.x + r, rect.bottom() - r),
            (rect.right() - r, rect.bottom() - r, rect.right() - r, rect.bottom() - r),
        ];
        for (left, top, centre_x, centre_y) in corners {
            for y in top..top + r {
                for x in left..left + r {
                    let alpha = disc_coverage(x, y, centre_x as f32, centre_y as f32, radius as f32);
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    /// A filled circle, anti-aliased.
    pub fn fill_circle(&mut self, centre_x: f32, centre_y: f32, radius: f32, color: Rgb) {
        let (left, top) = ((centre_x - radius).floor() as i32, (centre_y - radius).floor() as i32);
        let size = (radius * 2.0).ceil() as i32 + 1;
        for y in top..top + size {
            for x in left..left + size {
                self.blend(x, y, color, disc_coverage(x, y, centre_x, centre_y, radius));
            }
        }
    }

    /// A circle's outline, `thickness` wide, anti-aliased.
    pub fn stroke_circle(&mut self, centre_x: f32, centre_y: f32, radius: f32, thickness: f32, color: Rgb) {
        let (left, top) = ((centre_x - radius).floor() as i32, (centre_y - radius).floor() as i32);
        let size = (radius * 2.0).ceil() as i32 + 1;
        for y in top..top + size {
            for x in left..left + size {
                let outer = disc_coverage(x, y, centre_x, centre_y, radius) as i32;
                let inner = disc_coverage(x, y, centre_x, centre_y, radius - thickness) as i32;
                self.blend(x, y, color, (outer - inner).max(0) as u8);
            }
        }
    }

    /// Paints `color` through an 8-bit coverage mask, the way a glyph is drawn.
    pub fn paint_mask(&mut self, x: i32, y: i32, mask: &[u8], mask_width: usize, color: Rgb) {
        for (row, line) in mask.chunks_exact(mask_width.max(1)).enumerate() {
            for (column, alpha) in line.iter().enumerate() {
                self.blend(x + column as i32, y + row as i32, color, *alpha);
            }
        }
    }

    /// Copies `other` onto this image with its top left at `(x, y)`, clipped to the image.
    pub fn draw(&mut self, other: &Image, x: i32, y: i32) {
        let (columns, rows) = self.clip(Rect::new(x, y, other.width, other.height));
        for target_y in rows {
            let source_y = (target_y as i32 - y) as u32;
            let source = (source_y * other.width) as usize + (columns.start as i32 - x) as usize;
            let target = (target_y * self.width) as usize;
            let length = (columns.end - columns.start) as usize;
            self.pixels[target + columns.start as usize..target + columns.end as usize]
                .copy_from_slice(&other.pixels[source..source + length]);
        }
    }

    /// Clears everything outside a rounded rectangle to `outside`, so a window's corners show
    /// what is behind it.
    pub fn round_corners(&mut self, rect: Rect, radius: u32, outside: Rgb) {
        let radius = radius.min(rect.width / 2).min(rect.height / 2);
        let r = radius as i32;
        let corners = [
            (rect.x, rect.y, rect.x + r, rect.y + r),
            (rect.right() - r, rect.y, rect.right() - r, rect.y + r),
            (rect.x, rect.bottom() - r, rect.x + r, rect.bottom() - r),
            (rect.right() - r, rect.bottom() - r, rect.right() - r, rect.bottom() - r),
        ];
        for (left, top, centre_x, centre_y) in corners {
            for y in top..top + r {
                for x in left..left + r {
                    let alpha = 255 - disc_coverage(x, y, centre_x as f32, centre_y as f32, radius as f32);
                    self.blend(x, y, outside, alpha);
                }
            }
        }
    }

    /// The image scaled to exactly `width` × `height`, each pixel the average of what it covers.
    pub fn resize(&self, width: u32, height: u32) -> Image {
        if (width, height) == (self.width, self.height) {
            return self.clone();
        }
        let mut resized = Image::new(width.max(1), height.max(1), Rgb::default());
        let (scale_x, scale_y) = (self.width as f32 / width as f32, self.height as f32 / height as f32);
        for y in 0..resized.height {
            let (top, bottom) = span(y, scale_y, self.height);
            for x in 0..resized.width {
                let (left, right) = span(x, scale_x, self.width);
                let mut total = [0u32; 3];
                for source_y in top..bottom {
                    for source_x in left..right {
                        let Rgb(red, green, blue) = self.get(source_x, source_y);
                        total[0] += red as u32;
                        total[1] += green as u32;
                        total[2] += blue as u32;
                    }
                }
                let count = ((bottom - top) * (right - left)).max(1);
                resized.pixels[(y * width + x) as usize] =
                    Rgb((total[0] / count) as u8, (total[1] / count) as u8, (total[2] / count) as u8);
            }
        }

        resized
    }

    /// Scaled to fit inside `width` × `height` without changing shape, centred on `fill`.
    pub fn fit(&self, width: u32, height: u32, fill: Rgb) -> Image {
        if width == 0 || height == 0 || self.width == 0 || self.height == 0 {
            return Image::new(width.max(1), height.max(1), fill);
        }
        let scale = (width as f32 / self.width as f32).min(height as f32 / self.height as f32);
        let scaled = self.resize(
            ((self.width as f32 * scale).round() as u32).clamp(1, width),
            ((self.height as f32 * scale).round() as u32).clamp(1, height),
        );
        let mut fitted = Image::new(width, height, fill);
        fitted.draw(&scaled, ((width - scaled.width) / 2) as i32, ((height - scaled.height) / 2) as i32);

        fitted
    }

    /// Decodes a PNG of any color type, flattened onto `background` where it is transparent.
    pub fn from_png(bytes: &[u8], background: Rgb) -> Result<Image> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
        let mut reader = decoder.read_info().map_err(|error| Error::new(format!("not a PNG: {error}")))?;
        let mut buffer = vec![0; reader.output_buffer_size().ok_or_else(|| Error::new("PNG too large"))?];
        let info = reader.next_frame(&mut buffer).map_err(|error| Error::new(format!("not a PNG: {error}")))?;
        let channels = info.color_type.samples();
        let pixels = buffer[..info.buffer_size()]
            .chunks_exact(channels)
            .map(|pixel| match *pixel {
                [grey, alpha] => background.mix(Rgb(grey, grey, grey), alpha),
                [red, green, blue, alpha] => background.mix(Rgb(red, green, blue), alpha),
                [red, green, blue] => Rgb(red, green, blue),
                _ => Rgb(pixel[0], pixel[0], pixel[0]),
            })
            .collect();

        Ok(Image { width: info.width, height: info.height, pixels })
    }

    /// Reads a PNG file.
    pub fn open_png(path: &Path, background: Rgb) -> Result<Image> {
        let bytes = std::fs::read(path).map_err(|error| Error::io(path, error))?;
        Image::from_png(&bytes, background)
            .map_err(|error| Error::new(format!("{}: {}", path.display(), error.message)))
    }

    /// Encodes as an RGB PNG.
    pub fn to_png(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().expect("writing to memory cannot fail");
        let raw: Vec<u8> = self.pixels.iter().flat_map(|Rgb(red, green, blue)| [*red, *green, *blue]).collect();
        writer.write_image_data(&raw).expect("writing to memory cannot fail");
        writer.finish().expect("writing to memory cannot fail");

        bytes
    }
}

/// The source pixels one target pixel covers, along one axis.
fn span(target: u32, scale: f32, limit: u32) -> (u32, u32) {
    let start = ((target as f32 * scale) as u32).min(limit - 1);
    let end = (((target + 1) as f32 * scale).ceil() as u32).clamp(start + 1, limit);
    (start, end)
}

/// How much of the pixel at `(x, y)` a disc covers, out of 255, by 4×4 supersampling.
fn disc_coverage(x: i32, y: i32, centre_x: f32, centre_y: f32, radius: f32) -> u8 {
    if radius <= 0.0 {
        return 0;
    }
    let mut inside = 0;
    for sample_y in 0..4 {
        for sample_x in 0..4 {
            let dx = x as f32 + (sample_x as f32 + 0.5) / 4.0 - centre_x;
            let dy = y as f32 + (sample_y as f32 + 0.5) / 4.0 - centre_y;
            inside += (dx * dx + dy * dy <= radius * radius) as u32;
        }
    }

    (inside * 255 / 16) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Rgb = Rgb(0, 0, 0);
    const WHITE: Rgb = Rgb(255, 255, 255);

    #[test]
    fn fill_is_clipped_to_the_image() {
        let mut image = Image::new(4, 4, BLACK);
        image.fill(Rect::new(-2, 2, 4, 10), WHITE);

        assert_eq!(image.get(0, 2), WHITE);
        assert_eq!(image.get(1, 3), WHITE);
        assert_eq!(image.get(2, 2), BLACK);
        assert_eq!(image.get(0, 1), BLACK);
    }

    #[test]
    fn rounded_corners_leave_the_corner_pixel_out() {
        let mut image = Image::new(20, 20, BLACK);
        image.fill_rounded(Rect::new(0, 0, 20, 20), 8, WHITE);

        assert_eq!(image.get(0, 0), BLACK);
        assert_eq!(image.get(10, 10), WHITE);
        assert_eq!(image.get(0, 10), WHITE);
        let edge = image.get(2, 2);
        assert!(edge != BLACK && edge != WHITE, "the curve is anti-aliased: {edge:?}");
    }

    #[test]
    fn round_corners_cuts_what_is_already_drawn() {
        let mut image = Image::new(20, 20, WHITE);
        image.round_corners(Rect::new(0, 0, 20, 20), 6, BLACK);

        assert_eq!(image.get(0, 0), BLACK);
        assert_eq!(image.get(19, 19), BLACK);
        assert_eq!(image.get(10, 0), WHITE);
    }

    #[test]
    fn circles_are_round() {
        let mut image = Image::new(21, 21, BLACK);
        image.fill_circle(10.5, 10.5, 5.0, WHITE);
        assert_eq!(image.get(10, 10), WHITE);
        assert_eq!(image.get(10, 3), BLACK);

        let mut ring = Image::new(21, 21, BLACK);
        ring.stroke_circle(10.5, 10.5, 8.0, 1.5, WHITE);
        assert_eq!(ring.get(10, 10), BLACK, "a ring is empty inside");
        assert_ne!(ring.get(10, 3), BLACK);
    }

    #[test]
    fn draw_copies_with_clipping() {
        let mut image = Image::new(4, 4, BLACK);
        image.draw(&Image::new(3, 3, WHITE), 2, -1);

        assert_eq!(image.get(3, 0), WHITE);
        assert_eq!(image.get(2, 1), WHITE);
        assert_eq!(image.get(2, 2), BLACK);
        assert_eq!(image.get(1, 0), BLACK);
    }

    #[test]
    fn resize_averages() {
        let mut image = Image::new(2, 2, BLACK);
        image.fill(Rect::new(0, 0, 1, 2), WHITE);

        assert_eq!(image.resize(1, 1).get(0, 0), Rgb(127, 127, 127));
        assert_eq!(image.resize(4, 4).get(3, 3), BLACK);
        assert_eq!(image.resize(2, 2), image);
    }

    #[test]
    fn fit_keeps_the_shape_and_centres() {
        let fitted = Image::new(10, 5, WHITE).fit(20, 20, BLACK);

        assert_eq!((fitted.width, fitted.height), (20, 20));
        assert_eq!(fitted.get(10, 10), WHITE);
        assert_eq!(fitted.get(10, 2), BLACK);
    }

    #[test]
    fn png_round_trips() {
        let mut image = Image::new(3, 2, Rgb(1, 2, 3));
        image.fill(Rect::new(1, 1, 1, 1), Rgb(200, 100, 50));

        assert_eq!(Image::from_png(&image.to_png(), BLACK).unwrap(), image);
        assert!(Image::from_png(b"not a png", BLACK).is_err());
    }

    #[test]
    fn transparent_png_pixels_are_flattened_onto_the_background() {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 255, 0, 0, 0]).unwrap();
        writer.finish().unwrap();

        let image = Image::from_png(&bytes, Rgb(0, 0, 255)).unwrap();
        assert_eq!(image.pixels, [Rgb(255, 0, 0), Rgb(0, 0, 255)]);
    }

    #[test]
    fn rect_helpers() {
        let rect = Rect::new(10, 20, 30, 40);
        assert_eq!((rect.right(), rect.bottom()), (40, 60));
        assert_eq!(rect.inset(5), Rect::new(15, 25, 20, 30));
    }
}
