//! A tape, played for real: a shell, a browser, the clock, and whatever the host adds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::browser::Browser;
use crate::font::Faces;
use crate::image::Image;
use crate::recording::{Format, RecordedTracks, Recording};
use crate::render::{Layout, PaneFrame, Pointer, Renderer};
use crate::terminal::{Launch, Terminal};
use crate::timeline::{Clock, Timed, moments};
use crate::{Caption, Command, Error, Focus, Length, Result, Saved, ScrollTarget, Settings, Step, Tape, Timing, which};

/// How long the pointer takes to reach what it clicks, in the film.
const POINTER_TRAVEL: Duration = Duration::from_millis(600);
/// How long a click holds the button down, in the film.
const CLICK_HOLD: Duration = Duration::from_millis(250);
/// How long a `Scroll` takes, in the film.
const SCROLL_TIME: Duration = Duration::from_millis(700);
/// The real time after a keystroke into the browser: long enough for the page to handle it.
const BROWSER_KEY_DELAY: Duration = Duration::from_millis(8);
/// How long a terminal keystroke waits for its echo. The echo usually takes well under a
/// millisecond; this is for a loaded machine, and for a program that echoes nothing.
const ECHO_PATIENCE: Duration = Duration::from_millis(250);

type Action = Box<dyn FnMut() -> std::result::Result<(), String> + Send>;
type Listener = Box<dyn FnMut(&Event) + Send>;

/// Something worth telling whoever is watching a recording.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A scene began.
    Scene {
        /// Its number among the scenes with a caption, from 1 — the `2` of the strip's `2/5` —
        /// or nothing for steps before the first `Caption`.
        number: Option<usize>,
        /// How many scenes have a caption: the strip's `5`.
        total: usize,
        /// Its caption, if it has one.
        caption: Option<Caption>,
    },
    /// `Do <name>` ran.
    Action {
        /// The action's name.
        name: String,
    },
    /// Something is not as the tape asked, and the recording carries on without it.
    Warning {
        /// What, and what is used instead.
        message: String,
    },
    /// The tape has been played; what is left is encoding.
    Recorded {
        /// How long the film is.
        duration: Duration,
    },
    /// A file was written.
    Saved(Saved),
}

/// A tape, the actions its `Do`s may call, and someone to tell how it is going.
///
/// ```no_run
/// use demogod::Demo;
///
/// let saved = Demo::from_file("demo.tape")?
///     .action("break", || std::fs::write("src/cart.ts", "/* broken */"))
///     .on_event(|event| eprintln!("{event:?}"))
///     .run()?;
/// # Ok::<(), demogod::Error>(())
/// ```
pub struct Demo {
    tape: Tape,
    actions: HashMap<String, Action>,
    listener: Option<Listener>,
}

impl Demo {
    /// A demo of a tape already parsed.
    pub fn new(tape: Tape) -> Demo {
        Demo { tape, actions: HashMap::new(), listener: None }
    }

