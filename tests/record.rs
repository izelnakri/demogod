//! Recording real tapes, in a real shell and a real browser, end to end.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demogod::{Demo, Event};

/// A directory of its own for one test, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!("demogod-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }

    fn tape(&self, source: &str) -> PathBuf {
        let path = self.0.join("demo.tape");
        std::fs::write(&path, source).unwrap();
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A GIF frame's delay in hundredths of a second, and its rectangle: left, top, width, height.
type GifFrame = (u16, (u16, u16, u16, u16));

/// Every frame of a GIF.
fn gif_frames(path: &Path) -> Vec<GifFrame> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(std::fs::File::open(path).unwrap()).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        frames.push((frame.delay, (frame.left, frame.top, frame.width, frame.height)));
    }
    frames
}

fn total_centiseconds(frames: &[GifFrame]) -> u32 {
    frames.iter().map(|(delay, _)| *delay as u32).sum()
}

const SMALL: &str = "Set Width 480\nSet Height 240\nSet FontSize 12\nSet Padding 8\n";

#[test]
fn a_shell_session_is_saved_in_every_format() {
    let scratch = Scratch::new("formats");
    let tape = scratch.tape(&format!(
        "{SMALL}Output demo.gif\nOutput demo.cast\nOutput last.png\n\
         Type \"echo hello-from-the-shell\"\nEnter\nWait /hello-from-the-shell\\n/\nSleep 300ms\n"
    ));

    let saved = Demo::from_file(&tape).unwrap().run().unwrap();

    assert_eq!(saved.len(), 3);
    let frames = gif_frames(&scratch.path("demo.gif"));
    assert!(frames.len() > 20, "a frame per keystroke at least: {}", frames.len());
    assert_eq!(frames[0].1, (0, 0, 480, 240), "the first frame is the whole picture");
    assert!(frames[5].1.2 < 100, "later frames are only what changed: {:?}", frames[5].1);

    let cast = std::fs::read_to_string(scratch.path("demo.cast")).unwrap();
    assert!(cast.starts_with("{\""), "{cast}");
    assert!(cast.contains("hello-from-the-shell"));

    let png = std::fs::read(scratch.path("last.png")).unwrap();
    assert_eq!(&png[1..4], b"PNG");
}

#[test]
fn tape_timing_makes_the_same_film_every_time() {
    let scratch = Scratch::new("timing");
    // 7 characters at 50ms, Enter at 100ms, a Wait of 1s and a Sleep of 500ms: 1.95s, however
    // long each of them really took.
    let tape = scratch.tape(&format!("{SMALL}Output demo.gif\nType \"echo hi\"\nEnter\nWait\nSleep 500ms\n"));

    let first = Demo::from_file(&tape).unwrap().record().unwrap();
    let second = Demo::from_file(&tape).unwrap().record().unwrap();

    assert_eq!(first.duration(), Duration::from_millis(1950));
    assert_eq!(second.duration(), first.duration());
    first.save(scratch.path("demo.gif")).unwrap();
    assert_eq!(total_centiseconds(&gif_frames(&scratch.path("demo.gif"))), 195);
}

/// When each piece of output appears in an asciicast, in milliseconds.
fn cast_times(path: &Path) -> Vec<u64> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            (event[0].as_f64().unwrap() * 1000.0).round() as u64
        })
        .collect()
}

#[test]
fn each_keystroke_lands_exactly_a_typing_delay_after_the_last() {
    let scratch = Scratch::new("keystrokes");
    let tape = scratch.tape(&format!("{SMALL}Output demo.cast\nType \"echo hello world\"\n"));

    Demo::from_file(&tape).unwrap().run().unwrap();
    let first = cast_times(&scratch.path("demo.cast"));
    Demo::from_file(&tape).unwrap().run().unwrap();
    let second = cast_times(&scratch.path("demo.cast"));

    // The prompt at 0 (in however many pieces the shell wrote it), then one echo every 50ms.
    let typed = |times: Vec<u64>| times.into_iter().filter(|at| *at > 0).collect::<Vec<_>>();
    let expected: Vec<u64> = (1..=16).map(|index| index * 50).collect();
    assert_eq!(typed(first), expected);
    assert_eq!(typed(second), expected);
}

