//! The tape language: a `.tape` file in, a [`Tape`] out.
//!
//! Every command VHS has reads the same here, so a VHS tape is a demogod tape. On top of those:
//!
//! | Command                  | What it does                                                    |
//! |--------------------------|-----------------------------------------------------------------|
//! | `Caption "title" "more"` | starts a scene, and writes its title in a strip along the top   |
//! | `Open "http://…"`        | opens a browser pane beside the terminal and goes to the page   |
//! | `Click "button.save"`    | moves the pointer to an element and clicks it                   |
//! | `Hover "nav a"`          | moves the pointer to an element                                 |
//! | `Scroll 400`             | scrolls the page by pixels, or to an element: `Scroll "#faq"`   |
//! | `Focus terminal`         | sends the keys that follow to the terminal (or `browser`)       |
//! | `Image "shot.png"`       | shows a picture in the pane                                     |
//! | `Action fix "git stash"` | names a shell command…                                          |
//! | `Do fix`                 | …and runs it off camera, between two keystrokes                 |
//!
//! Parsing does everything that can be done without running anything: every path is made
//! absolute, every keystroke knows which view it goes to, every `Type` knows its speed. What is
//! left for recording is only what the tape cannot know.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::error::resolve;
use crate::settings::{self, STEP_SETTINGS};
use crate::{Error, Key, KeyCode, Location, Result, Settings};

/// A parsed tape: its settings, what it writes, and what happens, scene by scene.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Tape {
    /// The file it was read from — for one parsed from a string, `<tape>` in the current directory.
    pub path: PathBuf,
    /// Every `Set`, applied.
    pub settings: Settings,
    /// Every `Output` path, absolute.
    pub outputs: Vec<PathBuf>,
    /// Every `Require`d program.
    pub requires: Vec<String>,
    /// Every `Env`, in order: the shell starts with these set.
    pub env: Vec<(String, String)>,
    /// Every `Action <name> "<command>"`: the shell commands a `Do` can run.
    pub actions: BTreeMap<String, String>,
    /// The scenes, in order. A tape without a `Caption` is one scene without one.
    pub scenes: Vec<Scene>,
}

/// A caption, and everything that happens while it is on screen.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Scene {
    /// The caption strip's words, or nothing for steps before the first `Caption`.
    pub caption: Option<Caption>,
    /// What happens, in order.
    pub steps: Vec<Step>,
}

/// What the caption strip says during a scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caption {
    /// Bold, on the left.
    pub title: String,
    /// Dimmer, after the title. May be empty.
    pub detail: String,
}

/// Where a step's keys go and which screen its `Wait` reads: `Focus terminal` or `Focus browser`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Focus {
    /// The terminal: keys go to the shell, `Wait` reads the screen.
    #[default]
    Terminal,
    /// The browser pane: keys go to the page, `Wait` reads its text.
    Browser,
}

/// One line of a tape that does something.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Step {
    /// Where it is written, for errors that happen while it runs.
    pub location: Location,
    /// Where its keys go, or which screen it reads.
    pub focus: Focus,
    /// What it does.
    pub command: Command,
}

/// What a step does.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// `Type "text"`: types it, one character every `delay`.
    Type {
        /// The characters, as written.
        text: String,
        /// The time between two of them.
        delay: Duration,
    },
    /// `Enter`, `Ctrl+C`, `Backspace 3`: presses a key `count` times, `delay` apart.
    Press {
        /// The key, with its modifiers.
        key: Key,
        /// How many times.
        count: u32,
        /// The time between two presses.
        delay: Duration,
    },
    /// `Sleep 2s`.
    Sleep(Duration),
    /// `Wait /pattern/`: blocks until the view shows it, or fails after `timeout`.
    Wait {
        /// What to wait for. Nothing means: the terminal's prompt is back, or the page has loaded.
        pattern: Option<Pattern>,
        /// Where in the terminal to look.
        scope: WaitScope,
        /// How long before the recording fails.
        timeout: Duration,
    },
    /// `Hide`: what follows is not filmed, until `Show`.
    Hide,
    /// `Show`: filming resumes.
    Show,
    /// `Screenshot "file.png"`: saves the film as it is at this moment.
    Screenshot(PathBuf),
    /// `Copy "text"`: puts it on the clipboard `Paste` types from.
    Copy(String),
    /// `Paste`: types whatever was last copied.
    Paste,
    /// `Do name`: runs an action off camera.
    Do(String),
    /// `Open "url"`: opens the browser pane at a page. A path without a scheme is a file.
    Open(String),
    /// `Click "selector"`: moves the pointer to an element and clicks it.
    Click {
        /// A CSS selector, or `text=Visible words`.
        target: String,
        /// How long to look for it.
        timeout: Duration,
    },
    /// `Hover "selector"`: moves the pointer to an element.
    Hover {
        /// A CSS selector, or `text=Visible words`.
        target: String,
        /// How long to look for it.
        timeout: Duration,
    },
    /// `Scroll 400`, `Scroll "#pricing"`, `ScrollUp 3`: scrolls the page, or the terminal.
    Scroll(ScrollTarget),
    /// `Image "file.png"`: shows a picture in the pane.
    Image(PathBuf),
}

/// Where in the terminal `Wait` looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaitScope {
    /// Anywhere on the screen: `Wait` and `Wait+Screen`.
    #[default]
    Screen,
    /// The last line with anything on it: `Wait+Line`.
    Line,
}

/// Where `Scroll`, `ScrollUp` and `ScrollDown` go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrollTarget {
    /// `ScrollUp 3`, `ScrollDown`: the terminal's view through its scrollback moves by lines —
    /// negative is up — or, with the browser focused, the page moves by that many lines of text.
    Lines(i32),
    /// `Scroll 400`: down by this many pixels, or up when negative.
    Pixels(i32),
    /// `Scroll "#pricing"`: until this element is in view.
    Element(String),
}