    /// Reads a tape file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Demo> {
        Tape::from_file(path).map(Demo::new)
    }

    /// Parses a tape from a string, as if it were a file in the current directory.
    pub fn parse(source: &str) -> Result<Demo> {
        Tape::parse(source).map(Demo::new)
    }

    /// The tape being played.
    pub fn tape(&self) -> &Tape {
        &self.tape
    }

    /// Where the film is saved: the tape's `Output`s, unless these replace them.
    pub fn outputs(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Demo {
        let paths: Vec<PathBuf> = paths.into_iter().map(Into::into).collect();
        if !paths.is_empty() {
            self.tape.outputs = paths;
        }
        self
    }

    /// Names something `Do <name>` runs, off camera. It takes precedence over an `Action` of the
    /// same name in the tape.
    pub fn action<E: std::fmt::Display>(
        mut self,
        name: impl Into<String>,
        mut run: impl FnMut() -> std::result::Result<(), E> + Send + 'static,
    ) -> Demo {
        self.actions.insert(name.into(), Box::new(move || run().map_err(|error| error.to_string())));
        self
    }

    /// Calls `listener` as the recording goes: each scene, each action, each file saved.
    pub fn on_event(mut self, listener: impl FnMut(&Event) + Send + 'static) -> Demo {
        self.listener = Some(Box::new(listener));
        self
    }

    /// Everything that can fail before recording starts, so it fails before, not a minute in: a
    /// `Do` nothing declares, a `Require`d program missing, an output format that needs ffmpeg
    /// without it, a browser the tape opens and the machine does not have.
    pub fn check(&self) -> Result<()> {
        let tape = &self.tape;
        for (name, location) in tape.used_actions() {
            if !tape.actions.contains_key(name) && !self.actions.contains_key(name) {
                return Err(Error::new(format!("no action is named {name}"))
                    .at(location.clone())
                    .help(format!("declare it in the tape: Action {name} \"<shell command>\"")));
            }
        }
        for program in &tape.requires {
            if which(program).is_none() {
                return Err(Error::new(format!("{program} is required, and not on PATH")));
            }
        }
        for output in &tape.outputs {
            if Format::of(output)? == Format::Video && which("ffmpeg").is_none() {
                return Err(Error::new(format!("saving {} needs ffmpeg, which is not on PATH", output.display()))
                    .help("install ffmpeg, or save as .gif"));
            }
        }
        if tape.uses_terminal() {
            let (shell, _) = crate::terminal::split_shell(&tape.settings.shell);
            if which(shell).is_none() {
                return Err(Error::new(format!("the shell {shell} is not on PATH")).help("Set Shell \"bash\""));
            }
            if !crate::terminal::marks_prompt(&tape.settings.shell) {
                let bare_wait = tape.steps().find(|step| {
                    step.focus == Focus::Terminal && matches!(step.command, Command::Wait { pattern: None, .. })
                });
                if let Some(step) = bare_wait {
                    return Err(Error::new(format!(
                        "a bare Wait waits for the prompt, and demogod cannot see {shell}'s"
                    ))
                    .at(step.location.clone())
                    .help("wait for what the command prints, like Wait /done/, or Set WaitPattern"));
                }
            }
        }
        if tape.steps().any(|step| matches!(step.command, Command::Open(_))) {
            crate::browser::find_chrome()?;
        }
        stage(tape)?;
        if !tape.settings.directory.is_dir() {
            return Err(Error::new(format!("Set Directory: {} is not a directory", tape.settings.directory.display())));
        }

        Ok(())
    }

    /// Plays the tape and returns the recording, without saving it anywhere.
    pub fn record(mut self) -> Result<Recording> {
        self.play()
    }

    /// Plays the tape and saves it to every output, returning what was written. A tape without
    /// an `Output` is saved beside itself as a GIF: `demo.tape` to `demo.gif`.
    pub fn run(mut self) -> Result<Vec<Saved>> {
        if self.tape.outputs.is_empty() {
            let stem = self.tape.path.file_stem().and_then(|stem| stem.to_str()).filter(|stem| *stem != "<tape>");
            self.tape.outputs.push(self.tape.path.with_file_name(format!("{}.gif", stem.unwrap_or("demo"))));
        }
        let recording = self.play()?;
        let outputs = self.tape.outputs.clone();

        outputs
            .iter()
            .map(|path| {
                let saved = recording.save(path)?;
                self.emit(&Event::Saved(saved.clone()));
                Ok(saved)
            })
            .collect()
    }

    fn play(&mut self) -> Result<Recording> {
        self.check()?;
        let recording = Player::new(&self.tape)?.play(&self.tape, &mut self.actions, &mut self.listener)?;
        self.emit(&Event::Recorded { duration: recording.duration() });

        Ok(recording)
    }

    fn emit(&mut self, event: &Event) {
        if let Some(listener) = &mut self.listener {
            listener(event);
        }
    }
}

