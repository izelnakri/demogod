//! The film, written out: GIF, video and asciicast.
//!
//! Frames are drawn and compressed on every core, a batch at a time, and written in order — so a
//! long film never holds more than a batch of frames in memory.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::image::{Image, Rect};
use crate::recording::{Recording, Shot};
use crate::{Error, Result, Rgb};

/// How many frames are drawn at once: enough to keep every core busy, few enough to stay small.
fn batch_size() -> usize {
    std::thread::available_parallelism().map_or(4, |cores| cores.get()) * 4
}

/// `f` over every item, on every core, results in order.
pub(crate) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(usize, &T) -> R + Sync) -> Vec<R> {
    let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get()).min(items.len().max(1));
    let chunk = items.len().div_ceil(cores).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(chunk)
            .enumerate()
            .map(|(index, part)| {
                let f = &f;
                scope.spawn(move || {
                    part.iter().enumerate().map(|(offset, item)| f(index * chunk + offset, item)).collect::<Vec<_>>()
                })
            })
            .collect();
        workers.into_iter().flat_map(|worker| worker.join().expect("a frame never panics")).collect()
    })
}

// ── GIF ─────────────────────────────────────────────────────────────────────────────────────

/// Writes an animated GIF, and returns how many frames it has.
///
/// Every frame after the first holds only the rectangle that changed, with what did not change
/// inside it left transparent — a keystroke is a few hundred bytes. A frame with 256 colors or
/// fewer (every terminal frame) keeps its exact colors; one with more (a web page) is quantized.
pub(crate) fn gif(recording: &Recording, path: &Path) -> Result<usize> {
    // Browsers play GIF frames no faster than 50 a second.
    let shots = recording.looped_shots(50);
    let layout = &recording.renderer.layout;
    let file = File::create(path).map_err(|error| Error::io(path, error))?;
    let mut encoder = gif::Encoder::new(BufWriter::new(file), layout.width as u16, layout.height as u16, &[])
        .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;
    encoder.set_repeat(gif::Repeat::Infinite).map_err(|error| Error::new(error.to_string()))?;

    let delays = centiseconds(&shots);
    let mut previous: Option<Image> = None;
    for (batch_index, batch) in shots.chunks(batch_size()).enumerate() {
        let images = parallel_map(batch, |_, shot| recording.draw(shot));
        let first = batch_index * batch_size();
        let frames = parallel_map(&images, |index, image| {
            let before = if index == 0 { previous.as_ref() } else { Some(&images[index - 1]) };
            let mut frame = gif_frame(before, image);
            frame.delay = delays[first + index];
            frame.make_lzw_pre_encoded();
            frame
        });
        for frame in &frames {
            encoder
                .write_lzw_pre_encoded_frame(frame)
                .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;
        }
        previous = images.into_iter().last();
    }
    encoder.into_inner().and_then(|mut writer| Ok(writer.flush()?)).map_err(|error| Error::new(error.to_string()))?;

    Ok(shots.len())
}

/// Each shot's delay in hundredths of a second, rounded against the running total so the sum is
/// the film's length. Shots are at most 50 a second, so none is under the 2 browsers need.
fn centiseconds(shots: &[Shot]) -> Vec<u16> {
    let mut shown = 0u64;
    shots
        .iter()
        .map(|shot| {
            let end = ((shot.start + shot.length).as_secs_f64() * 100.0).round() as u64;
            let delay = end.saturating_sub(shown).clamp(2, u16::MAX as u64);
            shown += delay;
            delay as u16
        })
        .collect()
}

/// One GIF frame: the changed rectangle of `image`, or all of it when there is nothing before.
fn gif_frame(before: Option<&Image>, image: &Image) -> gif::Frame<'static> {
    let area = match before {
        Some(before) => changed(before, image).unwrap_or(Rect::new(0, 0, 1, 1)),
        None => Rect::new(0, 0, image.width, image.height),
    };
    let pixels: Vec<Option<Rgb>> = (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x as u32, y as u32)))
        .map(|(x, y)| {
            let now = image.get(x, y);
            match before {
                Some(before) if before.get(x, y) == now => None,
                _ => Some(now),
            }
        })
        .collect();

    let (palette, indices) = index(&pixels);
    let transparent = pixels.iter().any(Option::is_none).then_some((palette.len() / 3) as u8);
    let mut palette = palette;
    if transparent.is_some() {
        palette.extend([0, 0, 0]);
    }

    gif::Frame {
        left: area.x as u16,
        top: area.y as u16,
        width: area.width as u16,
        height: area.height as u16,
        buffer: indices.into(),
        palette: Some(palette),
        transparent,
        dispose: gif::DisposalMethod::Keep,
        ..gif::Frame::default()
    }
}