/// A `/regular expression/`, compared by how it is written.
#[derive(Clone, Debug)]
pub struct Pattern(regex_lite::Regex);

impl Pattern {
    /// The expression as written, without its slashes.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether the text has a match.
    pub fn matches(&self, text: &str) -> bool {
        self.0.is_match(text)
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{}/", self.0.as_str())
    }
}

/// Every command, for "did you mean" when a line starts with one that is not.
const COMMANDS: &[&str] = &[
    "Output",
    "Require",
    "Set",
    "Env",
    "Source",
    "Action",
    "Caption",
    "Focus",
    "Type",
    "Sleep",
    "Wait",
    "Hide",
    "Show",
    "Screenshot",
    "Copy",
    "Paste",
    "Do",
    "Open",
    "Click",
    "Hover",
    "Scroll",
    "ScrollUp",
    "ScrollDown",
    "Image",
    "Enter",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Escape",
    "Space",
    "Up",
    "Down",
    "Left",
    "Right",
    "Home",
    "End",
    "PageUp",
    "PageDown",
];

impl Tape {
    /// Reads and parses a tape file. Relative paths in it are relative to where it is.
    ///
    /// ```no_run
    /// let tape = demogod::Tape::from_file("demo.tape")?;
    /// println!("{} scenes", tape.scenes.len());
    /// # Ok::<(), demogod::Error>(())
    /// ```
    pub fn from_file(path: impl AsRef<Path>) -> Result<Tape> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path).map_err(|error| Error::io(path, error))?;

        Tape::parse_at(&source, path)
    }

    /// Parses a tape from a string, as if it were a file in the current directory.
    ///
    /// ```
    /// use demogod::{Command, Tape};
    ///
    /// let tape = Tape::parse("Output demo.gif\nType \"ls\"\nEnter\nSleep 1s").unwrap();
    /// assert_eq!(tape.scenes[0].steps.len(), 3);
    /// assert!(matches!(tape.scenes[0].steps[0].command, Command::Type { .. }));
    /// ```
    pub fn parse(source: &str) -> Result<Tape> {
        let directory = std::env::current_dir().unwrap_or_default();

        Tape::parse_at(source, &directory.join("<tape>"))
    }

    /// Parses a tape from a string, as if it had been read from `path`.
    pub fn parse_at(source: &str, path: &Path) -> Result<Tape> {
        let path = &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut parser = Parser {
            tape: Tape {
                path: path.to_path_buf(),
                settings: Settings { directory: directory.clone(), ..Settings::default() },
                outputs: Vec::new(),
                requires: Vec::new(),
                env: Vec::new(),
                actions: BTreeMap::new(),
                scenes: vec![Scene::default()],
            },
            focus: Focus::Terminal,
            has_steps: false,
            sources: vec![path.to_path_buf()],
        };
        parser.read(source, path)?;
        let mut tape = parser.tape;
        if tape.scenes.len() > 1 && tape.scenes[0].steps.is_empty() {
            tape.scenes.remove(0);
        }

        Ok(tape)
    }

    /// Every step, scene after scene.
    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.scenes.iter().flat_map(|scene| &scene.steps)
    }

    /// Whether any step types into, or reads, the terminal — if none does, there is no terminal.
    pub fn uses_terminal(&self) -> bool {
        self.steps().any(|step| {
            step.focus == Focus::Terminal
                && matches!(
                    step.command,
                    Command::Type { .. } | Command::Press { .. } | Command::Wait { .. } | Command::Paste
                )
        })
    }

    /// Whether there is a pane beside the terminal: any `Open`, or any `Image`.
    pub fn uses_pane(&self) -> bool {
        self.steps().any(|step| matches!(step.command, Command::Open(_) | Command::Image(_)))
    }

    /// Whether any scene has a caption, and so the film has a caption strip.
    pub fn has_captions(&self) -> bool {
        self.scenes.iter().any(|scene| scene.caption.is_some())
    }

    /// Each scene's number among the scenes with a caption, from 1 — what the caption strip
    /// counts, `2/5` — or nothing for a scene without one.
    ///
    /// ```
    /// let tape = demogod::Tape::parse("Type \"setup\"\nCaption \"One\"\nCaption \"Two\"").unwrap();
    /// assert_eq!(tape.caption_numbers(), [None, Some(1), Some(2)]);
    /// ```
    pub fn caption_numbers(&self) -> Vec<Option<usize>> {
        let mut number = 0;
        self.scenes
            .iter()
            .map(|scene| {
                scene.caption.as_ref().map(|_| {
                    number += 1;
                    number
                })
            })
            .collect()
    }

    /// Every action name a `Do` uses, once each, with the first line that uses it.
    pub fn used_actions(&self) -> Vec<(&str, &Location)> {
        let mut seen = Vec::<(&str, &Location)>::new();
        for step in self.steps() {
            if let Command::Do(name) = &step.command {
                if !seen.iter().any(|(known, _)| known == name) {
                    seen.push((name, &step.location));
                }
            }
        }

        seen
    }
}

struct Parser {
    tape: Tape,
    focus: Focus,
    has_steps: bool,
    sources: Vec<PathBuf>,
}

/// One word or string on a line, and where on the line it is.
#[derive(Clone, Debug)]
struct Token {
    text: String,
    span: std::ops::Range<usize>,
    kind: TokenKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TokenKind {
    Word,
    Quoted,
    Pattern,
}

/// A line being parsed: enough to point an error at the right part of it.
struct Line<'a> {
    text: &'a str,
    location: Location,
}

