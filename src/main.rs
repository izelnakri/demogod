//! `demogod`: records the demos tapes describe.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use demogod::{Demo, Error, Event, Tape, Theme};
use serde_json::json;

const HELP: &str = "\
demogod — records terminal and browser demos from a tape

Usage:
  demogod <tape>... [options]   record each tape, and save it to its Output files
  demogod new [file]            write a starter tape (demo.tape if no file is given)
  demogod check <tape>...       read each tape and check it can be recorded, without recording
  demogod themes                list the built-in themes

Options:
  -o, --output <file>   save here instead of the tape's Output; repeatable:
                        .gif .mp4 .webm, .png (the last frame), .cast (asciinema),
                        .txt (the terminal's screens as text) or dir/ (PNG frames)
  -w, --watch           record again each time a tape changes
  -q, --quiet           print nothing but errors
      --json            print progress as JSON lines on stdout, for other programs
      --action <name>   with --json: `Do <name>` asks the caller to run it (see the README)
  -h, --help            print this
  -V, --version         print the version

A tape is a file of commands — VHS's, plus Caption, Open, Click, Hover, Scroll, Focus,
Image, Action and Do. `demogod new` writes one to start from.

https://github.com/izelnakri/demogod";

const STARTER_TAPE: &str = r#"# A demogod tape. Record it with: demogod demo.tape
#
# One command per line. VHS tapes work as they are, and demogod adds Caption, Open, Click,
# Hover, Scroll, Focus, Image, Action and Do: https://github.com/izelnakri/demogod

Output demo.gif

Set Width 1200
Set Height 600
Set FontSize 18
Set Theme "Catppuccin Mocha"

Caption "Say hello" "typed at a human pace, into a real shell"
Type "echo 'Hello from demogod'"
Sleep 300ms
Enter
Wait
Sleep 1.5s

Caption "Look around" "a bare Wait lasts until the command is done"
Type "ls -la"
Enter
Wait
Sleep 2.5s
"#;

/// What was asked for on the command line.
#[derive(Debug, Default, PartialEq)]
struct Arguments {
    command: Option<String>,
    tapes: Vec<PathBuf>,
    outputs: Vec<PathBuf>,
    actions: Vec<String>,
    watch: bool,
    quiet: bool,
    json: bool,
}

fn main() -> ExitCode {
    let arguments = match parse(std::env::args_os().skip(1)) {
        Ok(Some(arguments)) => arguments,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}\n\nRun `demogod --help` for usage.");
            return ExitCode::from(2);
        }
    };
    let output = Output::new(arguments.quiet, arguments.json);

    let result = match arguments.command.as_deref() {
        Some("new") => new_tape(arguments.tapes.first(), &output),
        Some("check") => check(&arguments, &output),
        Some("themes") => {
            Theme::names().for_each(|name| println!("{name}"));
            Ok(())
        }
        _ if arguments.watch => watch(&arguments, &output),
        _ => arguments.tapes.iter().try_for_each(|tape| record(tape, &arguments, &output)),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output.error(&error);
            ExitCode::FAILURE
        }
    }
}

