//! A tape, played: every screen, page and caption it showed, on the film's clock.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::image::Image;
use crate::render::{Moment, PaneFrame, Pointer, Renderer};
use crate::terminal::Screen;
use crate::timeline::{Timed, showing};
use crate::{Caption, Error, Result, encode};

/// How long the cursor stays on, and then off, when it blinks.
const BLINK: Duration = Duration::from_millis(530);

/// A recorded demo, ready to be saved as a GIF, a video, a PNG or an asciicast.
///
/// ```no_run
/// let recording = demogod::Demo::from_file("demo.tape")?.record()?;
/// recording.save("demo.gif")?;
/// recording.save("demo.mp4")?;
/// println!("{:.1}s", recording.duration().as_secs_f32());
/// # Ok::<(), demogod::Error>(())
/// ```
pub struct Recording {
    pub(crate) renderer: Renderer,
    pub(crate) screens: Vec<Timed<Screen>>,
    pub(crate) output: Vec<Timed<Vec<u8>>>,
    pub(crate) panes: Vec<Timed<PaneFrame>>,
    /// Which scene is on, by index into `captions`.
    pub(crate) scenes: Vec<Timed<usize>>,
    /// Each scene's caption, numbered among the scenes that have one.
    pub(crate) captions: Vec<Option<(usize, Caption)>>,
    pub(crate) pointer: Vec<Timed<Pointer>>,
    pub(crate) length: Duration,
    pub(crate) framerate: u32,
    pub(crate) playback_speed: f32,
    pub(crate) cursor_blink: bool,
    /// Where a GIF starts, on the film's clock.
    pub(crate) loop_start: Duration,
    pub(crate) shell: String,
}

/// When each track changed, on the recording's real clock: only the times, which is all that is
/// needed to know which of the recording's frames was showing at a real moment.
pub(crate) struct RecordedTracks {
    pub screens: Vec<Timed<()>>,
    pub panes: Vec<Timed<()>>,
    pub scenes: Vec<Timed<usize>>,
    pub pointer: Vec<Timed<Pointer>>,
}

/// What a saved file came to.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Saved {
    /// Where it was written.
    pub path: PathBuf,
    /// How big it is.
    pub bytes: u64,
    /// How many pictures it holds: a GIF's frames, a video's or a `frames/` directory's at the
    /// framerate, the screens in a `.txt`, 1 for a PNG and 0 for an asciicast.
    pub frames: usize,
    /// How long it plays for.
    pub duration: Duration,
}

/// One distinct frame of the film: what it shows, from when, for how long.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shot {
    pub start: Duration,
    pub length: Duration,
    state: State,
}

/// Everything that decides what a frame looks like, as indices into the tracks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct State {
    screen: Option<usize>,
    pane: Option<usize>,
    scene: Option<usize>,
    pointer: Option<(i32, i32, bool)>,
    cursor_on: bool,
}

impl Recording {
    /// How long the film plays for, after `PlaybackSpeed`.
    pub fn duration(&self) -> Duration {
        at_speed(self.length, self.playback_speed)
    }

    /// Writes the film in the format its extension says: `.gif`, `.mp4`, `.webm`, `.png` (the
    /// last frame), `.cast` (an asciinema recording of the terminal) or `.txt` (every screen the
    /// terminal showed, as text, for golden-file tests). A path ending in `/` is a directory to
    /// fill with one PNG per frame, at the framerate.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<Saved> {
        let path = path.as_ref();
        let format = Format::of(path)?;
        let folder = if format == Format::Frames { Some(path) } else { path.parent() };
        if let Some(folder) = folder.filter(|folder| !folder.as_os_str().is_empty()) {
            std::fs::create_dir_all(folder).map_err(|error| Error::io(folder, error))?;
        }
        let frames = match format {
            Format::Gif => encode::gif(self, path)?,
            Format::Video => encode::video(self, path)?,
            Format::Png => {
                std::fs::write(path, self.draw_at(self.length).to_png()).map_err(|error| Error::io(path, error))?;
                1
            }
            Format::Asciicast => {
                encode::asciicast(self, path)?;
                0
            }
            Format::Text => encode::text(self, path)?,
            Format::Frames => encode::frames(self, path)?,
        };

