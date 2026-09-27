//! Headless Chrome, driven over the DevTools protocol, filmed by its own screencast.
//!
//! The page paints into Chrome's screencast, which sends a frame each time something on it
//! changes. Clicks and keys go in as real input events, so hover styles, focus rings and
//! `onChange` handlers all happen the way they would for a person.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::cdp::{Connection, base64_decode};
use crate::encode::parallel_map;
use crate::image::Image;
use crate::render::PaneFrame;
use crate::timeline::Timed;
use crate::{Error, Key, Pattern, Result, Rgb, ScrollTarget, which};

/// How often a page is asked whether it is ready yet.
const POLL: Duration = Duration::from_millis(40);

/// Every screencast frame, still encoded, and the address it was taken at.
#[derive(Default)]
struct Filmed {
    frames: Vec<Timed<(Vec<u8>, Option<String>)>>,
    url: Option<String>,
}

/// The Chrome process and its profile directory, which are gone when this is dropped — whatever
/// failed on the way to a working browser.
struct Chrome {
    process: Child,
    profile: PathBuf,
}

impl Drop for Chrome {
    fn drop(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && matches!(self.process.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// A headless browser with one page, being filmed.
pub(crate) struct Browser {
    connection: Connection,
    session: String,
    filmed: Arc<Mutex<Filmed>>,
    /// Last, so it is dropped after the connection has asked Chrome to close.
    _chrome: Chrome,
}

impl Browser {
    /// Starts Chrome with a page `size` pixels big, laid out `1 / zoom` times wider, and starts
    /// filming it.
    pub fn launch(size: (u32, u32), zoom: f32, epoch: Instant) -> Result<Browser> {
        let chrome = find_chrome()?;
        let profile =
            std::env::temp_dir().join(format!("demogod-chrome-{}-{}", std::process::id(), epoch.elapsed().as_nanos()));
        std::fs::create_dir_all(&profile).map_err(|error| Error::io(&profile, error))?;
        let mut command = Command::new(&chrome);
        command
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--window-size={},{}", size.0, size.1))
            .args(["--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--mute-audio"])
            .args(["--disable-extensions", "--disable-sync", "--disable-background-networking"])
            .args(["--disable-component-update", "--disable-default-apps", "--force-color-profile=srgb"])
            .args(std::env::var("DEMOGOD_CHROME_FLAGS").unwrap_or_default().split_whitespace())
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Chrome's sandbox needs user namespaces, which containers and CI runners often forbid.
        if cfg!(target_os = "linux") {
            command.arg("--no-sandbox");
        }
        let process = command.spawn().map_err(|error| {
            let _ = std::fs::remove_dir_all(&profile);
            Error::new(format!("could not start {}: {error}", chrome.display()))
        })?;
        let mut chrome = Chrome { process, profile };

        let (port, path) = devtools_address(&chrome.profile, &mut chrome.process)?;
        let filmed = Arc::new(Mutex::new(Filmed::default()));
        let filming = filmed.clone();
        let connection = Connection::open(
            port,
            &path,
            Box::new(move |connection, method, params, session| match method {
                "Page.screencastFrame" => {
                    let at = epoch.elapsed();
                    connection.notify("Page.screencastFrameAck", json!({ "sessionId": params["sessionId"] }), session);
                    let Ok(png) = base64_decode(params["data"].as_str().unwrap_or("")) else { return };
                    let mut filmed = filming.lock().expect("never poisoned");
                    let url = filmed.url.clone();
                    // Two frames within a hundredth of a second: only the later one is ever seen.
                    if filmed.frames.last().is_some_and(|last| at - last.at < Duration::from_millis(10)) {
                        filmed.frames.pop();
                    }
                    filmed.frames.push(Timed { at, value: (png, url) });
                }
                "Page.frameNavigated" if params["frame"].get("parentId").is_none() => {
                    let url = params["frame"]["url"].as_str().map(str::to_string);
                    filming.lock().expect("never poisoned").url = url.filter(|url| url != "about:blank");
                }
                _ => {}
            }),
        )?;

        let target = connection.call("Target.createTarget", json!({ "url": "about:blank" }), None)?;
        let attached = connection.call(
            "Target.attachToTarget",
            json!({ "targetId": target["targetId"], "flatten": true }),
            None,
        )?;
        let session =
            attached["sessionId"].as_str().ok_or_else(|| Error::new("the browser gave no page session"))?.to_string();
        let browser = Browser { connection, session, filmed, _chrome: chrome };
        browser.call("Page.enable", json!({}))?;
        browser.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": (size.0 as f32 / zoom).round() as u32,
                "height": (size.1 as f32 / zoom).round() as u32,
                "deviceScaleFactor": zoom,
                "mobile": false,
            }),
        )?;
        browser.call(
            "Page.startScreencast",
            json!({ "format": "png", "everyNthFrame": 1, "maxWidth": size.0, "maxHeight": size.1 }),
        )?;

        Ok(browser)
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.connection.call(method, params, Some(&self.session))
    }