impl Line<'_> {
    fn error(&self, message: impl Into<String>, span: std::ops::Range<usize>) -> Error {
        Error::new(message).at(self.location.clone()).excerpt(self.text, span)
    }
}

/// A line's tokens, taken a command at a time.
struct Tokens<'a> {
    line: &'a Line<'a>,
    tokens: Vec<Token>,
    next: usize,
}

impl Tokens<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.next)
    }

    fn take(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.next).cloned();
        self.next += token.is_some() as usize;
        token
    }

    /// The next token, if it is an argument rather than the next command.
    fn argument(&mut self) -> Option<Token> {
        match self.peek() {
            Some(token) if token.kind != TokenKind::Word || !is_command(&token.text) => self.take(),
            _ => None,
        }
    }

    /// The next token, if it is a quoted string.
    fn quoted(&mut self) -> Option<Token> {
        self.peek().filter(|token| token.kind == TokenKind::Quoted)?;
        self.take()
    }

    /// The next argument, or an error saying what `head` takes, with an example.
    fn required(&mut self, head: &Token, what: &str, example: &str) -> Result<Token> {
        self.argument().ok_or_else(|| {
            let span = match self.peek() {
                Some(next) => next.span.clone(),
                None => head.span.clone(),
            };
            self.line.error(format!("{} takes {what}", head.text), span).help(example.to_string())
        })
    }

    /// Skips every token that starts before `end`.
    fn skip_to(&mut self, end: usize) {
        while self.peek().is_some_and(|token| token.span.start < end) {
            self.next += 1;
        }
    }
}

impl Parser {
    fn read(&mut self, source: &str, path: &Path) -> Result<()> {
        let file: Arc<Path> = Arc::from(path);
        for (index, text) in source.lines().enumerate() {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let line = Line { text, location: Location { file: file.clone(), line: index + 1 } };
            self.read_line(&line)?;
        }

        Ok(())
    }

    /// Reads every command on a line. VHS allows several — `Type "ls" Sleep 500ms Enter` — so
    /// each takes the arguments it needs, and whatever is left starts the next.
    fn read_line(&mut self, line: &Line) -> Result<()> {
        let mut tokens = Tokens { line, tokens: tokenize(line)?, next: 0 };
        while let Some(head) = tokens.take() {
            self.read_command(&mut tokens, &head)?;
        }

        Ok(())
    }

    fn read_command(&mut self, tokens: &mut Tokens, head: &Token) -> Result<()> {
        let line = tokens.line;
        if head.kind != TokenKind::Word {
            let found = if head.kind == TokenKind::Quoted { "a string" } else { "a pattern" };
            return Err(line
                .error(format!("expected a command, not {found}"), head.span.clone())
                .help("a command takes one string: Type \"two words\""));
        }
        let (name, speed) = match head.text.split_once('@') {
            Some((name, value)) => {
                let at = head.span.start + name.len() + 1;
                let parsed = settings::duration(value).map_err(|message| {
                    line.error(message, at..head.span.end).help("durations are written 500ms, 2s or 1.5s")
                })?;
                (name, Some(parsed))
            }
            None => (head.text.as_str(), None),
        };
        let (base, modifier) = match name.split_once('+') {
            Some(("Wait", scope)) => ("Wait", Some(scope)),
            _ => (name, None),
        };

        let takes_speed =
            matches!(base, "Type" | "Wait" | "Click" | "Hover" | "ScrollUp" | "ScrollDown") || is_key(name);
        if speed.is_some() && !takes_speed {
            let at = head.span.start + name.len();
            return Err(line
                .error(format!("{base} takes no @"), at..head.span.end)
                .help("@ sets how fast Type and keys go, and how long Wait, Click and Hover look"));
        }
        let command = match base {
            "Output" => {
                let path = tokens.required(head, "a file", "Output demo.gif")?;
                self.tape.outputs.push(self.path(&path.text));
                return Ok(());
            }
            "Require" => {
                let program = tokens.required(head, "a program", "Require git")?;
                self.tape.requires.push(program.text);
                return Ok(());
            }
            "Set" => return self.set(tokens, head),
            "Env" => {
                let key = tokens.required(head, "a name and a value", "Env NODE_ENV \"test\"")?;
                let value = tokens.required(head, "a name and a value", "Env NODE_ENV \"test\"")?;
                self.tape.env.push((key.text, value.text));
                return Ok(());
            }
            "Source" => return self.source(tokens, head),
            "Action" => {
                let key = tokens.required(head, "a name and a command", "Action reset \"git stash\"")?;
                let command = tokens.required(head, "a name and a command", "Action reset \"git stash\"")?;
                word(line, &key, "Action")?;
                self.tape.actions.insert(key.text, command.text);
                return Ok(());
            }
            "Caption" => {
                let title = tokens.quoted().ok_or_else(|| {
                    line.error("Caption takes a title, and optionally a detail", head.span.clone())
                        .help("Caption \"Run the tests\" \"in a real browser\"")
                })?;
                let detail = tokens.quoted().map(|token| token.text).unwrap_or_default();
                self.tape
                    .scenes
                    .push(Scene { caption: Some(Caption { title: title.text, detail }), steps: Vec::new() });
                return Ok(());
            }
            "Focus" => {
                let target = tokens.required(head, "terminal or browser", "Focus terminal")?;
                self.focus = match target.text.as_str() {
                    "terminal" => Focus::Terminal,
                    "browser" => Focus::Browser,
                    _ => return Err(line.error("Focus takes terminal or browser", target.span)),
                };
                return Ok(());
            }
            "Type" => {
                let text = tokens.required(head, "a string", "Type \"npm test\"")?;
                Command::Type { text: text.text, delay: speed.unwrap_or(self.tape.settings.typing_speed) }
            }
            "Sleep" => {
                let value = tokens.required(head, "a duration", "Sleep 500ms")?;
                let duration = settings::duration(&value.text).map_err(|message| {
                    line.error(message, value.span.clone()).help("durations are written 500ms, 2s or 1.5s")
                })?;
                Command::Sleep(duration)
            }
            "Wait" => self.wait(tokens, head, modifier, speed)?,
            "Hide" => Command::Hide,
            "Show" => Command::Show,
            "Paste" => Command::Paste,
            "Screenshot" => {
                Command::Screenshot(self.path(&tokens.required(head, "a file", "Screenshot shot.png")?.text))
            }
            "Copy" => Command::Copy(tokens.required(head, "a string", "Copy \"hello\"")?.text),
            "Do" => {
                let action = tokens.required(head, "an action's name", "Do reset")?;
                Command::Do(word(line, &action, "Do")?.to_string())
            }
            "Open" => {
                let url = tokens.required(head, "a URL", "Open \"http://localhost:3000\"")?;
                self.focus = Focus::Browser;
                Command::Open(self.url(&url.text))
            }
            "Click" | "Hover" => {
                let target = tokens.required(head, "a CSS selector", &format!("{base} \"button.save\""))?.text;
                self.focus = Focus::Browser;
                let timeout = speed.unwrap_or(self.tape.settings.wait_timeout);
                if base == "Click" { Command::Click { target, timeout } } else { Command::Hover { target, timeout } }
            }
            "Scroll" => {
                let target = tokens.required(head, "pixels or a selector", "Scroll 400")?;
                self.focus = Focus::Browser;
                match (target.kind, target.text.parse::<i32>()) {
                    (TokenKind::Word, Ok(pixels)) => Command::Scroll(ScrollTarget::Pixels(pixels)),
                    _ => Command::Scroll(ScrollTarget::Element(target.text)),
                }
            }
            "ScrollUp" | "ScrollDown" => {
                let lines = self.count(tokens, base)? as i32;
                Command::Scroll(ScrollTarget::Lines(if base == "ScrollUp" { -lines } else { lines }))
            }
            "Image" => Command::Image(self.path(&tokens.required(head, "a PNG file", "Image \"screen.png\"")?.text)),
            _ => self.press(tokens, head, name, speed)?,
        };

        self.has_steps = true;
        let step = Step { location: line.location.clone(), focus: self.focus, command };
        self.tape.scenes.last_mut().expect("there is always a scene").steps.push(step);

        Ok(())
    }