#[test]
fn a_gif_plays_as_long_as_the_film_at_any_speed() {
    let scratch = Scratch::new("speed");
    let tape = scratch.tape(&format!(
        "{SMALL}Set PlaybackSpeed 3\nSet Framerate 100\nOutput demo.gif\nType \"echo fast\"\nEnter\nWait\n"
    ));

    let recording = Demo::from_file(&tape).unwrap().record().unwrap();
    recording.save(scratch.path("demo.gif")).unwrap();

    let centiseconds = total_centiseconds(&gif_frames(&scratch.path("demo.gif")));
    assert_eq!(centiseconds, (recording.duration().as_millis() as u32).div_ceil(10));
}

#[test]
fn a_screenshot_while_hidden_shows_that_moment() {
    let scratch = Scratch::new("hidden-screenshot");
    let tape = scratch.tape(&format!(
        "{SMALL}Output screens.txt\nHide\nType \"echo marker-one\"\nEnter\nWait\nScreenshot hidden.png\nType \"clear\"\nEnter\nWait\nShow\n"
    ));

    Demo::from_file(&tape).unwrap().run().unwrap();

    let screenshot = std::fs::read(scratch.path("hidden.png")).unwrap();
    let cleared = {
        let tape = scratch.tape(&format!("{SMALL}Screenshot cleared.png\nType \"x\"\n"));
        Demo::from_file(&tape).unwrap().record().unwrap();
        std::fs::read(scratch.path("cleared.png")).unwrap()
    };
    assert_ne!(screenshot, cleared, "the screenshot shows marker-one, not the cleared screen");
}

#[test]
fn a_shell_started_as_written_is_waited_on_by_what_it_prints() {
    let scratch = Scratch::new("custom-shell");
    let tape = scratch.tape(&format!(
        "{SMALL}Set Shell \"bash --norc --noprofile\"\nOutput screens.txt\nType \"echo custom-shell\"\nEnter\nWait /^custom-shell$/m\n"
    ));

    let started = std::time::Instant::now();
    Demo::from_file(&tape).unwrap().run().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5), "no ten-second stall: {:?}", started.elapsed());

    let bare = scratch.tape(&format!("{SMALL}Set Shell \"bash --norc\"\nType \"ls\"\nEnter\nWait\n"));
    let error = Demo::from_file(&bare).unwrap().check().unwrap_err();
    assert!(error.message.contains("a bare Wait waits for the prompt"), "{error}");
    assert_eq!(error.location.unwrap().line, 8);
}

#[test]
fn real_timing_takes_as_long_as_it_took() {
    let scratch = Scratch::new("real");
    let tape = scratch.tape(&format!("{SMALL}Set Timing \"real\"\nType \"echo hi\"\nEnter\nSleep 600ms\n"));

    let recording = Demo::from_file(&tape).unwrap().record().unwrap();

    assert!(recording.duration() >= Duration::from_millis(900), "{:?}", recording.duration());
}

#[test]
fn a_wait_that_never_matches_fails_on_its_line_and_shows_the_screen() {
    let scratch = Scratch::new("timeout");
    let tape = scratch.tape(&format!("{SMALL}Type \"echo nothing-to-see\"\nEnter\nWait@300ms /never/\n"));

    let error = Demo::from_file(&tape).unwrap().record().err().unwrap();

    assert!(error.message.contains("waited 0.3s for /never/"), "{error}");
    assert_eq!(error.location.as_ref().unwrap().line, 7);
    assert!(error.help.as_ref().unwrap().contains("nothing-to-see"), "{error}");
}

#[test]
fn actions_run_off_camera_from_the_tape_or_the_host() {
    let scratch = Scratch::new("actions");
    let tape = scratch.tape(&format!(
        "{SMALL}Action touch \"echo made > made.txt\"\nDo touch\nDo host\nType \"cat made.txt\"\nEnter\nWait /^made$/m\n"
    ));
    let called = Arc::new(Mutex::new(0));
    let counter = called.clone();

    Demo::from_file(&tape)
        .unwrap()
        .action("host", move || {
            *counter.lock().unwrap() += 1;
            Ok::<(), std::io::Error>(())
        })
        .record()
        .unwrap();

    assert_eq!(*called.lock().unwrap(), 1);
    assert_eq!(std::fs::read_to_string(scratch.path("made.txt")).unwrap(), "made\n");
}