        Ok(Saved { path: path.to_path_buf(), bytes: size_of(path)?, frames, duration: self.duration() })
    }

    /// The film as it is at `at`, drawn.
    pub(crate) fn draw_at(&self, at: Duration) -> Image {
        self.renderer.draw(&self.moment(&self.state_at(at)))
    }

    /// Every distinct frame, in order, each lasting until the next, at the tape's framerate.
    #[cfg(test)]
    pub(crate) fn shots(&self) -> Vec<Shot> {
        self.shots_at(self.framerate)
    }

    /// Every distinct frame, in order, each lasting until the next, on the output's clock (after
    /// `PlaybackSpeed`), and never closer together than `rate` a second — the most a format can
    /// play. Of several changes within one frame's time, the frame shows the last, so frames sit
    /// on the `rate` grid and their lengths add up to the film's exactly.
    pub(crate) fn shots_at(&self, rate: u32) -> Vec<Shot> {
        let rate = rate.clamp(1, self.framerate.max(1)) as f64;
        let speed = self.playback_speed as f64;
        let mut times: Vec<Duration> = [Duration::ZERO]
            .into_iter()
            .chain(self.screens.iter().map(|event| event.at))
            .chain(self.panes.iter().map(|event| event.at))
            .chain(self.scenes.iter().map(|event| event.at))
            .chain(self.pointer.iter().map(|event| event.at))
            .collect();
        // A moving pointer changes every frame.
        let frame_of_film = Duration::from_secs_f64(speed / rate);
        for pair in self.pointer.windows(2) {
            if (pair[0].value.x, pair[0].value.y) != (pair[1].value.x, pair[1].value.y) {
                let mut at = pair[0].at;
                while at < pair[1].at {
                    times.push(at);
                    at += frame_of_film;
                }
            }
        }
        if self.cursor_blink {
            times.extend((0..).map(|index| BLINK * index).take_while(|at| *at < self.length));
        }
        times.retain(|at| *at <= self.length);
        times.sort();

        let slot = |at: Duration| (at_speed(at, self.playback_speed).as_secs_f64() * rate).floor() as u64;
        let mut shots: Vec<Shot> = Vec::new();
        for (index, at) in times.iter().enumerate() {
            if times.get(index + 1).is_some_and(|next| slot(*next) == slot(*at)) {
                continue;
            }
            let start = Duration::from_secs_f64(slot(*at) as f64 / rate);
            let state = self.state_at(*at);
            match shots.last_mut() {
                Some(last) if last.state == state => {}
                _ => shots.push(Shot { start, length: Duration::ZERO, state }),
            }
        }
        let duration = self.duration();
        let ends: Vec<Duration> = shots.iter().skip(1).map(|shot| shot.start).chain([duration]).collect();
        for (shot, end) in shots.iter_mut().zip(ends) {
            shot.length = end.saturating_sub(shot.start);
        }
        shots.retain(|shot| !shot.length.is_zero() || duration.is_zero());

        shots
    }

    /// The shots, starting at the loop offset: what comes after it first, then what came before.
    pub(crate) fn looped_shots(&self, rate: u32) -> Vec<Shot> {
        let offset = at_speed(self.loop_start, self.playback_speed);
        let shots = self.shots_at(rate);
        if offset.is_zero() || offset >= self.duration() {
            return shots;
        }
        let (mut after, mut before) = (Vec::new(), Vec::new());
        for shot in shots {
            let end = shot.start + shot.length;
            if end <= offset {
                before.push(Shot { start: shot.start + (self.duration() - offset), ..shot });
            } else if shot.start >= offset {
                after.push(Shot { start: shot.start - offset, ..shot });
            } else {
                after.push(Shot { start: Duration::ZERO, length: end - offset, ..shot });
                before.push(Shot {
                    start: shot.start + (self.duration() - offset),
                    length: offset - shot.start,
                    ..shot
                });
            }
        }
        after.extend(before);

        after
    }

    /// What each track showed at a real moment, by the tracks as they were recorded — before
    /// the film's clock folds hidden stretches into a single instant.
    pub(crate) fn draw_as_recorded(&self, at: Duration, recorded: &RecordedTracks) -> Image {
        let state = State {
            screen: showing(&recorded.screens, at),
            pane: showing(&recorded.panes, at),
            scene: showing(&recorded.scenes, at).map(|index| recorded.scenes[index].value),
            pointer: showing(&recorded.pointer, at).map(|index| {
                let pointer = recorded.pointer[index].value;
                (pointer.x.round() as i32, pointer.y.round() as i32, pointer.pressed)
            }),
            cursor_on: true,
        };

        self.renderer.draw(&self.moment(&state))
    }

    /// Draws one shot.
    pub(crate) fn draw(&self, shot: &Shot) -> Image {
        self.renderer.draw(&self.moment(&shot.state))
    }

    fn state_at(&self, at: Duration) -> State {
        State {
            screen: showing(&self.screens, at),
            pane: showing(&self.panes, at),
            scene: showing(&self.scenes, at).map(|index| self.scenes[index].value),
            pointer: self
                .pointer_at(at)
                .map(|pointer| (pointer.x.round() as i32, pointer.y.round() as i32, pointer.pressed)),
            cursor_on: !self.cursor_blink || (at.as_millis() / BLINK.as_millis()) % 2 == 0,
        }
    }

    fn moment(&self, state: &State) -> Moment<'_> {
        Moment {
            screen: state.screen.map(|index| &self.screens[index].value),
            pane: state.pane.map(|index| &self.panes[index].value),
            caption: state
                .scene
                .and_then(|scene| self.captions.get(scene)?.as_ref().map(|(number, caption)| (*number, caption))),
            pointer: state.pointer.map(|(x, y, pressed)| Pointer { x: x as f32, y: y as f32, pressed }),
            cursor_on: state.cursor_on,
        }
    }

    /// Where the pointer is at `at`: eased between the two moves around it.
    fn pointer_at(&self, at: Duration) -> Option<Pointer> {
        let index = showing(&self.pointer, at)?;
        let from = &self.pointer[index];
        let Some(to) = self.pointer.get(index + 1) else { return Some(from.value) };
        let span = (to.at - from.at).as_secs_f32();
        if span == 0.0 {
            return Some(from.value);
        }
        let part = ((at - from.at).as_secs_f32() / span).clamp(0.0, 1.0);
        let eased = part * part * (3.0 - 2.0 * part);

        Some(Pointer {
            x: from.value.x + (to.value.x - from.value.x) * eased,
            y: from.value.y + (to.value.y - from.value.y) * eased,
            pressed: from.value.pressed,
        })
    }

    /// The terminal's size in cells, for an asciicast header.
    pub(crate) fn grid(&self) -> (u16, u16) {
        self.screens.first().map_or((24, 80), |screen| (screen.value.rows, screen.value.columns))
    }
}

