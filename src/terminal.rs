//! A shell in a real PTY, and every screen it showed.
//!
//! A thread reads the PTY into a `vt100` terminal and keeps a snapshot of the screen each time it
//! changes, with the moment it did. Nothing here draws anything: the snapshots are data, and
//! [`crate::render`] turns them into pixels later, as fast as the machine allows.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::timeline::Timed;
use crate::{Error, Result, WaitScope};

/// How many lines scrolled off the top are kept, for `ScrollUp`.
const SCROLLBACK: usize = 1000;

/// The prompt marker: OSC 133;A, the "a prompt starts here" of shell integration. Invisible, and
/// what lets a bare `Wait` know a command has finished without guessing at the prompt's text.
const PROMPT_MARK: &str = "\x1b]133;A\x07";

/// A cell's color, before a theme says what it looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Color {
    /// The theme's foreground or background.
    #[default]
    Default,
    /// One of the 256 palette entries.
    Indexed(u8),
    /// A 24-bit color.
    Rgb(u8, u8, u8),
}

/// One character on the screen, and how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub character: char,
    pub foreground: Color,
    pub background: Color,
    pub style: u8,
}

impl Cell {
    pub const BOLD: u8 = 1;
    pub const DIM: u8 = 2;
    pub const ITALIC: u8 = 4;
    pub const UNDERLINE: u8 = 8;
    pub const INVERSE: u8 = 16;
    /// The first half of a character two columns wide.
    pub const WIDE: u8 = 32;
    /// The second half, which draws nothing of its own.
    pub const WIDE_TAIL: u8 = 64;

    pub fn has(&self, style: u8) -> bool {
        self.style & style != 0
    }
}

/// The terminal's screen at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Screen {
    pub rows: u16,
    pub columns: u16,
    pub cells: Vec<Cell>,
    /// Where the cursor is, unless the program hid it.
    pub cursor: Option<(u16, u16)>,
}

impl Screen {
    /// Every row as text, trailing spaces trimmed.
    pub fn text(&self) -> String {
        (0..self.rows)
            .map(|row| {
                let line: String = (0..self.columns)
                    .map(|column| self.cell(row, column))
                    .filter(|cell| !cell.has(Cell::WIDE_TAIL))
                    .map(|cell| cell.character)
                    .collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn cell(&self, row: u16, column: u16) -> &Cell {
        &self.cells[row as usize * self.columns as usize + column as usize]
    }

    pub fn capture(screen: &vt100::Screen) -> Screen {
        let (rows, columns) = screen.size();
        let mut cells = Vec::with_capacity(rows as usize * columns as usize);
        for row in 0..rows {
            for column in 0..columns {
                let cell = screen.cell(row, column).expect("inside the screen");
                let flags = [
                    (cell.bold(), Cell::BOLD),
                    (cell.dim(), Cell::DIM),
                    (cell.italic(), Cell::ITALIC),
                    (cell.underline(), Cell::UNDERLINE),
                    (cell.inverse(), Cell::INVERSE),
                    (cell.is_wide(), Cell::WIDE),
                    (cell.is_wide_continuation(), Cell::WIDE_TAIL),
                ];
                cells.push(Cell {
                    character: cell.contents().chars().next().unwrap_or(' '),
                    foreground: color(cell.fgcolor()),
                    background: color(cell.bgcolor()),
                    style: flags.iter().filter(|(on, _)| *on).fold(0, |style, (_, flag)| style | flag),
                });
            }
        }
        let (row, column) = screen.cursor_position();

        Screen { rows, columns, cells, cursor: (!screen.hide_cursor()).then_some((row, column.min(columns - 1))) }
    }
}

fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

/// Counts the prompt marks the shell prints, which `vt100` hands to a callback rather than
/// keeping.
#[derive(Default)]
struct Marks {
    prompts: usize,
}

impl vt100::Callbacks for Marks {
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.first() == Some(&b"133".as_slice()) && params.get(1) == Some(&b"A".as_slice()) {
            self.prompts += 1;
        }
    }
}

/// What the reader thread shares with whoever is waiting on it.
struct Shared {
    parser: vt100::Parser<Marks>,
    screens: Vec<Timed<Screen>>,
    output: Vec<Timed<Vec<u8>>>,
    exited: bool,
}

/// How to start a terminal.
pub(crate) struct Launch<'a> {
    pub shell: &'a str,
    pub prompt: &'a str,
    pub directory: &'a Path,
    pub env: &'a [(String, String)],
    pub rows: u16,
    pub columns: u16,
    /// The moment everything recorded is timed from.
    pub epoch: Instant,
}

/// A shell running in a PTY.
pub(crate) struct Terminal {
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    shared: Arc<(Mutex<Shared>, Condvar)>,
    /// How many prompts had been printed when Enter was last pressed.
    prompts_at_enter: usize,
    /// Kept so the rc files outlive the shell that reads them.
    _scratch: Option<Scratch>,
}

impl Terminal {
    /// Starts the shell and waits for its first prompt.
    pub fn start(launch: Launch) -> Result<Terminal> {
        let pty = native_pty_system()
            .openpty(PtySize { rows: launch.rows, cols: launch.columns, pixel_width: 0, pixel_height: 0 })
            .map_err(|error| Error::new(format!("could not open a terminal: {error}")))?;
        let (mut command, scratch) = shell_command(launch.shell, launch.prompt)?;
        command.cwd(launch.directory);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("HISTFILE", if cfg!(windows) { "NUL" } else { "/dev/null" });
        if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
            command.env("LANG", "C.UTF-8");
        }
        for (key, value) in launch.env {
            command.env(key, value);
        }