#[test]
fn a_failing_action_says_what_it_ran_and_what_it_printed() {
    let scratch = Scratch::new("failing-action");
    let tape = scratch.tape(&format!("{SMALL}Action boom \"echo kaboom >&2; exit 3\"\nType \"x\"\nDo boom\n"));

    let error = Demo::from_file(&tape).unwrap().record().err().unwrap();

    assert!(error.message.contains("Do boom failed"), "{error}");
    assert_eq!(error.help.as_deref(), Some("kaboom"));
    assert_eq!(error.location.unwrap().line, 7);
}

#[test]
fn an_undeclared_action_fails_before_anything_starts() {
    let scratch = Scratch::new("undeclared");
    let tape = scratch.tape("Type \"echo hi\"\nDo reset\n");

    let started = std::time::Instant::now();
    let error = Demo::from_file(&tape).unwrap().record().err().unwrap();

    assert!(error.message.contains("no action is named reset"), "{error}");
    assert!(error.help.unwrap().contains("Action reset"));
    assert!(started.elapsed() < Duration::from_millis(500), "no shell was started");
}

#[test]
fn missing_requirements_fail_before_anything_starts() {
    let scratch = Scratch::new("require");
    let tape = scratch.tape("Require definitely-not-a-program-9000\nType \"x\"\n");

    let error = Demo::from_file(&tape).unwrap().check().unwrap_err();
    assert!(error.message.contains("definitely-not-a-program-9000 is required"), "{error}");
}

#[test]
fn hidden_steps_take_no_time_and_leave_their_effects() {
    let scratch = Scratch::new("hidden");
    let tape = scratch.tape(&format!(
        "{SMALL}Output demo.cast\nHide\nType \"export GREETING=hidden-hello\"\nEnter\nWait\nType \"clear\"\nEnter\nWait\nShow\n\
         Type \"echo $GREETING\"\nEnter\nWait\n"
    ));

    let recording = Demo::from_file(&tape).unwrap().record().unwrap();

    // Only what was shown: "echo $GREETING", an Enter, a Wait.
    assert_eq!(recording.duration(), Duration::from_millis(14 * 50 + 100 + 1000));
    recording.save(scratch.path("demo.cast")).unwrap();
    assert!(std::fs::read_to_string(scratch.path("demo.cast")).unwrap().contains("hidden-hello"));
}

#[test]
fn screenshots_are_taken_where_they_are_written() {
    let scratch = Scratch::new("screenshot");
    let tape = scratch.tape(&format!("{SMALL}Type \"echo one\"\nScreenshot shots/one.png\nEnter\nWait\n"));

    Demo::from_file(&tape).unwrap().record().unwrap();

    let png = std::fs::read(scratch.path("shots/one.png")).unwrap();
    assert_eq!(&png[1..4], b"PNG");
}

