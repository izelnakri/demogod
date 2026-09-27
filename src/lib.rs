//! **demogod** records demos from a tape: a terminal, a browser beside it, or both, typed into
//! and clicked through by a script, and saved as a GIF, an MP4 or WebM, a PNG, or an asciicast.
//!
//! ```text
//! Output demo.gif
//! Set Theme "Catppuccin Mocha"
//!
//! Caption "Run the tests" "in a real shell"
//! Type "npm test"
//! Enter
//! Wait /passing/
//! Sleep 2s
//! ```
//!
//! The tape language is VHS's, so a VHS tape is a demogod tape; demogod adds captions, a browser
//! pane, and actions run off camera. The terminal is a real PTY rendered by demogod itself — no
//! ttyd, no browser, no ffmpeg — so a terminal demo needs nothing but this crate. And under the
//! default tape timing, every step lasts as long as the tape says it does, not as long as the
//! machine took: the same tape makes the same film, on a laptop or a busy CI runner.
//!
//! # Recording from Rust
//!
//! ```no_run
//! use demogod::Demo;
//!
//! let saved = Demo::from_file("demo.tape")?
//!     // `Do break` in the tape runs this, off camera.
//!     .action("break", || std::fs::write("test/cart-test.ts", "// broken"))
//!     .run()?;
//! for file in saved {
//!     println!("{}: {} bytes", file.path.display(), file.bytes);
//! }
//! # Ok::<(), demogod::Error>(())
//! ```
//!
//! [`Demo::record`] returns the [`Recording`] instead, to save wherever and however often.
//! [`Tape::parse`] reads a tape without running anything, and [`Demo::check`] finds what would
//! fail — a missing program, an undeclared action — before a recording starts.

mod browser;
mod cdp;
mod color;
mod demo;
mod encode;
mod error;
mod font;
mod image;
mod key;
mod recording;
mod render;
mod settings;
mod tape;
mod terminal;
mod theme;
mod timeline;

pub use color::Rgb;
pub use demo::{Demo, Event};
pub use error::{Error, Location, Result};
pub use key::{Key, KeyCode};
pub use recording::{Recording, Saved};
pub use settings::{Length, LoopOffset, Settings, Timing, WindowBar};
pub use tape::{Caption, Command, Focus, Pattern, Scene, ScrollTarget, Step, Tape, WaitScope};
pub use theme::Theme;

/// Where a program is on PATH, the way a shell would find it: as named, or on Windows with any
/// of `PATHEXT`'s extensions added.
pub(crate) fn which(program: &str) -> Option<std::path::PathBuf> {
    let mut names = vec![program.to_string()];
    if cfg!(windows) {
        let extensions = std::env::var("PATHEXT").unwrap_or(".EXE;.CMD;.BAT".into());
        names.extend(
            extensions
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(|extension| format!("{program}{extension}")),
        );
    }
    let path = std::path::Path::new(program);
    if path.components().count() > 1 {
        return names.iter().map(std::path::PathBuf::from).find(|candidate| candidate.is_file());
    }

    std::env::split_paths(&std::env::var_os("PATH")?)
        .find_map(|directory| names.iter().map(|name| directory.join(name)).find(|candidate| candidate.is_file()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn which_finds_programs_by_name_or_path_and_not_what_is_missing() {
        let shell = super::which("sh").expect("sh is on PATH");
        assert!(shell.is_file());
        assert_eq!(super::which(shell.to_str().unwrap()), Some(shell));
        assert_eq!(super::which("definitely-not-a-program-9000"), None);
    }
}