    /// Runs JavaScript in the page and returns what it evaluates to.
    fn evaluate(&self, expression: &str) -> Result<Value> {
        let result = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        )?;
        if let Some(exception) = result.get("exceptionDetails") {
            let message = exception["exception"]["description"].as_str().or(exception["text"].as_str()).unwrap_or("?");
            return Err(Error::new(format!("the page threw: {message}")));
        }

        Ok(result["result"]["value"].clone())
    }

    /// Goes to a page and waits for it to load and paint.
    pub fn open(&mut self, url: &str, timeout: Duration) -> Result<()> {
        let frames_before = self.filmed.lock().expect("never poisoned").frames.len();
        let navigated = self.call("Page.navigate", json!({ "url": url }))?;
        if let Some(error) = navigated["errorText"].as_str().filter(|error| !error.is_empty()) {
            return Err(Error::new(format!("could not open {url}: {error}"))
                .help("is the server up? `Wait` for it in the terminal before opening it"));
        }
        self.wait(None, timeout)?;
        // The step is over when the page is on screen, not merely loaded.
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.filmed.lock().expect("never poisoned").frames.len() == frames_before && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }

        Ok(())
    }

    /// Blocks until the page's text matches, or — with no pattern — until it has loaded.
    pub fn wait(&self, pattern: Option<&Pattern>, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let done = match pattern {
                None => self.evaluate("document.readyState")? == "complete",
                Some(pattern) => pattern
                    .matches(self.evaluate("document.body ? document.body.innerText : ''")?.as_str().unwrap_or("")),
            };
            if done {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let wanted = pattern.map_or("the page to load".to_string(), |pattern| pattern.to_string());
                return Err(Error::new(format!("waited {}s for {wanted} in the browser", timeout.as_secs_f32())));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Where an element's centre is, in page pixels, once it is visible and scrolled into view.
    pub fn locate(&self, target: &str, timeout: Duration) -> Result<(f32, f32)> {
        let expression = LOCATE.replace("TARGET", &serde_json::to_string(target).expect("a string serializes"));
        let deadline = Instant::now() + timeout;
        let mut last = Value::Null;
        loop {
            let found = self.evaluate(&expression)?;
            // Scrolling smoothly: found only once it stops moving.
            if let (Some(x), Some(y)) = (found["x"].as_f64(), found["y"].as_f64()) {
                if found == last {
                    return Ok((x as f32, y as f32));
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::new(format!("no visible element matches {target}"))
                    .help("use a CSS selector like \"button.save\", or text=Visible words"));
            }
            last = found;
            std::thread::sleep(POLL);
        }
    }

    /// Moves the pointer from one place to another through the points between, so the page sees
    /// the path, hover effects and all.
    pub fn move_pointer(&self, from: (f32, f32), to: (f32, f32)) -> Result<()> {
        const STEPS: u32 = 8;
        for step in 1..=STEPS {
            let part = step as f32 / STEPS as f32;
            let (x, y) = (from.0 + (to.0 - from.0) * part, from.1 + (to.1 - from.1) * part);
            self.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y }))?;
            std::thread::sleep(Duration::from_millis(12));
        }

        Ok(())
    }

    /// Presses and releases the left button where the pointer is.
    pub fn click(&self, x: f32, y: f32) -> Result<()> {
        for kind in ["mousePressed", "mouseReleased"] {
            self.call(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 }),
            )?;
        }

        Ok(())
    }

    /// Types one character into whatever has focus, as a real key press.
    pub fn type_character(&self, character: char) -> Result<()> {
        let text = character.to_string();
        self.call(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyDown", "text": text, "key": text, "unmodifiedText": text }),
        )?;
        self.call("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": text }))?;

        Ok(())
    }

    /// Presses a key, with its modifiers, in whatever has focus.
    pub fn press(&self, key: &Key) -> Result<()> {
        let (name, code, key_code, text) = key.dom();
        let modifiers = key.dom_modifiers();
        let mut down = json!({
            "type": if text.is_some() { "keyDown" } else { "rawKeyDown" },
            "key": name, "code": code, "windowsVirtualKeyCode": key_code, "modifiers": modifiers,
        });
        if let Some(text) = text {
            down["text"] = json!(text);
        }
        self.call("Input.dispatchKeyEvent", down)?;
        self.call(
            "Input.dispatchKeyEvent",
            json!({ "type": "keyUp", "key": name, "code": code, "windowsVirtualKeyCode": key_code, "modifiers": modifiers }),
        )?;

        Ok(())
    }

    /// Inserts text all at once, the way pasting does.
    pub fn insert_text(&self, text: &str) -> Result<()> {
        self.call("Input.insertText", json!({ "text": text })).map(drop)
    }

    /// Scrolls smoothly, by pixels or to an element, and waits for the page to come to rest.
    pub fn scroll(&self, target: &ScrollTarget, timeout: Duration) -> Result<()> {
        match target {
            ScrollTarget::Pixels(pixels) => {
                self.evaluate(&format!("window.scrollBy({{ top: {pixels}, behavior: 'smooth' }})"))?;
            }
            ScrollTarget::Lines(lines) => {
                self.evaluate(&format!("window.scrollBy({{ top: {lines} * 40, behavior: 'smooth' }})"))?;
            }
            ScrollTarget::Element(selector) => {
                self.locate(selector, timeout)?;
            }
        }
        // At rest: the same position twice in a row.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut last = Value::Null;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(80));
            let now = self.evaluate("[scrollX, scrollY]")?;
            if now == last {
                break;
            }
            last = now;
        }

        Ok(())
    }

    /// Stops filming, closes the browser, and returns every frame it painted, decoded.
    pub fn finish(self) -> Result<Vec<Timed<PaneFrame>>> {
        let _ = self.call("Page.stopScreencast", json!({}));
        let frames = std::mem::take(&mut self.filmed.lock().expect("never poisoned").frames);

        let decoded = parallel_map(&frames, |_, frame| {
            let (png, url) = &frame.value;
            Image::from_png(png, Rgb(255, 255, 255))
                .map(|image| Timed { at: frame.at, value: PaneFrame { image: Arc::new(image), url: url.clone() } })
        });

        decoded.into_iter().collect()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Asked to close, Chrome exits on its own; `Chrome` makes sure of it.
        self.connection.notify("Browser.close", json!({}), None);
    }
}