#[test]
fn events_arrive_in_order() {
    let scratch = Scratch::new("events");
    let tape = scratch.tape(&format!(
        "{SMALL}Set FontFamily \"Not Installed Anywhere\"\nOutput demo.gif\nAction noop \"true\"\nCaption \"One\"\nType \"a\"\nCaption \"Two\" \"more\"\nDo noop\nType \"b\"\n"
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = events.clone();

    Demo::from_file(&tape).unwrap().on_event(move |event| log.lock().unwrap().push(event.clone())).run().unwrap();

    let events = events.lock().unwrap();
    let names: Vec<&str> = events
        .iter()
        .map(|event| match event {
            Event::Scene { .. } => "scene",
            Event::Action { .. } => "action",
            Event::Recorded { .. } => "recorded",
            Event::Warning { .. } => "warning",
            Event::Saved(_) => "saved",
            _ => "other",
        })
        .collect();
    assert_eq!(names, ["warning", "scene", "scene", "action", "recorded", "saved"]);
    let Event::Warning { message } = &events[0] else { panic!("{:?}", events[0]) };
    assert!(message.contains("Not Installed Anywhere"), "{message}");
    let Event::Scene { number: Some(2), total: 2, caption: Some(caption) } = &events[2] else {
        panic!("{:?}", events[2])
    };
    assert_eq!(caption.detail, "more");
}

#[test]
fn an_image_is_shown_in_the_pane() {
    let scratch = Scratch::new("image");
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 4, 4);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0].repeat(16)).unwrap();
    }
    std::fs::write(scratch.path("red.png"), png).unwrap();
    let tape =
        scratch.tape(&format!("{SMALL}Output last.png\nType \"echo look right\"\nImage \"red.png\"\nSleep 100ms\n"));

    Demo::from_file(&tape).unwrap().run().unwrap();

    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(scratch.path("last.png")).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let pixel = |x: usize, y: usize| &pixels[(y * info.width as usize + x) * 3..][..3];
    assert_eq!(pixel(470, 120), [255, 0, 0], "the right side is the picture");
    assert_ne!(pixel(10, 120), [255, 0, 0], "the left is the terminal");
}

#[test]
fn outputs_can_be_replaced() {
    let scratch = Scratch::new("outputs");
    let tape = scratch.tape(&format!("{SMALL}Output ignored.gif\nType \"x\"\n"));

    let saved = Demo::from_file(&tape).unwrap().outputs([scratch.path("chosen.png")]).run().unwrap();

    assert_eq!(saved[0].path, scratch.path("chosen.png"));
    assert!(!scratch.path("ignored.gif").exists());
}

#[test]
fn a_tape_without_an_output_is_saved_beside_itself_as_a_gif() {
    let scratch = Scratch::new("default-output");
    let tape = scratch.path("intro.tape");
    std::fs::write(&tape, format!("{SMALL}Type \"x\"\n")).unwrap();

    let saved = Demo::from_file(&tape).unwrap().run().unwrap();

    assert_eq!(saved[0].path, scratch.path("intro.gif"));
}

/// Whether a program is installed; the tests that need one say so and pass when it is not.
fn installed(program: &str) -> bool {
    let found = std::process::Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipped: {program} is not installed");
    }
    found
}

#[test]
fn other_shells_start_clean_with_a_prompt_that_waits_work_with() {
    for shell in ["zsh", "sh", "fish"] {
        if !installed(shell) {
            continue;
        }
        let scratch = Scratch::new(shell);
        let tape = scratch.tape(&format!(
            "{SMALL}Set Shell \"{shell}\"\nOutput screens.txt\nType \"echo from-{shell}\"\nEnter\nWait\nType \"echo again\"\nEnter\nWait\n"
        ));

        Demo::from_file(&tape).unwrap().run().unwrap_or_else(|error| panic!("{shell}: {error}"));

        let screens = std::fs::read_to_string(scratch.path("screens.txt")).unwrap();
        assert!(screens.contains(&format!("from-{shell}")), "{shell}: {screens}");
    }
}

#[test]
fn the_terminal_scrolls_back_through_what_scrolled_away() {
    let scratch = Scratch::new("scrollback");
    let tape = scratch
        .tape(&format!("{SMALL}Output screens.txt\nType \"seq 1 60\"\nEnter\nWait\nScrollUp 40\nScrollDown 5\n"));

    Demo::from_file(&tape).unwrap().run().unwrap();

    let screens = std::fs::read_to_string(scratch.path("screens.txt")).unwrap();
    let last = screens.split('─').rfind(|screen| !screen.trim().is_empty()).unwrap();
    assert!(!last.contains("\n60\n"), "scrolled up, 60 is below the view:\n{last}");
    assert!(last.lines().any(|line| line.trim() == "20"), "{last}");
}