    fn set(&mut self, tokens: &mut Tokens, head: &Token) -> Result<()> {
        let line = tokens.line;
        let name = tokens.required(head, "a name and a value", "Set FontSize 20")?;
        // A JSON theme is everything from its `{` to the `}` that closes it.
        let json_start = tokens
            .peek()
            .filter(|token| name.text == "Theme" && token.text.starts_with('{'))
            .map(|token| token.span.start);
        let (value, value_span) = match json_start {
            Some(start) => {
                let end = closing_brace(line.text, start)
                    .ok_or_else(|| line.error("this theme is never closed", start..line.text.len()))?;
                tokens.skip_to(end);
                (line.text[start..end].to_string(), start..end)
            }
            _ => {
                let value = tokens
                    .argument()
                    .ok_or_else(|| line.error(format!("Set {} needs a value", name.text), name.span.clone()))?;
                (value.text, value.span)
            }
        };
        if self.has_steps && !STEP_SETTINGS.contains(&name.text.as_str()) {
            return Err(line
                .error(format!("Set {} has to come before the first step", name.text), head.span.start..value_span.end)
                .help(format!("only {} can change between steps; move it to the top", STEP_SETTINGS.join(", "))));
        }

        self.tape.settings.set(&name.text, &value).map_err(|message| {
            let known = settings::NAMES.contains(&name.text.as_str());
            let error = line.error(message, if known { value_span } else { name.span.clone() });
            match suggest(&name.text, settings::NAMES).filter(|_| !known) {
                Some(suggestion) => error.help(format!("did you mean Set {suggestion}?")),
                None => error,
            }
        })?;
        if name.text == "Directory" {
            self.tape.settings.directory = self.path(&value);
        }

        Ok(())
    }

    fn source(&mut self, tokens: &mut Tokens, head: &Token) -> Result<()> {
        let file = tokens.required(head, "a tape", "Source setup.tape")?;
        let path = self.path(&file.text);
        if self.sources.contains(&path) {
            return Err(tokens.line.error("this tape sources itself", file.span));
        }
        let source = std::fs::read_to_string(&path)
            .map_err(|error| tokens.line.error(format!("{}: {error}", path.display()), file.span.clone()))?;
        self.sources.push(path.clone());
        let result = self.read(&source, &path);
        self.sources.pop();

        result
    }