        let child = pty.slave.spawn_command(command).map_err(|error| {
            Error::new(format!("could not start {}: {error}", launch.shell)).help("Set Shell to one that is installed")
        })?;
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().map_err(|error| Error::new(error.to_string()))?;
        let writer = pty.master.take_writer().map_err(|error| Error::new(error.to_string()))?;

        let parser = vt100::Parser::new_with_callbacks(launch.rows, launch.columns, SCROLLBACK, Marks::default());
        let shared = Arc::new((
            Mutex::new(Shared { parser, screens: Vec::new(), output: Vec::new(), exited: false }),
            Condvar::new(),
        ));
        let (epoch, reading) = (launch.epoch, shared.clone());
        std::thread::spawn(move || {
            // The master has to outlive the reader on some platforms, so it lives here.
            let _master = pty.master;
            let mut buffer = [0u8; 16 * 1024];
            loop {
                let read = match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let (lock, changed) = &*reading;
                let mut shared = lock.lock().expect("the reader never panics while holding the lock");
                let at = epoch.elapsed();
                shared.parser.process(&buffer[..read]);
                shared.output.push(Timed { at, value: buffer[..read].to_vec() });
                let screen = Screen::capture(shared.parser.screen());
                if shared.screens.last().is_none_or(|last| last.value != screen) {
                    shared.screens.push(Timed { at, value: screen });
                }
                changed.notify_all();
            }
            let (lock, changed) = &*reading;
            lock.lock().expect("the reader never panics while holding the lock").exited = true;
            changed.notify_all();
        });

        let terminal = Terminal { writer, child, shared, prompts_at_enter: 0, _scratch: scratch };
        if marks_prompt(launch.shell) {
            let first_prompt =
                terminal.wait_for(Duration::from_secs(10), |shared| shared.parser.callbacks().prompts > 0);
            if first_prompt.is_err() {
                return Err(Error::new(format!("{} showed no prompt in 10s", launch.shell)));
            }
        } else {
            terminal.wait_until_quiet(Duration::from_millis(300), Duration::from_secs(10));
        }

        Ok(terminal)
    }