#[test]
fn paste_types_what_was_copied() {
    let scratch = Scratch::new("paste");
    let tape = scratch
        .tape(&format!("{SMALL}Output demo.cast\nCopy \"echo pasted-text\"\nPaste\nEnter\nWait /^pasted-text$/m\n"));

    Demo::from_file(&tape).unwrap().run().unwrap();

    assert!(std::fs::read_to_string(scratch.path("demo.cast")).unwrap().contains("pasted-text"));
}

// ── the browser ─────────────────────────────────────────────────────────────────────────────

/// Whether a browser is installed; the browser tests say so and pass when there is none.
fn has_chrome() -> bool {
    let found = ["google-chrome-stable", "google-chrome", "chromium", "chromium-browser"].iter().any(|name| {
        std::process::Command::new("sh")
            .args(["-c", &format!("command -v {name}")])
            .output()
            .is_ok_and(|output| output.status.success())
    }) || std::env::var_os("CHROME_BIN").is_some();
    if !found {
        eprintln!("skipped: no Chrome to record with");
    }
    found
}

const PAGE: &str = r#"<!doctype html><html><body style="margin:0;font:20px sans-serif">
<input id="name" placeholder="name"> <button id="go" onclick="document.getElementById('out').textContent = 'Hello, ' + document.getElementById('name').value">Greet</button>
<p id="out">nobody yet</p><div style="height:2000px"></div><footer id="end">the end</footer>
</body></html>"#;

#[test]
fn a_page_is_clicked_typed_into_and_scrolled() {
    if !has_chrome() {
        return;
    }
    let scratch = Scratch::new("browser");
    std::fs::write(scratch.path("page.html"), PAGE).unwrap();
    let tape = scratch.tape(&format!(
        "{SMALL}Output demo.gif\nOpen \"page.html\"\nClick \"#name\"\nType \"Ada\"\nClick \"text=Greet\"\nWait /Hello, Ada/\n\
         Scroll \"#end\"\nScreenshot end.png\n"
    ));

    let saved = Demo::from_file(&tape).unwrap().run().unwrap();

    assert!(saved[0].frames > 10, "{saved:?}");
    assert!(scratch.path("end.png").exists());
}

#[test]
fn keys_hover_paste_and_scrolling_reach_the_page() {
    if !has_chrome() {
        return;
    }
    let scratch = Scratch::new("browser-keys");
    let page = r##"<!doctype html><body style="margin:0;font:20px sans-serif">
<input id="a"><input id="b"><p id="out"></p><a id="link" href="#" onmouseover="this.textContent='hovered'">link</a>
<div style="height:3000px"></div>
<script>
  document.addEventListener('keydown', (event) => { if (event.key === 'Enter') out.textContent = 'entered ' + b.value; });
</script></body>"##;
    std::fs::write(scratch.path("page.html"), page).unwrap();
    let tape = scratch.tape(&format!(
        "{SMALL}Output last.png\nOpen \"page.html\"\nClick \"#a\"\nType \"first\"\nTab\nCopy \"second\"\nPaste\nEnter\nWait /entered second/\n\
         Hover \"#link\"\nWait /hovered/\nScroll 500\nScrollDown 2\nWait\n"
    ));

    Demo::from_file(&tape).unwrap().run().unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn a_page_that_never_shows_the_text_fails_on_its_line() {
    if !has_chrome() {
        return;
    }
    let scratch = Scratch::new("browser-timeout");
    std::fs::write(scratch.path("page.html"), PAGE).unwrap();
    let tape = scratch.tape("Open \"page.html\"\nWait@500ms /Goodbye/\n");

    let error = Demo::from_file(&tape).unwrap().record().err().unwrap();

    assert!(error.message.contains("/Goodbye/ in the browser"), "{error}");
    assert_eq!(error.location.unwrap().line, 2);
}

#[test]
fn clicking_what_is_not_there_says_so() {
    if !has_chrome() {
        return;
    }
    let scratch = Scratch::new("browser-missing");
    std::fs::write(scratch.path("page.html"), PAGE).unwrap();
    let tape = scratch.tape("Open \"page.html\"\nClick@300ms \"#nope\"\n");

    let error = Demo::from_file(&tape).unwrap().record().err().unwrap();

    assert!(error.message.contains("no visible element matches #nope"), "{error}");
}