/// What one keystroke sends: a character to type, or a key to press.
#[derive(Clone, Copy)]
enum Keystroke<'a> {
    Text(char),
    Key(&'a crate::Key),
}

/// Everything that exists only while a tape plays.
struct Player {
    renderer: Renderer,
    epoch: Instant,
    clock: Clock,
    timing: Timing,
    terminal: Option<Terminal>,
    browser: Option<Browser>,
    hidden: bool,
    clipboard: String,
    panes: Vec<Timed<PaneFrame>>,
    scenes: Vec<Timed<usize>>,
    pointer: Vec<Timed<Pointer>>,
    screenshots: Vec<(Duration, PathBuf)>,
    warnings: Vec<String>,
}

impl Player {
    fn new(tape: &Tape) -> Result<Player> {
        let (renderer, settings, warnings) = stage(tape)?;
        let settings = &settings;
        let epoch = Instant::now();

        let terminal = if tape.uses_terminal() {
            let (rows, columns) = renderer.layout.grid(&renderer.font, settings.padding);
            Some(Terminal::start(Launch {
                shell: &settings.shell,
                prompt: &settings.prompt,
                directory: &settings.directory,
                env: &tape.env,
                rows,
                columns,
                epoch,
            })?)
        } else {
            None
        };

        Ok(Player {
            renderer,
            epoch,
            clock: Clock::starting_at(epoch.elapsed()),
            timing: settings.timing,
            terminal,
            browser: None,
            hidden: false,
            clipboard: String::new(),
            panes: Vec::new(),
            scenes: Vec::new(),
            pointer: Vec::new(),
            screenshots: Vec::new(),
            warnings,
        })
    }

    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    /// Ends a stretch of the film here: `nominal` long under tape timing, as long as it really
    /// was under real timing, and not at all while hidden.
    fn mark(&mut self, nominal: Duration) {
        let now = self.now();
        let shown = match (self.hidden, self.timing) {
            (true, _) => Duration::ZERO,
            (false, Timing::Tape) => nominal,
            (false, Timing::Real) => now.saturating_sub(self.clock.last_real()),
        };
        self.clock.mark(now, shown);
    }

    /// Ends a stretch of the film that is not shown, whatever the timing: an action, or the
    /// search for an element.
    fn skip(&mut self) {
        self.clock.mark(self.now(), Duration::ZERO);
    }

    fn play(
        mut self,
        tape: &Tape,
        actions: &mut HashMap<String, Action>,
        listener: &mut Option<Listener>,
    ) -> Result<Recording> {
        if let Some(listener) = listener {
            for message in std::mem::take(&mut self.warnings) {
                listener(&Event::Warning { message });
            }
        }
        let numbers = tape.caption_numbers();
        let total = numbers.iter().flatten().count();
        for (index, (scene, number)) in tape.scenes.iter().zip(numbers).enumerate() {
            self.scenes.push(Timed { at: self.now(), value: index });
            if let Some(listener) = listener {
                listener(&Event::Scene { number, total, caption: scene.caption.clone() });
            }
            for step in &scene.steps {
                let shown =
                    self.step(tape, step, actions, listener).map_err(|error| error.at(step.location.clone()))?;
                self.mark(shown);
            }
        }
        self.finish(tape)
    }