    /// Sends bytes as if they were typed.
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.contains(&b'\r') {
            let prompts = self.lock().parser.callbacks().prompts;
            self.prompts_at_enter = prompts;
        }
        self.writer.write_all(bytes).and_then(|()| self.writer.flush()).map_err(|error| {
            Error::new(format!("the shell stopped reading input: {error}"))
                .help("a command in the tape may have exited the shell")
        })
    }

    /// Types bytes, then waits for the program to echo something back, for at most `patience`.
    ///
    /// Waiting for the echo rather than a fixed time keeps one keystroke one frame on a loaded
    /// machine, and makes typing as fast as the program on an idle one.
    pub fn type_bytes(&mut self, bytes: &[u8], patience: Duration) -> Result<()> {
        let before = self.lock().output.len();
        self.send(bytes)?;
        let _ = self.wait_for(patience, |shared| shared.output.len() > before);

        Ok(())
    }

    /// Blocks until the screen matches, or `timeout` passes. `None` waits for the prompt to come
    /// back after the last Enter.
    pub fn wait(&self, pattern: Option<&crate::Pattern>, scope: WaitScope, timeout: Duration) -> Result<()> {
        let after = self.prompts_at_enter;
        self.wait_for(timeout, |shared| match pattern {
            None => shared.parser.callbacks().prompts > after,
            Some(pattern) => {
                let text = shared.parser.screen().contents();
                match scope {
                    WaitScope::Screen => pattern.matches(&text),
                    WaitScope::Line => {
                        pattern.matches(text.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or(""))
                    }
                }
            }
        })
        .map_err(|()| {
            let wanted = pattern.map_or("the prompt to come back".to_string(), |pattern| pattern.to_string());
            Error::new(format!("waited {}s for {wanted}", timeout.as_secs_f32()))
                .help(format!("the screen was:\n{}", indent(self.text().trim_end())))
        })
    }

    /// Moves the view through the scrollback by `lines`: up when negative, back down when
    /// positive, as far as there is to go.
    pub fn scroll(&self, lines: i32, epoch: Instant) {
        let mut shared = self.lock();
        let offset = (shared.parser.screen().scrollback() as i64 - lines as i64).max(0) as usize;
        shared.parser.screen_mut().set_scrollback(offset);
        let screen = Screen::capture(shared.parser.screen());
        if shared.screens.last().is_none_or(|last| last.value != screen) {
            shared.screens.push(Timed { at: epoch.elapsed(), value: screen });
        }
    }

    /// The screen as text, one line per row.
    pub fn text(&self) -> String {
        self.lock().parser.screen().contents()
    }

    /// Everything recorded so far: the screens, and the raw bytes behind them.
    pub fn take(&self) -> (Vec<Timed<Screen>>, Vec<Timed<Vec<u8>>>) {
        let mut shared = self.lock();
        let screens = std::mem::take(&mut shared.screens);
        // The last screen stays, so what is taken next starts from what is on screen now.
        if let Some(last) = screens.last() {
            shared.screens.push(last.clone());
        }

        (screens, std::mem::take(&mut shared.output))
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.0.lock().expect("the reader never panics while holding the lock")
    }

    /// Waits for a shell that does not mark its prompt: until it has printed something and then
    /// nothing for `quiet`, or `timeout` passes.
    fn wait_until_quiet(&self, quiet: Duration, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut seen = 0;
        while Instant::now() < deadline {
            let printed = self.lock().output.len();
            if printed > 0 && printed == seen {
                return;
            }
            seen = printed;
            std::thread::sleep(quiet);
        }
    }

    fn wait_for(&self, timeout: Duration, done: impl Fn(&Shared) -> bool) -> Result<(), ()> {
        let (lock, changed) = &*self.shared;
        let deadline = Instant::now() + timeout;
        let mut shared = lock.lock().expect("the reader never panics while holding the lock");
        while !done(&shared) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || shared.exited {
                return Err(());
            }
            shared = changed.wait_timeout(shared, left).expect("the reader never panics while holding the lock").0;
        }

        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("    │ {line}")).collect::<Vec<_>>().join("\n")
}