/// The smallest rectangle holding every pixel that differs, or `None` if none does.
fn changed(before: &Image, after: &Image) -> Option<Rect> {
    let differs = |y: u32| {
        let row = (y * after.width) as usize..((y + 1) * after.width) as usize;
        before.pixels[row.clone()] != after.pixels[row]
    };
    let top = (0..after.height).find(|y| differs(*y))?;
    let bottom = (top..after.height).rev().find(|y| differs(*y))?;
    let column_differs = |x: u32| (top..=bottom).any(|y| before.get(x, y) != after.get(x, y));
    let left = (0..after.width).find(|x| column_differs(*x))?;
    let right = (left..after.width).rev().find(|x| column_differs(*x))?;

    Some(Rect::new(left as i32, top as i32, right - left + 1, bottom - top + 1))
}

/// A palette and each pixel's index into it, with `None` pixels at the index after the palette.
/// Exact when there are 255 colors or fewer, NeuQuant's best 255 otherwise.
fn index(pixels: &[Option<Rgb>]) -> (Vec<u8>, Vec<u8>) {
    let mut colors: HashMap<Rgb, u8> = HashMap::new();
    let mut exact = true;
    for pixel in pixels.iter().flatten() {
        if !colors.contains_key(pixel) {
            if colors.len() == 255 {
                exact = false;
                break;
            }
            colors.insert(*pixel, colors.len() as u8);
        }
    }

    if exact {
        let mut palette = vec![0u8; colors.len() * 3];
        for (Rgb(red, green, blue), index) in &colors {
            palette[*index as usize * 3..*index as usize * 3 + 3].copy_from_slice(&[*red, *green, *blue]);
        }
        let hole = colors.len() as u8;
        let indices = pixels.iter().map(|pixel| pixel.map_or(hole, |pixel| colors[&pixel])).collect();
        return (palette, indices);
    }

    let rgba: Vec<u8> = pixels.iter().flatten().flat_map(|Rgb(red, green, blue)| [*red, *green, *blue, 255]).collect();
    let quantizer = color_quant::NeuQuant::new(10, 255, &rgba);
    let mut cache: HashMap<Rgb, u8> = HashMap::new();
    let indices = pixels
        .iter()
        .map(|pixel| match pixel {
            None => 255,
            Some(pixel) => {
                *cache.entry(*pixel).or_insert_with(|| quantizer.index_of(&[pixel.0, pixel.1, pixel.2, 255]) as u8)
            }
        })
        .collect();

    (quantizer.color_map_rgb(), indices)
}

// ── video ───────────────────────────────────────────────────────────────────────────────────

/// Writes an MP4, WebM, MOV or MKV through ffmpeg, which is the one outside program any output
/// needs. Returns how many frames it holds.
pub(crate) fn video(recording: &Recording, path: &Path) -> Result<usize> {
    let ffmpeg = crate::which("ffmpeg").ok_or_else(|| {
        Error::new(format!("saving {} needs ffmpeg, which is not on PATH", path.display()))
            .help("install ffmpeg, or save as .gif instead")
    })?;
    let layout = &recording.renderer.layout;
    let framerate = recording.framerate.min(60);
    let webm = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("webm"));
    let codec: &[&str] = if webm {
        &["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-row-mt", "1", "-deadline", "good", "-cpu-used", "5"]
    } else {
        &["-c:v", "libx264", "-preset", "medium", "-crf", "18", "-tune", "animation", "-movflags", "+faststart"]
    };
    let mut child = Command::new(ffmpeg)
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", &format!("{}x{}", layout.width, layout.height), "-r", &framerate.to_string(), "-i", "-"])
        .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-pix_fmt", "yuv420p"])
        .args(codec)
        .arg(path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::new(format!("could not start ffmpeg: {error}")))?;

    let shots = recording.shots_at(framerate);
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut written = 0u64;
    let mut failed = None;
    'batches: for batch in shots.chunks(batch_size()) {
        let frames = parallel_map(batch, |_, shot| {
            let raw: Vec<u8> =
                recording.draw(shot).pixels.iter().flat_map(|Rgb(red, green, blue)| [*red, *green, *blue]).collect();
            (raw, shot.start + shot.length)
        });
        for (raw, end) in frames {
            // A constant framerate: each frame repeated until the film reaches its end.
            let until = ((end.as_secs_f64() * framerate as f64).round() as u64).max(written + 1);
            while written < until {
                if let Err(error) = stdin.write_all(&raw) {
                    failed = Some(error);
                    break 'batches;
                }
                written += 1;
            }
        }
    }
    drop(stdin);
    let output = child.wait_with_output().map_err(|error| Error::new(format!("ffmpeg: {error}")))?;
    if !output.status.success() || failed.is_some() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(Error::new(format!("ffmpeg could not write {}: {}", path.display(), message.trim())));
    }

    Ok(written as usize)
}