    /// Runs one step, and returns how long it lasts in the film under tape timing.
    fn step(
        &mut self,
        tape: &Tape,
        step: &Step,
        actions: &mut HashMap<String, Action>,
        listener: &mut Option<Listener>,
    ) -> Result<Duration> {
        let settings = &tape.settings;
        match (&step.command, step.focus) {
            (Command::Type { text, delay }, focus) => {
                for character in text.chars() {
                    self.keystroke(focus, Keystroke::Text(character), *delay)?;
                }
                Ok(Duration::ZERO)
            }
            (Command::Press { key, count, delay }, focus) => {
                for _ in 0..*count {
                    self.keystroke(focus, Keystroke::Key(key), *delay)?;
                }
                Ok(Duration::ZERO)
            }
            (Command::Paste, Focus::Terminal) => {
                let text = self.clipboard.clone();
                self.terminal()?.send(text.as_bytes())?;
                Ok(settings.typing_speed)
            }
            (Command::Paste, Focus::Browser) => {
                let text = self.clipboard.clone();
                self.browser()?.insert_text(&text)?;
                Ok(settings.typing_speed)
            }
            (Command::Copy(text), _) => {
                self.clipboard = text.clone();
                Ok(Duration::ZERO)
            }
            (Command::Sleep(duration), _) => {
                std::thread::sleep(*duration);
                Ok(*duration)
            }
            (Command::Wait { pattern, scope, timeout }, Focus::Terminal) => {
                self.terminal()?.wait(pattern.as_ref(), *scope, *timeout)?;
                Ok(settings.wait_duration)
            }
            (Command::Wait { pattern, timeout, .. }, Focus::Browser) => {
                self.browser()?.wait(pattern.as_ref(), *timeout)?;
                Ok(settings.wait_duration)
            }
            (Command::Hide, _) => {
                self.hidden = true;
                Ok(Duration::ZERO)
            }
            (Command::Show, _) => {
                self.hidden = false;
                Ok(Duration::ZERO)
            }
            (Command::Screenshot(path), _) => {
                self.screenshots.push((self.now(), path.clone()));
                Ok(Duration::ZERO)
            }
            (Command::Do(name), _) => {
                match actions.get_mut(name) {
                    Some(action) => action().map_err(|message| Error::new(format!("Do {name} failed: {message}")))?,
                    None => run_shell_action(tape, name)?,
                }
                self.skip();
                if let Some(listener) = listener {
                    listener(&Event::Action { name: name.clone() });
                }
                Ok(Duration::ZERO)
            }
            (Command::Image(path), _) => {
                let image = Image::open_png(path, settings.theme.background)?;
                self.panes.push(Timed { at: self.now(), value: PaneFrame { image: Arc::new(image), url: None } });
                Ok(Duration::ZERO)
            }
            (Command::Open(url), _) => {
                if self.browser.is_none() {
                    let size = self.renderer.layout.page_size();
                    self.browser = Some(Browser::launch(size, settings.browser_zoom, self.epoch)?);
                }
                self.browser()?.open(url, settings.wait_timeout)?;
                Ok(settings.wait_duration)
            }
            (Command::Click { target, timeout }, _) => {
                self.point_at(target, *timeout)?;
                let pointer = self.last_pointer();
                self.pointer.push(Timed { at: self.now(), value: Pointer { pressed: true, ..pointer } });
                self.browser()?.click(pointer.x, pointer.y)?;
                std::thread::sleep(Duration::from_millis(50));
                self.pointer.push(Timed { at: self.now(), value: pointer });
                self.mark(CLICK_HOLD);
                Ok(Duration::ZERO)
            }
            (Command::Hover { target, timeout }, _) => {
                self.point_at(target, *timeout)?;
                Ok(Duration::ZERO)
            }
            (Command::Scroll(ScrollTarget::Lines(lines)), Focus::Terminal) => {
                let epoch = self.epoch;
                self.terminal()?.scroll(*lines, epoch);
                Ok(settings.typing_speed.max(Duration::from_millis(100)) * lines.unsigned_abs())
            }
            (Command::Scroll(target), _) => {
                self.browser()?.scroll(target, settings.wait_timeout)?;
                Ok(SCROLL_TIME)
            }
        }
    }