/// A directory that is removed when dropped.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The command that starts a shell with nothing of the user's configuration and a prompt that
/// marks itself — or, for a shell this does not know, the command as written.
fn shell_command(shell: &str, prompt: &str) -> Result<(CommandBuilder, Option<Scratch>)> {
    let (program, arguments) = split_shell(shell);
    let (symbol, gap) = split_prompt(prompt);
    let mut command = CommandBuilder::new(program);
    if !arguments.is_empty() {
        command.args(&arguments);
        return Ok((command, None));
    }

    let scratch = || -> Result<Scratch> {
        let directory = scratch_directory("shell");
        std::fs::create_dir_all(&directory).map_err(|error| Error::io(&directory, error))?;
        Ok(Scratch(directory))
    };
    let write = |scratch: &Scratch, file: &str, contents: String| -> Result<()> {
        let path = scratch.0.join(file);
        std::fs::write(&path, contents).map_err(|error| Error::io(&path, error))
    };

    match shell_name(program).as_str() {
        "bash" => {
            let scratch = scratch()?;
            // Readline redraws the whole prompt from column 0 when anything invisible follows a
            // character wider than a byte, so `❯` goes uncolored rather than jump over output.
            let ps1 = if symbol.is_ascii() {
                format!("\\[{PROMPT_MARK}\\e[1;35m\\]{symbol}\\[\\e[0m\\]{gap}")
            } else {
                format!("\\[{PROMPT_MARK}\\]{symbol}{gap}")
            };
            write(&scratch, "bashrc", format!("PS1='{}'\nPS2=''\nset +m\nunset PROMPT_COMMAND\n", quote(&ps1)))?;
            command.args(["--noprofile", "--rcfile"]);
            command.arg(scratch.0.join("bashrc"));
            command.arg("-i");
            Ok((command, Some(scratch)))
        }
        "zsh" => {
            let scratch = scratch()?;
            let prompt = format!("%{{{PROMPT_MARK}%}}%B%F{{magenta}}{}%f%b{gap}", symbol.replace('%', "%%"));
            write(&scratch, ".zshrc", format!("PROMPT='{}'\nRPROMPT=''\nunsetopt PROMPT_SP\n", quote(&prompt)))?;
            command.env("ZDOTDIR", scratch.0.as_os_str());
            command.arg("-i");
            Ok((command, Some(scratch)))
        }
        "fish" => {
            let function = format!(
                "function fish_prompt; printf '\\e]133;A\\a'; set_color -o magenta; printf '%s' '{}'; set_color normal; printf '{gap}'; end; function fish_greeting; end",
                quote(symbol)
            );
            command.args(["--no-config", "-i", "-C", &function]);
            Ok((command, None))
        }
        "sh" | "dash" | "ash" => {
            command.env("PS1", format!("{PROMPT_MARK}\x1b[1;35m{symbol}\x1b[0m{gap}"));
            command.arg("-i");
            Ok((command, None))
        }
        "powershell" | "pwsh" => {
            // Inside a double-quoted PowerShell string, a backtick escapes `$`, `"` and itself.
            let escape = |text: &str| text.replace('`', "``").replace('$', "`$").replace('"', "`\"");
            let function = format!(
                "function prompt {{ \"$([char]27)]133;A$([char]7)$([char]27)[1;35m{}$([char]27)[0m{}\" }}",
                escape(symbol),
                escape(gap),
            );
            command.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &function]);
            Ok((command, None))
        }
        _ => Ok((command, None)),
    }
}

/// The program and its arguments from `Set Shell`: a path to an existing file is taken whole, so
/// it may have spaces in it; anything else is split on spaces.
pub(crate) fn split_shell(shell: &str) -> (&str, Vec<&str>) {
    if Path::new(shell.trim()).is_file() {
        return (shell.trim(), Vec::new());
    }
    let mut words = shell.split_whitespace();
    (words.next().unwrap_or("bash"), words.collect())
}

/// `bash` for `/usr/bin/bash` or `C:\Git\bin\bash.exe`.
fn shell_name(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    file.strip_suffix(".exe").map(str::to_string).unwrap_or(file)
}

/// Whether demogod starts this shell itself, with a prompt that marks itself — so a bare `Wait`
/// knows when a command has finished. A shell given with arguments is started as written.
pub(crate) fn marks_prompt(shell: &str) -> bool {
    let (program, arguments) = split_shell(shell);
    let known = ["bash", "zsh", "fish", "sh", "dash", "ash", "powershell", "pwsh"];
    arguments.is_empty() && known.contains(&shell_name(program).as_str())
}