/// Finds an element by CSS selector or `text=…`, and returns its centre if it is visible and in
/// view — or starts scrolling it into view and returns nothing yet.
const LOCATE: &str = r#"(() => {
  const target = TARGET;
  let element = null;
  if (target.startsWith('text=')) {
    const wanted = target.slice(5).trim();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    let partial = null;
    while (walker.nextNode()) {
      const text = (walker.currentNode.innerText || '').trim();
      if (text === wanted) element = walker.currentNode;
      else if (!element && text.includes(wanted)) partial = walker.currentNode;
    }
    element = element || partial;
  } else {
    element = document.querySelector(target);
  }
  if (!element) return null;
  const box = element.getBoundingClientRect();
  const style = getComputedStyle(element);
  if (box.width === 0 || box.height === 0 || style.visibility === 'hidden') return null;
  const x = box.left + box.width / 2, y = box.top + box.height / 2;
  if (x < 0 || y < 0 || x > innerWidth || y > innerHeight) {
    element.scrollIntoView({ block: 'center', inline: 'center', behavior: 'smooth' });
    return { scrolling: true, at: [scrollX, scrollY] };
  }
  return { x, y };
})()"#;

/// Waits for Chrome to write the port it listens on into its profile.
fn devtools_address(profile: &std::path::Path, process: &mut Child) -> Result<(u16, String)> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(contents) = std::fs::read_to_string(&file) {
            let mut lines = contents.lines();
            if let (Some(Ok(port)), Some(path)) = (lines.next().map(str::parse), lines.next()) {
                return Ok((port, path.to_string()));
            }
        }
        if let Ok(Some(status)) = process.try_wait() {
            return Err(Error::new(format!("the browser exited before it was ready ({status})"))
                .help("set DEMOGOD_CHROME_FLAGS for flags your system needs, or CHROME_BIN to another Chrome"));
        }
        if Instant::now() > deadline {
            return Err(Error::new("the browser took more than 20s to start"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Chrome, Chromium or Edge: `CHROME_BIN` if it is set, else the first one installed.
pub(crate) fn find_chrome() -> Result<PathBuf> {
    find_chrome_in(|variable| std::env::var_os(variable))
}

fn find_chrome_in(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf> {
    if let Some(path) = env("CHROME_BIN").map(PathBuf::from) {
        return if path.is_file() {
            Ok(path)
        } else {
            Err(Error::new(format!("CHROME_BIN is {}, which is not a file", path.display())))
        };
    }
    let names =
        ["google-chrome-stable", "google-chrome", "chromium", "chromium-browser", "chrome", "microsoft-edge", "msedge"];
    if let Some(path) = names.iter().find_map(|name| which(name)) {
        return Ok(path);
    }

    let home = env("HOME").map(PathBuf::from).unwrap_or_default();
    let program_files = [env("ProgramFiles"), env("ProgramFiles(x86)"), env("LOCALAPPDATA")];
    let mut candidates = vec![
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
    ];
    for root in program_files.into_iter().flatten().map(PathBuf::from) {
        candidates.push(root.join(r"Google\Chrome\Application\chrome.exe"));
        candidates.push(root.join(r"Microsoft\Edge\Application\msedge.exe"));
    }

    candidates.into_iter().find(|path| path.is_file()).ok_or_else(|| {
        Error::new("this tape opens a browser, and no Chrome, Chromium or Edge is installed")
            .help("install one, or point CHROME_BIN at it")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_bin_wins_and_must_exist() {
        let env = |variable: &str| (variable == "CHROME_BIN").then(|| "/definitely/not/chrome".into());
        let error = find_chrome_in(env).unwrap_err();

        assert!(error.message.contains("CHROME_BIN is /definitely/not/chrome"), "{}", error.message);
    }

    #[test]
    fn an_existing_chrome_bin_is_used_as_is() {
        let path = std::env::current_exe().unwrap();
        let env = |variable: &str| (variable == "CHROME_BIN").then(|| path.clone().into_os_string());

        assert_eq!(find_chrome_in(env).unwrap(), path);
    }
}