/// What a film can be saved as, by the path it is saved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Gif,
    /// `.mp4`, `.webm`, `.mov` or `.mkv`, through ffmpeg.
    Video,
    /// The last frame.
    Png,
    Asciicast,
    /// Every screen the terminal showed, as text.
    Text,
    /// A directory of PNGs, one per frame: a path ending in `/`, or a directory already.
    Frames,
}

impl Format {
    pub fn of(path: &Path) -> Result<Format> {
        if path.as_os_str().to_string_lossy().ends_with(['/', '\\']) || path.is_dir() {
            return Ok(Format::Frames);
        }
        let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_ascii_lowercase();
        match extension.as_str() {
            "gif" => Ok(Format::Gif),
            "mp4" | "webm" | "mov" | "mkv" => Ok(Format::Video),
            "png" => Ok(Format::Png),
            "cast" => Ok(Format::Asciicast),
            "txt" | "ascii" => Ok(Format::Text),
            _ => Err(Error::new(format!("cannot save {}: unknown format", path.display()))
                .help("save as .gif, .mp4, .webm, .png, .cast, .txt, or a directory/ of PNG frames")),
        }
    }
}

/// How many bytes a file holds, or all the files in a directory.
fn size_of(path: &Path) -> Result<u64> {
    let metadata = std::fs::metadata(path).map_err(|error| Error::io(path, error))?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let entries = std::fs::read_dir(path).map_err(|error| Error::io(path, error))?;

    Ok(entries.flatten().filter_map(|entry| entry.metadata().ok()).map(|metadata| metadata.len()).sum())
}