/// A fresh directory name under the system's temporary directory, unique to this process.
pub(crate) fn scratch_directory(purpose: &str) -> std::path::PathBuf {
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    std::env::temp_dir().join(format!("demogod-{purpose}-{}-{count}-{nanos}", std::process::id()))
}

/// `"❯ "` into what is colored and the space after it, which is not.
fn split_prompt(prompt: &str) -> (&str, &str) {
    let symbol = prompt.trim_end();
    (symbol, &prompt[symbol.len()..])
}

/// Escapes for the inside of a single-quoted shell string.
fn quote(text: &str) -> String {
    text.replace('\'', r"'\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_is_split_into_its_symbol_and_its_gap() {
        assert_eq!(split_prompt("❯ "), ("❯", " "));
        assert_eq!(split_prompt("$"), ("$", ""));
        assert_eq!(split_prompt("λ  "), ("λ", "  "));
    }

    #[test]
    fn quotes_survive_single_quoting() {
        assert_eq!(quote("it's"), r"it'\''s");
    }

    #[test]
    fn a_shell_path_may_have_spaces_and_arguments_are_split() {
        let here = std::env::current_exe().unwrap();
        let path = here.to_str().unwrap();
        assert_eq!(split_shell(path), (path, Vec::new()));
        assert_eq!(split_shell("bash --posix -i"), ("bash", vec!["--posix", "-i"]));
    }

    #[test]
    fn only_shells_started_clean_mark_their_prompt() {
        assert!(marks_prompt("bash") && marks_prompt("/usr/bin/zsh") && marks_prompt(r"C:\Windows\pwsh.exe"));
        assert!(!marks_prompt("bash --norc") && !marks_prompt("nu") && !marks_prompt("cmd"));
    }

    #[test]
    fn powershell_prompts_escape_what_powershell_would_expand() {
        let (command, _) = shell_command("pwsh", "$ `x` ").unwrap();
        let argv: Vec<_> = command.get_argv().iter().map(|arg| arg.to_string_lossy().to_string()).collect();
        assert!(argv.last().unwrap().contains("`$ ``x``"), "{argv:?}");
    }

    #[test]
    fn scratch_directories_are_plain_and_unique() {
        let (one, two) = (scratch_directory("x"), scratch_directory("x"));
        assert_ne!(one, two);
        let name = one.file_name().unwrap().to_str().unwrap();
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{name}");
    }

    #[test]
    fn a_shell_with_arguments_is_run_as_written() {
        let (command, scratch) = shell_command("bash --posix", "❯ ").unwrap();
        assert!(scratch.is_none());
        let argv: Vec<_> = command.get_argv().iter().map(|arg| arg.to_string_lossy().to_string()).collect();
        assert_eq!(argv, ["bash", "--posix"]);
    }

    #[test]
    fn prompt_marks_are_counted() {
        let mut parser = vt100::Parser::new_with_callbacks(4, 20, 0, Marks::default());
        parser.process(format!("{PROMPT_MARK}❯ ls\r\nfile\r\n{PROMPT_MARK}❯ ").as_bytes());

        assert_eq!(parser.callbacks().prompts, 2);
        assert_eq!(parser.screen().contents(), "❯ ls\nfile\n❯ ");
    }

    #[test]
    fn a_screen_captures_colors_styles_and_wide_characters() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process("\x1b[1;31mA\x1b[0m\x1b[38;2;1;2;3;48;5;4mB\x1b[0m界\x1b[?25l".as_bytes());
        let screen = Screen::capture(parser.screen());

        assert_eq!(screen.cell(0, 0).foreground, Color::Indexed(1));
        assert!(screen.cell(0, 0).has(Cell::BOLD));
        assert_eq!(screen.cell(0, 1).foreground, Color::Rgb(1, 2, 3));
        assert_eq!(screen.cell(0, 1).background, Color::Indexed(4));
        assert!(screen.cell(0, 2).has(Cell::WIDE));
        assert!(screen.cell(0, 3).has(Cell::WIDE_TAIL));
        assert_eq!(screen.cursor, None);
    }
}