    fn wait(
        &mut self,
        tokens: &mut Tokens,
        head: &Token,
        modifier: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<Command> {
        let line = tokens.line;
        let scope = match modifier {
            None | Some("Screen") => WaitScope::Screen,
            Some("Line") => WaitScope::Line,
            Some(other) => {
                let start = head.span.start + 5;
                return Err(line
                    .error(format!("no such Wait scope: {other}"), start..start + other.len())
                    .help("Wait+Screen or Wait+Line"));
            }
        };
        if scope == WaitScope::Line && self.focus == Focus::Browser {
            return Err(line.error("Wait+Line reads the terminal, and the browser has focus", head.span.clone()));
        }
        let pattern = match tokens.argument() {
            None => self.tape.settings.wait_pattern.clone(),
            Some(token) if token.kind == TokenKind::Pattern => Some(pattern(line, &token)?),
            Some(token) => {
                return Err(line
                    .error("Wait takes a /pattern/", token.span.clone())
                    .help(format!("Wait /{}/", regex_lite::escape(&token.text))));
            }
        };

        Ok(Command::Wait { pattern, scope, timeout: timeout.unwrap_or(self.tape.settings.wait_timeout) })
    }

    fn press(&mut self, tokens: &mut Tokens, head: &Token, name: &str, delay: Option<Duration>) -> Result<Command> {
        let line = tokens.line;
        let Some(key) = Key::parse(name).filter(|_| is_key(name)) else {
            let error = line.error(format!("no such command: {name}"), head.span.start..head.span.start + name.len());
            return Err(match suggest(name, COMMANDS) {
                _ if name.chars().count() == 1 => error.help(format!("to type it: Type \"{name}\"")),
                Some(suggestion) => error.help(format!("did you mean {suggestion}?")),
                None => error,
            });
        };
        let count = self.count(tokens, &key.to_string())?;
        let delay = delay.unwrap_or(match key.code {
            KeyCode::Char(_) => self.tape.settings.typing_speed,
            _ => self.tape.settings.typing_speed.max(Duration::from_millis(100)),
        });

        Ok(Command::Press { key, count, delay })
    }

    /// The optional count after a key or a scroll: `Enter 3`, `ScrollUp 5`.
    fn count(&self, tokens: &mut Tokens, name: &str) -> Result<u32> {
        match tokens.peek() {
            Some(token) if token.kind == TokenKind::Word && token.text.chars().all(|c| c.is_ascii_digit()) => {
                let token = tokens.take().expect("peeked");
                token
                    .text
                    .parse::<u32>()
                    .ok()
                    .filter(|count| (1..=10_000).contains(count))
                    .ok_or_else(|| tokens.line.error(format!("{name} takes how many times, 1 to 10000"), token.span))
            }
            _ => Ok(1),
        }
    }

    fn path(&self, text: &str) -> PathBuf {
        let tape = self.sources.last().expect("the tape itself is always there");

        resolve(tape.parent().unwrap_or(Path::new("")), text)
    }

    /// A URL as given, or a file URL for a path, so `Open "index.html"` opens the one beside the tape.
    fn url(&self, text: &str) -> String {
        if text.contains("://") || text.starts_with("about:") || text.starts_with("data:") {
            return text.to_string();
        }
        if text.starts_with("localhost") || text.starts_with("127.0.0.1") {
            return format!("http://{text}");
        }

        file_url(&self.path(text))
    }
}

/// A `file:///` URL for a path: forward slashes, and anything a URL would read otherwise — a
/// space, `#`, `?`, `%` — percent-encoded.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let encoded: String = text
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect();

    format!("file://{}{encoded}", if encoded.starts_with('/') { "" } else { "/" })
}

fn tokenize(line: &Line) -> Result<Vec<Token>> {
    let text = line.text;
    let bytes = text.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let start = at;
        let after_wait = tokens.last().is_some_and(|token| token.text.starts_with("Wait"));
        let token = match bytes[at] {
            // A comment runs to the end of the line; a color like #ff8800 is not one.
            b'#' if !is_hex_color(&text[at..]) => break,
            quote @ (b'"' | b'\'' | b'`') => {
                let Some(length) = text[at + 1..].find(quote as char) else {
                    return Err(line
                        .error("this string is never closed", start..text.len())
                        .help(format!("end it with {}", quote as char)));
                };
                at += length + 2;
                Token { text: text[start + 1..at - 1].to_string(), span: start..at, kind: TokenKind::Quoted }
            }
            b'/' if after_wait || tokens.last().is_some_and(|token| token.text == "WaitPattern") => {
                // The pattern ends at the first unescaped `/` that is followed by flags and a space.
                let mut index = at + 1;
                let end = loop {
                    match bytes.get(index) {
                        None => break None,
                        Some(b'\\') => index += 2,
                        Some(b'/') => {
                            let flags = bytes[index + 1..].iter().take_while(|byte| byte.is_ascii_alphabetic()).count();
                            let next = index + 1 + flags;
                            if next == bytes.len() || bytes[next].is_ascii_whitespace() {
                                break Some(next);
                            }
                            index += 1;
                        }
                        Some(_) => index += 1,
                    }
                };
                let Some(end) = end.filter(|end| *end > start + 2) else {
                    return Err(line.error("this pattern is never closed", start..text.len()).help("Wait /ok 1/"));
                };
                at = end;
                Token { text: text[start..at].to_string(), span: start..at, kind: TokenKind::Pattern }
            }
            _ => {
                // A word ends at a space, or where a string starts: `Type@.2'fast'` is two tokens.
                while at < bytes.len() && !bytes[at].is_ascii_whitespace() && !matches!(bytes[at], b'"' | b'\'' | b'`')
                {
                    at += 1;
                }
                Token { text: text[start..at].to_string(), span: start..at, kind: TokenKind::Word }
            }
        };
        tokens.push(token);
    }

    Ok(tokens)
}

fn is_hex_color(text: &str) -> bool {
    let digits = text[1..].chars().take_while(char::is_ascii_hexdigit).count();
    let end = text[1 + digits..].chars().next();
    matches!(digits, 3 | 6 | 8) && end.is_none_or(|character| character.is_whitespace() || character == '"')
}

/// Whether a word starts a command: a command's name, possibly with `@speed` or `+Scope`, or a key.
fn is_command(word: &str) -> bool {
    let name = word.split_once('@').map_or(word, |(name, _)| name);
    COMMANDS.contains(&name) || name.starts_with("Wait+") || is_key(name)
}

/// Whether a word is a key a tape can press: a named key, or anything with a modifier.
fn is_key(name: &str) -> bool {
    Key::parse(name).is_some() && (Key::is_named(name) || name.contains('+'))
}