    /// One keystroke, a typing delay after the last. Under tape timing that delay is exactly what
    /// the film shows, and what the keystroke causes — its echo — is shown at the moment it lands,
    /// so typing keeps the same rhythm on any machine.
    fn keystroke(&mut self, focus: Focus, keystroke: Keystroke, delay: Duration) -> Result<()> {
        match self.timing {
            Timing::Real => std::thread::sleep(delay),
            Timing::Tape => self.mark(delay),
        }
        match (focus, keystroke) {
            (Focus::Terminal, keystroke) => {
                let bytes = match keystroke {
                    Keystroke::Text(character) => character.to_string().into_bytes(),
                    Keystroke::Key(key) => key.bytes(),
                };
                self.terminal()?.type_bytes(&bytes, ECHO_PATIENCE)?;
            }
            (Focus::Browser, Keystroke::Text(character)) => {
                self.browser()?.type_character(character)?;
                std::thread::sleep(BROWSER_KEY_DELAY);
            }
            (Focus::Browser, Keystroke::Key(key)) => {
                self.browser()?.press(key)?;
                std::thread::sleep(BROWSER_KEY_DELAY);
            }
        }
        if self.timing == Timing::Tape {
            self.mark(Duration::ZERO);
        }

        Ok(())
    }

    /// Finds an element and moves the pointer onto it, marking the clock for the search (which
    /// the film skips) and the travel (which it shows).
    fn point_at(&mut self, target: &str, timeout: Duration) -> Result<()> {
        let (x, y) = self.browser()?.locate(target, timeout)?;
        let from = self.last_pointer();
        self.skip();
        self.pointer.push(Timed { at: self.now(), value: from });
        self.browser()?.move_pointer((from.x, from.y), (x, y))?;
        self.pointer.push(Timed { at: self.now(), value: Pointer { x, y, pressed: false } });
        self.mark(POINTER_TRAVEL);

        Ok(())
    }

    /// Where the pointer comes from the first time: the lower right of the page.
    fn pointer_start(&self) -> Pointer {
        let (width, height) = self.renderer.layout.page_size();
        let zoom = self.renderer.zoom;
        Pointer { x: width as f32 / zoom * 0.8, y: height as f32 / zoom * 0.85, pressed: false }
    }

    /// Where the pointer is now.
    fn last_pointer(&self) -> Pointer {
        self.pointer.last().map_or_else(|| self.pointer_start(), |event| event.value)
    }

    fn terminal(&mut self) -> Result<&mut Terminal> {
        self.terminal.as_mut().ok_or_else(|| Error::new("there is no terminal"))
    }

    fn browser(&mut self) -> Result<&mut Browser> {
        self.browser.as_mut().ok_or_else(|| {
            Error::new("the browser is not open").help("Open a page first: Open \"http://localhost:3000\"")
        })
    }

    /// Everything gathered, put on the film's clock.
    fn finish(mut self, tape: &Tape) -> Result<Recording> {
        let (screens, output) = match &self.terminal {
            Some(terminal) => terminal.take(),
            None => (Vec::new(), Vec::new()),
        };
        let mut panes = std::mem::take(&mut self.panes);
        if let Some(browser) = self.browser.take() {
            panes.extend(browser.finish()?);
            panes.sort_by_key(|event| event.at);
        }
        drop(self.terminal.take());

        let clock = &self.clock;
        let settings = &tape.settings;
        let recorded = RecordedTracks {
            screens: moments(&screens),
            panes: moments(&panes),
            scenes: self.scenes.clone(),
            pointer: self.pointer.clone(),
        };
        let captions = tape
            .scenes
            .iter()
            .zip(tape.caption_numbers())
            .map(|(scene, number)| Some((number? - 1, scene.caption.clone()?)))
            .collect();
        let recording = Recording {
            screens: clock.place(screens),
            output: clock.place(output),
            panes: clock.place(panes),
            scenes: clock.place(self.scenes),
            captions,
            pointer: clock.place(self.pointer),
            length: clock.length(),
            framerate: settings.framerate,
            playback_speed: settings.playback_speed,
            cursor_blink: settings.cursor_blink,
            loop_start: settings.loop_offset.within(clock.length(), settings.framerate),
            shell: settings.shell.clone(),
            renderer: self.renderer,
        };
        for (at, path) in &self.screenshots {
            let image = recording.draw_as_recorded(*at, &recorded);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
            }
            std::fs::write(path, image.to_png()).map_err(|error| Error::io(path, error))?;
        }