// ── frames ──────────────────────────────────────────────────────────────────────────────────

/// Fills a directory with the film as PNGs, `frame-00001.png` onwards, one per frame at the
/// framerate the way a video has them. Returns how many were written.
pub(crate) fn frames(recording: &Recording, directory: &Path) -> Result<usize> {
    let shots = recording.shots_at(recording.framerate.min(60));
    let framerate = recording.framerate.min(60) as f64;
    let mut written = 0usize;
    for batch in shots.chunks(batch_size()) {
        let pictures = parallel_map(batch, |_, shot| (recording.draw(shot).to_png(), shot.start + shot.length));
        for (png, end) in pictures {
            let until = ((end.as_secs_f64() * framerate).round() as usize).max(written + 1);
            while written < until {
                written += 1;
                let path = directory.join(format!("frame-{written:05}.png"));
                std::fs::write(&path, &png).map_err(|error| Error::io(&path, error))?;
            }
        }
    }

    Ok(written)
}

// ── asciicast ───────────────────────────────────────────────────────────────────────────────

/// Writes an asciicast v2 file: the terminal's own output, on the film's clock, for asciinema.
pub(crate) fn asciicast(recording: &Recording, path: &Path) -> Result<()> {
    let (rows, columns) = recording.grid();
    let header = serde_json::json!({
        "version": 2,
        "width": columns,
        "height": rows,
        "duration": recording.duration().as_secs_f64(),
        "env": { "TERM": "xterm-256color", "SHELL": recording.shell },
    });
    let mut lines = vec![header.to_string()];
    let mut pending = Vec::new();
    for event in &recording.output {
        pending.extend_from_slice(&event.value);
        // Hold back a character split across two reads until the rest of it arrives.
        let complete = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => pending.len(),
        };
        if complete == 0 {
            continue;
        }
        let text = String::from_utf8_lossy(&pending[..complete]).into_owned();
        pending.drain(..complete);
        let at = crate::recording::at_speed(event.at.min(recording.length), recording.playback_speed);
        lines.push(serde_json::json!([round_to_ms(at), "o", text]).to_string());
    }
    lines.push(String::new());

    std::fs::write(path, lines.join("\n")).map_err(|error| Error::io(path, error))
}

// ── text ────────────────────────────────────────────────────────────────────────────────────

/// Writes every distinct screen the terminal showed, as text, between rules — what VHS writes for
/// `.txt`, and what a golden-file test compares. Returns how many screens there were.
pub(crate) fn text(recording: &Recording, path: &Path) -> Result<usize> {
    let mut screens: Vec<String> = Vec::new();
    for screen in &recording.screens {
        let text = screen.value.text();
        if screens.last() != Some(&text) {
            screens.push(text);
        }
    }
    let rule = "─".repeat(80);
    let contents: String = screens.iter().map(|screen| format!("{screen}\n{rule}\n")).collect();
    std::fs::write(path, contents).map_err(|error| Error::io(path, error))?;

    Ok(screens.len())
}