/// Reads the command line, or prints help or the version and returns `None`.
fn parse(argv: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Option<Arguments>, String> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_args(argv);
    let mut arguments = Arguments::default();
    let describe = |error: lexopt::Error| error.to_string();
    while let Some(argument) = parser.next().map_err(describe)? {
        match argument {
            Short('h') | Long("help") => {
                println!("{HELP}");
                return Ok(None);
            }
            Short('V') | Long("version") => {
                println!("demogod {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Short('o') | Long("output") => arguments.outputs.push(parser.value().map_err(describe)?.into()),
            Short('w') | Long("watch") => arguments.watch = true,
            Short('q') | Long("quiet") => arguments.quiet = true,
            Long("json") => arguments.json = true,
            Long("action") => arguments.actions.push(parser.value().map_err(describe)?.string().map_err(describe)?),
            Value(value) => {
                let value = value.string().map_err(describe)?;
                let is_command = matches!(value.as_str(), "new" | "check" | "themes");
                if arguments.command.is_none() && arguments.tapes.is_empty() && is_command {
                    arguments.command = Some(value);
                } else {
                    arguments.tapes.push(value.into());
                }
            }
            _ => return Err(argument.unexpected().to_string()),
        }
    }

    match arguments.command.as_deref() {
        Some("themes") | Some("new") => {}
        _ if arguments.tapes.is_empty() => return Err("no tape given".into()),
        _ => {}
    }
    if !arguments.actions.is_empty() && !arguments.json {
        return Err("--action needs --json: the caller answers on stdin".into());
    }
    if !arguments.outputs.is_empty() && arguments.tapes.len() > 1 && arguments.command.is_none() {
        return Err("-o names one film, and there are several tapes: record them one at a time".into());
    }
    if arguments.watch && arguments.json {
        return Err("--watch and --json do not go together".into());
    }

    Ok(Some(arguments))
}

/// Reads a tape, from stdin when it is `-`: all of stdin, or with `--json` its first line, as
/// `{"tape": "<source>"}`, since the lines after it answer actions.
fn read_tape(path: &Path, json: bool) -> demogod::Result<Tape> {
    if path != Path::new("-") {
        return Tape::from_file(path);
    }
    let mut source = String::new();
    if json {
        std::io::stdin().lock().read_line(&mut source)?;
        let message: serde_json::Value = serde_json::from_str(&source)
            .map_err(|_| Error::new("with --json, a tape on stdin comes as one line: {\"tape\": \"<source>\"}"))?;
        source = message["tape"].as_str().unwrap_or_default().to_string();
    } else {
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut source)?;
    }

    Tape::parse(&source)
}

fn demo_for(path: &Path, arguments: &Arguments) -> demogod::Result<Demo> {
    let mut demo = Demo::new(read_tape(path, arguments.json)?).outputs(arguments.outputs.iter().cloned());
    for name in &arguments.actions {
        let action = name.clone();
        demo = demo.action(name.clone(), move || ask_caller(&action));
    }

    Ok(demo)
}

fn record(path: &Path, arguments: &Arguments, output: &Output) -> demogod::Result<()> {
    let started = Instant::now();
    let demo = demo_for(path, arguments)?;
    output.start(path, demo.tape());
    let reporter = output.clone();
    demo.on_event(move |event| reporter.event(event)).run()?;
    output.done(started.elapsed());

    Ok(())
}

fn check(arguments: &Arguments, output: &Output) -> demogod::Result<()> {
    for path in &arguments.tapes {
        let demo = demo_for(path, arguments)?;
        demo.check()?;
        let tape = demo.tape();
        let steps = tape.steps().count();
        output.say(&format!("{} {}  {} scenes, {steps} steps", output.tick(), path.display(), tape.scenes.len()));
        if output.json {
            output.line(json!({ "event": "checked", "path": path, "scenes": tape.scenes.len(), "steps": steps }));
        }
    }

    Ok(())
}

fn new_tape(path: Option<&PathBuf>, output: &Output) -> demogod::Result<()> {
    let path = path.cloned().unwrap_or_else(|| PathBuf::from("demo.tape"));
    if path.exists() {
        return Err(
            Error::new(format!("{} already exists", path.display())).help("give another name: demogod new intro.tape")
        );
    }
    std::fs::write(&path, STARTER_TAPE).map_err(|error| Error::io(&path, error))?;
    output.say(&format!("{} wrote {} — record it with: demogod {}", output.tick(), path.display(), path.display()));

    Ok(())
}

/// Records every tape, then again whenever one changes, until interrupted.
fn watch(arguments: &Arguments, output: &Output) -> demogod::Result<()> {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok();
    let mut seen: Vec<Option<SystemTime>> = arguments.tapes.iter().map(|path| modified(path)).collect();
    for path in &arguments.tapes {
        if let Err(error) = record(path, arguments, output) {
            output.error(&error);
        }
    }
    output.say("watching for changes; Ctrl+C to stop");
    loop {
        std::thread::sleep(Duration::from_millis(250));
        for (index, path) in arguments.tapes.iter().enumerate() {
            let now = modified(path);
            if now != seen[index] {
                seen[index] = now;
                if let Err(error) = record(path, arguments, output) {
                    output.error(&error);
                }
            }
        }
    }
}

/// A path as short as it can be written from here.
fn relative(path: &Path) -> &Path {
    std::env::current_dir().ok().and_then(|here| path.strip_prefix(here).ok()).unwrap_or(path)
}

/// Asks whoever started demogod with `--json --action <name>` to run an action, and waits for
/// the answer: `{"ok": true}` or `{"error": "why"}`, one line on stdin.
fn ask_caller(name: &str) -> Result<(), String> {
    let request = json!({ "event": "action-request", "name": name });
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{request}").and_then(|()| stdout.flush()).map_err(|error| error.to_string())?;
    drop(stdout);

    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).map_err(|error| error.to_string())?;
    let answer: serde_json::Value =
        serde_json::from_str(&answer).map_err(|_| format!("the caller answered {answer:?}, not JSON"))?;
    match answer.get("error").and_then(|error| error.as_str()) {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

/// Where progress goes: nowhere, stderr for people, or stdout as JSON lines for programs.
#[derive(Clone)]
struct Output {
    quiet: bool,
    json: bool,
    color: bool,
}

impl Output {
    fn new(quiet: bool, json: bool) -> Output {
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Output { quiet, json, color }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    fn tick(&self) -> String {
        self.paint("32", "✓")
    }

    /// A line for people, on stderr.
    fn say(&self, text: &str) {
        if !self.quiet && !self.json {
            eprintln!("{text}");
        }
    }

    /// A line for programs, on stdout.
    fn line(&self, value: serde_json::Value) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{value}").and_then(|()| stdout.flush());
    }

    fn start(&self, path: &Path, tape: &Tape) {
        let scenes = tape.caption_numbers().into_iter().flatten().count().max(1);
        let plural = if scenes == 1 { "" } else { "s" };
        self.say(&format!("{} {}  {scenes} scene{plural}", self.paint("35", "●"), path.display()));
    }

    fn event(&self, event: &Event) {
        if self.json {
            let value = match event {
                Event::Scene { number, total, caption } => json!({
                    "event": "scene", "number": number, "total": total,
                    "title": caption.as_ref().map(|caption| &caption.title),
                    "detail": caption.as_ref().map(|caption| &caption.detail),
                }),
                Event::Action { name } => json!({ "event": "action", "name": name }),
                Event::Warning { message } => json!({ "event": "warning", "message": message }),
                Event::Recorded { duration } => json!({ "event": "recorded", "duration": duration.as_secs_f64() }),
                Event::Saved(saved) => json!({
                    "event": "saved", "path": saved.path, "bytes": saved.bytes,
                    "frames": saved.frames, "duration": saved.duration.as_secs_f64(),
                }),
                _ => return,
            };
            return self.line(value);
        }
        match event {
            Event::Warning { message } => eprintln!("{} {message}", self.paint("33", "warning:")),
            Event::Scene { number: Some(number), total, caption: Some(caption) } => {
                let counter = self.paint("2", &format!("{number}/{total}"));
                self.say(&format!("  {counter} {}", caption.title));
            }
            Event::Saved(saved) => {
                let size = if saved.bytes >= 1024 * 1024 {
                    format!("{:.1} MB", saved.bytes as f64 / (1024.0 * 1024.0))
                } else {
                    format!("{} KB", saved.bytes.div_ceil(1024))
                };
                let frames = if saved.frames > 0 { format!(" · {} frames", saved.frames) } else { String::new() };
                let details = self.paint("2", &format!("{size} · {:.1}s{frames}", saved.duration.as_secs_f32()));
                self.say(&format!("{} {}  {details}", self.tick(), relative(&saved.path).display()));
            }
            _ => {}
        }
    }

    fn done(&self, took: Duration) {
        self.say(&self.paint("2", &format!("  done in {:.1}s", took.as_secs_f32())));
    }

    fn error(&self, error: &Error) {
        if self.json {
            let location = error
                .location
                .as_ref()
                .map(|location| json!({ "file": location.file.display().to_string(), "line": location.line }));
            self.line(json!({ "event": "error", "message": error.message, "location": location, "help": error.help, "text": error.to_string() }));
        }
        eprintln!("{} {error}", self.paint("1;31", "error:"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(line: &str) -> Result<Option<Arguments>, String> {
        parse(line.split_whitespace().map(Into::into))
    }

    #[test]
    fn tapes_and_options() {
        let parsed = arguments("demo.tape -o a.gif --output b.mp4 -q").unwrap().unwrap();
        assert_eq!(parsed.tapes, [PathBuf::from("demo.tape")]);
        assert_eq!(parsed.outputs, [PathBuf::from("a.gif"), PathBuf::from("b.mp4")]);
        assert!(parsed.quiet && !parsed.watch && parsed.command.is_none());
    }

    #[test]
    fn subcommands() {
        assert_eq!(arguments("new").unwrap().unwrap().command.as_deref(), Some("new"));
        assert_eq!(arguments("new intro.tape").unwrap().unwrap().tapes, [PathBuf::from("intro.tape")]);
        assert_eq!(arguments("check a.tape").unwrap().unwrap().command.as_deref(), Some("check"));
        assert_eq!(arguments("themes").unwrap().unwrap().command.as_deref(), Some("themes"));
        assert_eq!(arguments("a.tape check").unwrap().unwrap().tapes.len(), 2, "only the first word can be a command");
    }

    #[test]
    fn mistakes_are_usage_errors() {
        assert_eq!(arguments("").unwrap_err(), "no tape given");
        assert_eq!(arguments("check").unwrap_err(), "no tape given");
        assert!(arguments("demo.tape --nope").unwrap_err().contains("--nope"));
        assert!(arguments("demo.tape -o").unwrap_err().contains("missing argument"));
        assert!(arguments("demo.tape --action x").unwrap_err().contains("needs --json"));
        assert!(arguments("demo.tape --watch --json").unwrap_err().contains("do not go together"));
        assert!(arguments("demo.tape --json --action x").unwrap().unwrap().json);
        assert!(arguments("a.tape b.tape -o x.gif").unwrap_err().contains("one at a time"));
    }

    #[test]
    fn the_starter_tape_parses() {
        let tape = Tape::parse(STARTER_TAPE).unwrap();
        assert_eq!(tape.scenes.len(), 2);
        assert!(tape.uses_terminal());
    }
}