        Ok(recording)
    }
}

/// Everything a tape looks like before anything runs: the renderer, the settings it was made
/// with (sized to `Columns` and `Rows`), and warnings about fonts that are not installed. Fails
/// when the settings leave a window no room.
fn stage(tape: &Tape) -> Result<(Renderer, Settings, Vec<String>)> {
    let faces = Faces::load(&tape.settings.font_family)?;
    let warnings = faces
        .missing()
        .iter()
        .map(|family| format!("the font {family} is not installed; drawing with the next one instead"))
        .collect();
    let settings = fit_to_grid(&tape.settings, &faces, tape);
    let pane_is_browser = tape.steps().any(|step| matches!(step.command, Command::Open(_)));
    let layout = Layout::new(&settings, tape.has_captions(), tape.uses_terminal(), tape.uses_pane(), pane_is_browser);
    let captioned = tape.caption_numbers().into_iter().flatten().count();
    let renderer = Renderer::new(&settings, faces, layout, captioned);
    if let Some(problem) = renderer.layout.problem(&renderer.font, settings.padding) {
        return Err(Error::new(problem).help("make Width or Height bigger, or Margin, Padding or FontSize smaller"));
    }

    Ok((renderer, settings, warnings))
}

/// The settings with `Width` and `Height` made to fit `Set Columns` and `Set Rows` exactly, when
/// the tape sets them.
fn fit_to_grid(settings: &Settings, faces: &Faces, tape: &Tape) -> Settings {
    let mut fitted = settings.clone();
    if settings.columns.is_none() && settings.rows.is_none() {
        return fitted;
    }
    let font = crate::font::Font::new(faces.clone(), settings.font_size, settings.line_height, settings.letter_spacing);
    let layout = Layout::new(settings, tape.has_captions(), true, tape.uses_pane(), false);
    let terminal = layout.terminal.expect("a terminal was asked for");
    let padding = 2 * settings.padding;
    if let Some(columns) = settings.columns {
        let wanted = columns as u32 * font.cell_width + padding;
        fitted.width = (settings.width as i64 + wanted as i64 - terminal.body.width as i64).clamp(16, 8192) as u32;
    }
    if let Some(rows) = settings.rows {
        let wanted = rows as u32 * font.cell_height + padding;
        fitted.height = (settings.height as i64 + wanted as i64 - terminal.body.height as i64).clamp(16, 8192) as u32;
    }
    if let Length::Percent(_) = settings.pane_width {
        // The pane keeps the width it had, rather than growing with the terminal.
        fitted.pane_width = layout.pane.map_or(settings.pane_width, |pane| Length::Pixels(pane.frame.width));
    }

    fitted
}

/// Runs a tape's `Action <name> "<command>"` in the shell's directory, with the tape's `Env`.
fn run_shell_action(tape: &Tape, name: &str) -> Result<()> {
    let command = tape.actions.get(name).ok_or_else(|| Error::new(format!("no action is named {name}")))?;
    let mut shell = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
    #[cfg(windows)]
    {
        // cmd.exe reads its command line whole; Rust's quoting of arguments would change it.
        use std::os::windows::process::CommandExt;
        shell.raw_arg(format!("/C {command}"));
    }
    #[cfg(not(windows))]
    shell.args(["-c", command]);
    let output = shell
        .current_dir(&tape.settings.directory)
        .envs(tape.env.iter().map(|(key, value)| (key, value)))
        .output()
        .map_err(|error| Error::new(format!("Do {name}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(
            Error::new(format!("Do {name} failed ({}): {command}", output.status)).help(stderr.trim().to_string())
        );
    }

    Ok(())
}
