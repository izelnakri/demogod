//! The `demogod` command, run as a person or a program would run it.
// Recording needs a POSIX shell, so what only recording tests use is unused elsewhere.
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn demogod(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_demogod"))
        .args(arguments)
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A directory of its own for one test, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!("demogod-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }

    fn write(&self, name: &str, contents: &str) -> &Scratch {
        std::fs::write(self.0.join(name), contents).unwrap();
        self
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SMALL: &str = "Set Width 400\nSet Height 200\nSet FontSize 12\nSet Padding 8\n";

#[test]
fn version_and_help() {
    let scratch = Scratch::new("help");
    let version = demogod(&scratch.0, &["--version"]);
    assert!(version.status.success());
    assert_eq!(text(&version.stdout).trim(), format!("demogod {}", env!("CARGO_PKG_VERSION")));

    let help = demogod(&scratch.0, &["-h"]);
    assert!(text(&help.stdout).contains("Usage:"));
}

#[test]
fn usage_mistakes_exit_2_and_point_at_help() {
    let scratch = Scratch::new("usage");
    for arguments in [&[][..], &["demo.tape", "--nope"], &["check"]] {
        let output = demogod(&scratch.0, arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(text(&output.stderr).contains("demogod --help"), "{arguments:?}");
    }
}

#[test]
fn themes_are_listed_one_per_line() {
    let scratch = Scratch::new("themes");
    let output = demogod(&scratch.0, &["themes"]);
    let names = text(&output.stdout);
    assert!(names.lines().any(|name| name == "Catppuccin Mocha"), "{names}");
}

#[test]
fn new_writes_a_tape_that_checks_and_never_overwrites() {
    let scratch = Scratch::new("new");
    let first = demogod(&scratch.0, &["new"]);
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert!(scratch.0.join("demo.tape").exists());

    let again = demogod(&scratch.0, &["new"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(text(&again.stderr).contains("demo.tape already exists"));

    let named = demogod(&scratch.0, &["new", "intro.tape"]);
    assert!(named.status.success());
    let check = demogod(&scratch.0, &["check", "demo.tape", "intro.tape"]);
    assert!(check.status.success(), "{}", text(&check.stderr));
    assert_eq!(text(&check.stderr).matches("2 scenes").count(), 2);
}

#[test]
fn an_error_in_a_tape_exits_1_with_its_line() {
    let scratch = Scratch::new("error");
    scratch.write("broken.tape", "Type \"a\"\nSlep 1s\n");

    let output = demogod(&scratch.0, &["broken.tape"]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("error: no such command: Slep"), "{stderr}");
    assert!(stderr.contains("--> broken.tape:2"), "{stderr}");
    assert!(stderr.contains("did you mean Sleep?"), "{stderr}");
}

#[test]
fn json_reports_errors_as_data() {
    let scratch = Scratch::new("json-error");
    scratch.write("broken.tape", "Slep 1s\n");

    let output = demogod(&scratch.0, &["broken.tape", "--json"]);
    let error: serde_json::Value = serde_json::from_str(text(&output.stdout).lines().last().unwrap()).unwrap();

    assert_eq!(error["event"], "error");
    assert_eq!(error["message"], "no such command: Slep");
    assert_eq!(error["location"]["line"], 1);
    assert_eq!(error["help"], "did you mean Sleep?");
}

#[cfg(unix)]
#[test]
fn records_to_the_outputs_it_is_given() {
    let scratch = Scratch::new("record");
    scratch.write("demo.tape", &format!("{SMALL}Output ignored.gif\nCaption \"Hi\"\nType \"echo hi\"\nEnter\nWait\n"));

    let output = demogod(&scratch.0, &["demo.tape", "-o", "a.gif", "--output", "b.cast"]);

    let stderr = text(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("1/1 Hi"), "{stderr}");
    assert!(stderr.contains("a.gif") && stderr.contains("b.cast"), "{stderr}");
    assert!(scratch.0.join("a.gif").exists() && scratch.0.join("b.cast").exists());
    assert!(!scratch.0.join("ignored.gif").exists());
}

#[cfg(unix)]
#[test]
fn quiet_prints_nothing() {
    let scratch = Scratch::new("quiet");
    scratch.write("demo.tape", &format!("{SMALL}Output demo.png\nType \"x\"\n"));

    let output = demogod(&scratch.0, &["demo.tape", "-q"]);

    assert!(output.status.success());
    assert!(output.stderr.is_empty() && output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn a_tape_can_come_from_stdin() {
    let scratch = Scratch::new("stdin");
    let mut child = Command::new(env!("CARGO_BIN_EXE_demogod"))
        .args(["-", "-o", "piped.png"])
        .current_dir(&scratch.0)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(format!("{SMALL}Type \"piped\"\n").as_bytes()).unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(scratch.0.join("piped.png").exists());
}

#[cfg(unix)]
#[test]
fn json_asks_the_caller_to_run_actions_and_reports_progress() {
    let scratch = Scratch::new("json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_demogod"))
        .args(["-", "--json", "--action", "greet", "-o", "out.cast"])
        .current_dir(&scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let tape =
        format!("{SMALL}Caption \"One\"\nDo greet\nType \"cat greeting\"\nEnter\nWait /hello from the caller/\n");
    writeln!(stdin, "{}", serde_json::json!({ "tape": tape })).unwrap();

    let mut events = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let event: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        if event["event"] == "action-request" {
            assert_eq!(event["name"], "greet");
            std::fs::write(scratch.0.join("greeting"), "hello from the caller\n").unwrap();
            writeln!(stdin, "{}", serde_json::json!({ "ok": true })).unwrap();
        }
        events.push(event["event"].as_str().unwrap().to_string());
    }

    assert!(child.wait().unwrap().success());
    assert_eq!(events, ["scene", "action-request", "action", "recorded", "saved"]);
}

#[cfg(unix)]
#[test]
fn json_passes_on_an_action_the_caller_says_failed() {
    let scratch = Scratch::new("json-fail");
    let mut child = Command::new(env!("CARGO_BIN_EXE_demogod"))
        .args(["-", "--json", "--action", "boom", "-o", "out.png"])
        .current_dir(&scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{}", serde_json::json!({ "tape": format!("{SMALL}Type \"x\"\nDo boom\n") })).unwrap();

    let mut last = serde_json::Value::Null;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        last = serde_json::from_str(&line.unwrap()).unwrap();
        if last["event"] == "action-request" {
            writeln!(stdin, "{}", serde_json::json!({ "error": "no can do" })).unwrap();
        }
    }

    assert_eq!(child.wait().unwrap().code(), Some(1));
    assert_eq!(last["message"], "Do boom failed: no can do");
    assert_eq!(last["location"]["line"], 6);
}

#[test]
fn check_with_json_counts_scenes_and_steps() {
    let scratch = Scratch::new("check-json");
    scratch.write("demo.tape", "Caption \"One\"\nType \"a\"\nEnter\n");

    let output = demogod(&scratch.0, &["check", "demo.tape", "--json"]);
    let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(checked["event"], "checked");
    assert_eq!((checked["scenes"].as_u64(), checked["steps"].as_u64()), (Some(1), Some(2)));
}