/// A span of the film played at `speed`, to the nanosecond, so 1× is exactly what it was.
pub(crate) fn at_speed(span: Duration, speed: f32) -> Duration {
    Duration::from_nanos((span.as_nanos() as f64 / speed as f64).round() as u64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::render::Layout;
    use crate::{Settings, font::Faces};

    pub(crate) fn screen(text: &str) -> Screen {
        let mut parser = vt100::Parser::new(3, 12, 0);
        parser.process(text.as_bytes());
        Screen::capture(parser.screen())
    }

    pub(crate) fn recording(screens: Vec<Timed<Screen>>, length: Duration) -> Recording {
        let settings = Settings { width: 160, height: 80, padding: 4, font_size: 12.0, ..Settings::default() };
        let layout = Layout::new(&settings, false, true, false, false);
        Recording {
            renderer: Renderer::new(&settings, Faces::load("").unwrap(), layout, 1),
            screens,
            output: Vec::new(),
            panes: Vec::new(),
            scenes: Vec::new(),
            captions: Vec::new(),
            pointer: Vec::new(),
            length,
            framerate: 50,
            playback_speed: 1.0,
            cursor_blink: false,
            loop_start: Duration::ZERO,
            shell: "bash".into(),
        }
    }

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn each_change_is_a_shot_lasting_until_the_next() {
        let recording = recording(
            vec![
                Timed { at: ms(0), value: screen("a") },
                Timed { at: ms(500), value: screen("ab") },
                Timed { at: ms(1_000), value: screen("abc") },
            ],
            ms(2_000),
        );
        let shots = recording.shots();

        assert_eq!(shots.iter().map(|shot| shot.start).collect::<Vec<_>>(), [ms(0), ms(500), ms(1_000)]);
        assert_eq!(shots.iter().map(|shot| shot.length).collect::<Vec<_>>(), [ms(500), ms(500), ms(1_000)]);
    }

    #[test]
    fn changes_within_one_frame_show_the_last() {
        let recording = recording(
            vec![
                Timed { at: ms(0), value: screen("a") },
                Timed { at: ms(1_001), value: screen("b") },
                Timed { at: ms(1_005), value: screen("c") },
            ],
            ms(2_000),
        );
        let shots = recording.shots();

        assert_eq!(shots.len(), 2);
        assert_eq!(shots[1].start, ms(1_000));
        assert_eq!(shots[1].state.screen, Some(2));
    }

    #[test]
    fn a_change_back_to_the_same_state_is_not_a_new_shot() {
        let mut recording = recording(vec![Timed { at: ms(0), value: screen("a") }], ms(1_000));
        recording.scenes = vec![Timed { at: ms(0), value: 0 }, Timed { at: ms(400), value: 0 }];

        assert_eq!(recording.shots().len(), 1);
    }

    #[test]
    fn playback_speed_scales_every_shot() {
        let mut recording = recording(
            vec![Timed { at: ms(0), value: screen("a") }, Timed { at: ms(1_000), value: screen("b") }],
            ms(2_000),
        );
        recording.playback_speed = 2.0;

        assert_eq!(recording.duration(), ms(1_000));
        assert_eq!(recording.shots().iter().map(|shot| shot.start).collect::<Vec<_>>(), [ms(0), ms(500)]);
    }

    #[test]
    fn a_blinking_cursor_adds_shots() {
        let mut recording = recording(vec![Timed { at: ms(0), value: screen("a") }], ms(2_000));
        recording.cursor_blink = true;
        let shots = recording.shots();

        assert_eq!(shots.len(), 4);
        assert_eq!(shots.iter().map(|shot| shot.state.cursor_on).collect::<Vec<_>>(), [true, false, true, false]);
    }

    #[test]
    fn the_pointer_eases_between_moves() {
        let mut recording = recording(Vec::new(), ms(1_000));
        recording.pointer = vec![
            Timed { at: ms(0), value: Pointer { x: 0.0, y: 0.0, pressed: false } },
            Timed { at: ms(400), value: Pointer { x: 100.0, y: 50.0, pressed: false } },
        ];

        assert_eq!(recording.pointer_at(ms(200)), Some(Pointer { x: 50.0, y: 25.0, pressed: false }));
        assert!(recording.pointer_at(ms(100)).unwrap().x < 25.0, "slow to start");
        assert_eq!(recording.pointer_at(ms(900)).unwrap().x, 100.0);
        assert_eq!(recording.shots().len(), 21, "one shot per frame while it moves, then still");
    }

    #[test]
    fn a_loop_offset_starts_the_film_part_way_through() {
        let mut recording = recording(
            vec![Timed { at: ms(0), value: screen("a") }, Timed { at: ms(1_000), value: screen("b") }],
            ms(2_000),
        );
        recording.loop_start = ms(500);
        let shots = recording.looped_shots(50);

        let spans: Vec<_> = shots.iter().map(|shot| (shot.start, shot.length, shot.state.screen)).collect();
        assert_eq!(spans, [(ms(0), ms(500), Some(0)), (ms(500), ms(1_000), Some(1)), (ms(1_500), ms(500), Some(0))]);
    }

    #[test]
    fn saving_needs_a_known_format() {
        let recording = recording(vec![Timed { at: ms(0), value: screen("a") }], ms(100));
        let error = recording.save(std::env::temp_dir().join("demogod.avi")).unwrap_err();
        assert!(error.message.contains("unknown format"));
    }

    #[test]
    fn a_png_is_the_last_frame() {
        let recording = recording(vec![Timed { at: ms(0), value: screen("hello") }], ms(100));
        let path = std::env::temp_dir().join(format!("demogod-last-{}.png", std::process::id()));
        let saved = recording.save(&path).unwrap();
        let image = Image::open_png(&path, crate::Rgb(0, 0, 0)).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!((saved.frames, image.width, image.height), (1, 160, 80));
        assert!(saved.bytes > 0);
    }
}