fn round_to_ms(at: Duration) -> f64 {
    (at.as_secs_f64() * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::tests::{recording, screen};
    use crate::timeline::Timed;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn temporary(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("demogod-{}-{name}", std::process::id()))
    }

    #[test]
    fn parallel_map_keeps_the_order() {
        let items: Vec<u32> = (0..1000).collect();
        assert_eq!(parallel_map(&items, |index, item| (index as u32, item * 2))[999], (999, 1998));
        assert!(parallel_map(&Vec::<u32>::new(), |_, item| *item).is_empty());
    }

    #[test]
    fn changed_finds_the_smallest_rectangle() {
        let before = Image::new(10, 10, Rgb(0, 0, 0));
        let mut after = before.clone();
        after.fill(Rect::new(2, 3, 4, 2), Rgb(1, 1, 1));

        assert_eq!(changed(&before, &after), Some(Rect::new(2, 3, 4, 2)));
        assert_eq!(changed(&before, &before), None);
    }

    #[test]
    fn indexing_is_exact_under_256_colors_and_marks_the_unchanged() {
        let pixels = [Some(Rgb(1, 2, 3)), None, Some(Rgb(4, 5, 6)), Some(Rgb(1, 2, 3))];
        let (palette, indices) = index(&pixels);

        assert_eq!(palette.len(), 6);
        assert_eq!(indices, [0, 2, 1, 0]);
        assert_eq!(&palette[3..6], &[4, 5, 6]);
    }

    #[test]
    fn many_colors_are_quantized_to_255() {
        let pixels: Vec<_> =
            (0..4096u32).map(|index| Some(Rgb(index as u8, (index >> 4) as u8, (index >> 8) as u8))).collect();
        let (palette, indices) = index(&pixels);

        assert_eq!(palette.len(), 255 * 3);
        assert!(indices.iter().all(|index| *index < 255));
    }

    #[test]
    fn delays_add_up_to_the_film_and_are_never_too_short() {
        let shots = recording(
            vec![
                Timed { at: ms(0), value: screen("a") },
                Timed { at: ms(333), value: screen("b") },
                Timed { at: ms(666), value: screen("c") },
            ],
            ms(1_000),
        )
        .shots();
        let delays = centiseconds(&shots);

        assert_eq!(delays.iter().map(|delay| *delay as u32).sum::<u32>(), 100);
        assert!(delays.iter().all(|delay| *delay >= 2));
    }

    #[test]
    fn a_gif_has_a_frame_per_change_and_decodes() {
        let recording = recording(
            vec![Timed { at: ms(0), value: screen("$ ") }, Timed { at: ms(200), value: screen("$ ls") }],
            ms(1_000),
        );
        let path = temporary("two.gif");
        assert_eq!(recording.save(&path).unwrap().frames, 2);

        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(File::open(&path).unwrap()).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            frames.push((frame.delay, frame.width, frame.height));
        }
        std::fs::remove_file(&path).unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].1, frames[0].2), (160, 80), "the first frame is whole");
        assert!(frames[1].1 < 160 && frames[1].2 < 80, "the second is only what changed: {:?}", frames[1]);
        assert_eq!(frames[0].0 + frames[1].0, 100);
    }

    #[test]
    fn an_asciicast_has_a_header_and_the_output() {
        let mut recording = recording(vec![Timed { at: ms(0), value: screen("") }], ms(1_000));
        recording.output = vec![
            Timed { at: ms(0), value: b"$ ".to_vec() },
            Timed { at: ms(500), value: "caf\u{e9}".as_bytes()[..4].to_vec() },
            Timed { at: ms(600), value: "caf\u{e9}".as_bytes()[4..].to_vec() },
        ];
        let path = temporary("out.cast");
        recording.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();

        assert_eq!(lines[0]["version"], 2);
        assert_eq!(lines[0]["width"], 12);
        assert_eq!(lines[1], serde_json::json!([0.0, "o", "$ "]));
        assert_eq!(lines[2], serde_json::json!([0.5, "o", "caf"]));
        assert_eq!(lines[3], serde_json::json!([0.6, "o", "\u{e9}"]), "a character split across reads is kept whole");
    }

    #[test]
    fn text_is_every_distinct_screen_between_rules() {
        let recording = recording(
            vec![
                Timed { at: ms(0), value: screen("$ ") },
                Timed { at: ms(100), value: screen("$ ") },
                Timed { at: ms(200), value: screen("$ ls") },
            ],
            ms(1_000),
        );
        let path = temporary("screens.txt");
        let saved = recording.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(saved.frames, 2);
        assert!(text.starts_with("$\n\n\n────"), "{text:?}");
        assert!(text.contains("$ ls"));
    }

    #[test]
    fn a_directory_gets_a_png_per_frame_at_the_framerate() {
        let mut recording = recording(
            vec![Timed { at: ms(0), value: screen("a") }, Timed { at: ms(100), value: screen("b") }],
            ms(200),
        );
        recording.framerate = 20;
        let directory = temporary("frames/");
        let saved = recording.save(&directory).unwrap();
        let mut names: Vec<_> =
            std::fs::read_dir(&directory).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        names.sort();
        std::fs::remove_dir_all(&directory).unwrap();

        assert_eq!(saved.frames, 4);
        assert_eq!(names.first().unwrap(), "frame-00001.png");
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn video_needs_ffmpeg_and_says_so() {
        if crate::which("ffmpeg").is_none() {
            let recording = recording(vec![Timed { at: ms(0), value: screen("a") }], ms(100));
            let error = recording.save(temporary("x.mp4")).unwrap_err();
            assert!(error.message.contains("needs ffmpeg"));
        }
    }

    #[test]
    fn video_is_written_when_ffmpeg_is_there() {
        if crate::which("ffmpeg").is_none() {
            return;
        }
        let recording = recording(
            vec![Timed { at: ms(0), value: screen("a") }, Timed { at: ms(500), value: screen("ab") }],
            ms(1_000),
        );
        let path = temporary("film.mp4");
        let saved = recording.save(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(saved.bytes > 0);
        assert_eq!(saved.frames, 50, "a second at 50 frames a second");
    }
}