/// Where the `}` closing the `{` at `start` is, counting nested braces and skipping strings.
fn closing_brace(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in text[start..].char_indices() {
        match character {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + index + 1);
                }
            }
            _ => {}
        }
    }

    None
}

/// A pattern as `Set WaitPattern` takes it: `/done/`, `/done/i`, or just `done`.
pub(crate) fn pattern_from(value: &str) -> std::result::Result<Pattern, String> {
    match value.strip_prefix('/').and_then(|rest| rest.rsplit_once('/')) {
        Some((body, flags)) => compile(body, flags),
        None => compile(value, ""),
    }
}

/// A `/pattern/flags` token, compiled.
fn pattern(line: &Line, token: &Token) -> Result<Pattern> {
    let close = token.text.rfind('/').expect("a pattern token ends in /");
    compile(&token.text[1..close], &token.text[close + 1..]).map_err(|message| {
        let error = line.error(message.clone(), token.span.clone());
        if message.starts_with("no such pattern flag") { error.help("flags are i, m and s") } else { error }
    })
}

/// A regular expression and its flags: `i` ignores case, `m` makes `^` and `$` match at every
/// line, `s` lets `.` match a newline.
fn compile(body: &str, flags: &str) -> std::result::Result<Pattern, String> {
    let mut builder = regex_lite::RegexBuilder::new(body);
    for flag in flags.chars() {
        match flag {
            'i' => builder.case_insensitive(true),
            'm' => builder.multi_line(true),
            's' => builder.dot_matches_new_line(true),
            _ => return Err(format!("no such pattern flag: {flag}")),
        };
    }

    builder.build().map(Pattern).map_err(|error| {
        let message = error.to_string();
        format!("not a pattern: {}", message.lines().last().unwrap_or(&message))
    })
}

fn word<'a>(line: &Line, token: &'a Token, command: &str) -> Result<&'a str> {
    let valid = !token.text.is_empty()
        && token.text.chars().all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '.'));
    if valid {
        Ok(&token.text)
    } else {
        Err(line
            .error(format!("{command} takes a name of letters, digits, - and _"), token.span.clone())
            .help(format!("{command} {}", token.text.replace(|c: char| !c.is_alphanumeric(), "-"))))
    }
}

/// The known name closest to a misspelt one, if any is close enough to be what was meant.
fn suggest<'a>(wrong: &str, known: &[&'a str]) -> Option<&'a str> {
    let wrong = wrong.to_lowercase();
    known
        .iter()
        .map(|name| (distance(&wrong, &name.to_lowercase()), *name))
        .filter(|(distance, name)| *distance <= 2.max(name.len() / 4))
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, name)| name)
}

/// Levenshtein distance.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, a) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + (a != *b) as usize).min(row[j] + 1).min(above + 1);
            diagonal = above;
        }
    }

    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Tape {
        Tape::parse_at(source, Path::new("/demo/demo.tape")).unwrap_or_else(|error| panic!("{error}"))
    }

    fn error(source: &str) -> String {
        Tape::parse_at(source, Path::new("/demo/demo.tape")).unwrap_err().to_string()
    }

    fn commands(source: &str) -> Vec<Command> {
        parse(source).steps().map(|step| step.command.clone()).collect()
    }

    #[test]
    fn a_vhs_tape_reads_as_it_does_in_vhs() {
        let tape = parse(
            r#"
            # A comment, then blank lines.

            Output demo.gif
            Require echo
            Set FontSize 22
            Set Theme "Catppuccin Mocha"
            Env GREETING "hi"
            Type "echo $GREETING"
            Enter
            Sleep 1s
            Ctrl+L
            Backspace@10ms 3
            Hide
            Show
            Screenshot shot.png
            "#,
        );

        assert_eq!(tape.outputs, [PathBuf::from("/demo/demo.gif")]);
        assert_eq!(tape.requires, ["echo"]);
        assert_eq!(tape.env, [("GREETING".to_string(), "hi".to_string())]);
        assert_eq!(tape.settings.font_size, 22.0);
        assert_eq!(tape.settings.theme.name, "Catppuccin Mocha");
        assert_eq!(tape.scenes.len(), 1);
        assert_eq!(tape.scenes[0].caption, None);
        let commands: Vec<_> = tape.steps().map(|step| &step.command).collect();
        assert!(matches!(commands[0], Command::Type { text, .. } if text == "echo $GREETING"));
        assert!(matches!(commands[1], Command::Press { key, count: 1, .. } if key.code == KeyCode::Enter));
        assert_eq!(*commands[2], Command::Sleep(Duration::from_secs(1)));
        assert!(matches!(commands[3], Command::Press { key, .. } if key.ctrl && key.code == KeyCode::Char('l')));
        assert!(matches!(commands[4], Command::Press { count: 3, delay, .. } if *delay == Duration::from_millis(10)));
        assert_eq!(commands[5..7], [&Command::Hide, &Command::Show]);
        assert_eq!(*commands[7], Command::Screenshot("/demo/shot.png".into()));
    }

    #[test]
    fn captions_start_scenes() {
        let tape = parse("Caption \"One\" \"first\"\nType \"a\"\nCaption \"Two\"\nType \"b\"");

        assert_eq!(tape.scenes.len(), 2);
        assert_eq!(tape.scenes[0].caption, Some(Caption { title: "One".into(), detail: "first".into() }));
        assert_eq!(tape.scenes[1].caption.as_ref().unwrap().detail, "");
        assert!(tape.has_captions());
    }

    #[test]
    fn steps_before_the_first_caption_are_a_scene_of_their_own() {
        let tape = parse("Hide\nType \"setup\"\nShow\nCaption \"One\"\nType \"a\"");

        assert_eq!(tape.scenes.len(), 2);
        assert_eq!(tape.scenes[0].caption, None);
        assert_eq!(tape.scenes[0].steps.len(), 3);
    }

    #[test]
    fn typing_speed_can_change_between_steps() {
        let delays: Vec<_> = commands("Type \"a\"\nSet TypingSpeed 10ms\nType \"b\"\nType@1s \"c\"")
            .into_iter()
            .map(|command| match command {
                Command::Type { delay, .. } => delay.as_millis(),
                _ => unreachable!(),
            })
            .collect();

        assert_eq!(delays, [50, 10, 1000]);
    }

    #[test]
    fn other_settings_cannot() {
        let message = error("Type \"a\"\nSet Theme \"Nord\"");
        assert!(message.contains("Set Theme has to come before the first step"), "{message}");
        assert!(message.contains("move it to the top"), "{message}");
    }

    #[test]
    fn a_json_theme_is_the_rest_of_the_line() {
        let tape = parse(r##"Set Theme { "background": "#000000", "foreground": "#ffffff" }"##);
        assert_eq!(tape.settings.theme.background, crate::Rgb(0, 0, 0));
    }

    #[test]
    fn waits() {
        let waits = commands("Wait\nWait /ok 1/\nWait+Line@3s /done$/i\nWait+Screen /x/");

        assert_eq!(
            waits[0],
            Command::Wait { pattern: None, scope: WaitScope::Screen, timeout: Duration::from_secs(15) }
        );
        let Command::Wait { pattern: Some(pattern), scope, timeout } = &waits[2] else { unreachable!() };
        assert!(pattern.matches("all DONE"));
        assert_eq!((*scope, *timeout), (WaitScope::Line, Duration::from_secs(3)));
        assert!(matches!(&waits[3], Command::Wait { scope: WaitScope::Screen, .. }));
    }

    #[test]
    fn patterns_can_hold_slashes_and_spaces() {
        let Command::Wait { pattern: Some(pattern), .. } = &commands(r"Wait /a\/b c/")[0] else { unreachable!() };
        assert!(pattern.matches("a/b c"));
        assert_eq!(pattern.to_string(), r"/a\/b c/");
    }

    #[test]
    fn browser_commands_move_the_focus_to_the_browser() {
        let tape = parse(
            "Type \"npm start\"\nOpen \"localhost:3000\"\nClick \"text=Sign in\"\nType \"me\"\nFocus terminal\nCtrl+C",
        );
        let views: Vec<_> = tape.steps().map(|step| step.focus).collect();

        assert_eq!(views, [Focus::Terminal, Focus::Browser, Focus::Browser, Focus::Browser, Focus::Terminal]);
        assert!(matches!(&tape.scenes[0].steps[1].command, Command::Open(url) if url == "http://localhost:3000"));
        assert!(tape.uses_terminal() && tape.uses_pane());
    }

    #[test]
    fn a_browser_only_tape_has_no_terminal() {
        let tape = parse("Open \"https://example.com\"\nScroll 300\nScroll \"#faq\"\nHover@2s \"a\"");

        assert!(!tape.uses_terminal());
        let commands: Vec<_> = tape.steps().map(|step| step.command.clone()).collect();
        assert_eq!(commands[1], Command::Scroll(ScrollTarget::Pixels(300)));
        assert_eq!(commands[2], Command::Scroll(ScrollTarget::Element("#faq".into())));
        assert_eq!(commands[3], Command::Hover { target: "a".into(), timeout: Duration::from_secs(2) });
    }

    #[test]
    fn open_turns_a_path_into_a_file_url() {
        assert_eq!(commands("Open \"site/index.html\"")[0], Command::Open("file:///demo/site/index.html".into()));
        assert_eq!(commands("Open \"about:blank\"")[0], Command::Open("about:blank".into()));
    }

    #[test]
    fn file_urls_are_encoded_and_use_forward_slashes() {
        assert_eq!(file_url(Path::new("/a b/#1?.html")), "file:///a%20b/%231%3F.html");
        assert_eq!(file_url(Path::new(r"C:\docs\site.html")), "file:///C:/docs/site.html");
    }

    #[test]
    fn actions_are_declared_and_done_by_name() {
        let tape = parse("Action break \"sed -i s/3/1/ cart.ts\"\nDo break\nDo fix\nDo break");

        assert_eq!(tape.actions["break"], "sed -i s/3/1/ cart.ts");
        let names: Vec<_> = tape.used_actions().into_iter().map(|(name, location)| (name, location.line)).collect();
        assert_eq!(names, [("break", 2), ("fix", 3)]);
    }

    #[test]
    fn all_three_quotes_work_and_nothing_is_escaped() {
        let texts: Vec<_> = commands(
            r#"Type "it's"
Type 'say "hi"'
Type `C:\new`"#,
        )
        .into_iter()
        .map(|command| match command {
            Command::Type { text, .. } => text,
            _ => unreachable!(),
        })
        .collect();

        assert_eq!(texts, ["it's", "say \"hi\"", r"C:\new"]);
    }

    #[test]
    fn source_reads_another_tape_in_place() {
        let directory = std::env::temp_dir().join(format!("demogod-source-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("setup.tape"), "Hide\nType \"cd /tmp\"\nShow").unwrap();
        std::fs::write(directory.join("loop.tape"), "Source loop.tape").unwrap();

        let tape = Tape::parse_at("Source setup.tape\nType \"ls\"", &directory.join("demo.tape")).unwrap();
        assert_eq!(tape.steps().count(), 4);
        assert_eq!(tape.scenes[0].steps[1].location.file.as_ref(), directory.join("setup.tape"));

        let looping = Tape::parse_at("Source loop.tape", &directory.join("demo.tape")).unwrap_err();
        assert!(looping.message.contains("sources itself"), "{looping}");
        let missing = Tape::parse_at("Source nope.tape", &directory.join("demo.tape")).unwrap_err();
        assert!(missing.location.is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn misspelt_commands_get_a_suggestion() {
        let message = error("Typ \"ls\"");
        assert!(message.contains("no such command: Typ"), "{message}");
        assert!(message.contains("did you mean Type?"), "{message}");
        assert!(error("Entr").contains("did you mean Enter?"));
        assert!(error("A").contains("Type \"A\""));
        assert!(error("Set FontSise 20").contains("did you mean Set FontSize?"));
    }

    #[test]
    fn errors_point_at_the_line_and_the_part_of_it() {
        assert_eq!(
            error("Type \"a\"\n\nSleep 5x"),
            "not a duration: 5x\n --> /demo/demo.tape:3\n  |\n3 | Sleep 5x\n  |       ^^\n  = help: durations are written 500ms, 2s or 1.5s"
        );
    }

    #[test]
    fn every_malformed_line_is_an_error_not_a_guess() {
        let cases = [
            ("Type", "Type takes a string"),
            ("Type \"a\" \"b\"", "expected a command, not a string"),
            ("Type \"never closed", "never closed"),
            ("Wait /never closed", "never closed"),
            ("Wait ok", "Wait takes a /pattern/"),
            ("Wait /a/ extra", "no such command: extra"),
            ("Wait /(/", "not a pattern"),
            ("Wait /a/x", "no such pattern flag: x"),
            ("Wait+Word /a/", "no such Wait scope: Word"),
            ("Open \"x\"\nWait+Line /a/", "the browser has focus"),
            ("Enter 0", "how many times"),
            ("Set Theme { \"background\": \"#000\"", "never closed"),
            ("Set FontSize", "needs a value"),
            ("Enter 1 2", "no such command: 2"),
            ("Hide now", "no such command: now"),
            ("Do \"two words\"", "a name of letters"),
            ("Focus mouse", "terminal or browser"),
            ("Caption", "Caption takes a title"),
            ("Env A", "Env takes a name and a value"),
            ("Set", "Set takes a name"),
            ("Set Width", "needs a value"),
            ("Type@fast \"a\"", "not a duration"),
            ("Output", "Output takes a file"),
            ("Sleep@1s 2s", "Sleep takes no @"),
            ("Set WaitPattern /a/x", "no such pattern flag: x"),
        ];
        for (source, expected) in cases {
            let message = error(source);
            assert!(message.contains(expected), "{source:?} gave:\n{message}");
        }
    }

    #[test]
    fn a_line_can_hold_several_commands_the_way_vhs_allows() {
        let commands = commands("Type \"ls\" Sleep 500ms Enter\nSleep .3 Tab Sleep .3\nCtrl+C Wait /done/ Enter 2");

        assert_eq!(commands.len(), 9);
        assert!(matches!(&commands[0], Command::Type { text, .. } if text == "ls"));
        assert_eq!(commands[1], Command::Sleep(Duration::from_millis(500)));
        assert!(matches!(&commands[2], Command::Press { key, .. } if key.code == KeyCode::Enter));
        assert_eq!(commands[3], Command::Sleep(Duration::from_millis(300)));
        assert!(matches!(&commands[6], Command::Press { key, .. } if key.ctrl));
        assert!(matches!(&commands[7], Command::Wait { pattern: Some(_), .. }));
        assert!(matches!(&commands[8], Command::Press { count: 2, .. }));
    }

    #[test]
    fn a_comment_can_end_a_line_and_a_color_is_not_one() {
        let tape = parse("Set MarginFill #6B50FF # purple\nSet LoopOffset 50% # % is optional\nSpace # exit");

        assert_eq!(tape.settings.margin_fill, crate::Rgb(0x6b, 0x50, 0xff));
        assert_eq!(tape.steps().count(), 1);
    }

    #[test]
    fn a_speed_can_touch_its_string() {
        let Command::Type { text, delay } = &commands("Type@.2'[ .products[] ]'")[0] else { unreachable!() };
        assert_eq!((text.as_str(), *delay), ("[ .products[] ]", Duration::from_millis(200)));
    }

    #[test]
    fn a_bare_word_types_itself_and_scrolling_counts_lines() {
        let commands = commands("Type o\nScrollUp\nScrollDown 3");
        assert!(matches!(&commands[0], Command::Type { text, .. } if text == "o"));
        assert_eq!(commands[1], Command::Scroll(ScrollTarget::Lines(-1)));
        assert_eq!(commands[2], Command::Scroll(ScrollTarget::Lines(3)));
    }

    #[test]
    fn a_pattern_ends_at_its_own_slash_not_the_last_on_the_line() {
        let commands = commands("Wait /http:\\/\\/x/ Type \"a/b\"");
        let Command::Wait { pattern: Some(pattern), .. } = &commands[0] else { unreachable!() };
        assert!(pattern.matches("http://x"));
        assert!(matches!(&commands[1], Command::Type { text, .. } if text == "a/b"));
        assert_eq!(parse("Output /tmp/demo.gif").outputs, [PathBuf::from("/tmp/demo.gif")]);
    }

    #[test]
    fn a_json_theme_can_be_followed_by_more() {
        let tape = parse(r##"Set Theme { "background": "#101010", "name": "a } b" } # dark"##);
        assert_eq!(tape.settings.theme.background, crate::Rgb(16, 16, 16));
        assert_eq!(tape.settings.theme.name, "a } b");
    }

    #[test]
    fn distance_counts_edits() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(suggest("Slep", COMMANDS), Some("Sleep"));
        assert_eq!(suggest("Xyzzy", COMMANDS), None);
    }
}
