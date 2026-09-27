//! One error type for everything, which points at the tape line it is about when there is one.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A line in a tape: which file, and which line of it, counted from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The tape the line is in — the one given, or one it `Source`s.
    pub file: Arc<Path>,
    /// The line number, from 1.
    pub line: usize,
}

impl fmt::Display for Location {
    /// `demo.tape:12`, relative to the current directory when the tape is inside it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let here = std::env::current_dir().unwrap_or_default();
        let file = self.file.strip_prefix(&here).unwrap_or(&self.file);
        write!(f, "{}:{}", file.display(), self.line)
    }
}

/// What went wrong, and where in the tape if it was the tape's doing.
///
/// Displayed the way a compiler reports: the message, the file and line, the line itself with the
/// offending part underlined, and a hint when there is an obvious fix.
///
/// ```text
/// error: not a duration: 5x
///   --> demo.tape:12
///    |
/// 12 | Sleep 5x
///    |       ^^
///    = help: durations are written 500ms, 2s or 1.5s
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// What went wrong, in a sentence.
    pub message: String,
    /// The tape line it happened on.
    pub location: Option<Location>,
    /// The text of that line, and the byte range of it to underline.
    pub excerpt: Option<(String, std::ops::Range<usize>)>,
    /// How to fix it, when that is clear.
    pub help: Option<String>,
}

impl Error {
    /// An error that is not about any particular line.
    pub fn new(message: impl Into<String>) -> Error {
        Error { message: message.into(), location: None, excerpt: None, help: None }
    }

    /// The same error, placed on a tape line.
    pub fn at(mut self, location: Location) -> Error {
        self.location.get_or_insert(location);
        self
    }

    /// The same error, with the line's text and the part of it to underline.
    pub fn excerpt(mut self, text: &str, span: std::ops::Range<usize>) -> Error {
        self.excerpt = Some((text.to_string(), span));
        self
    }

    /// The same error, with a hint on how to fix it.
    pub fn help(mut self, help: impl Into<String>) -> Error {
        self.help = Some(help.into());
        self
    }

    /// An I/O error, saying which file it was about.
    pub fn io(path: &Path, error: std::io::Error) -> Error {
        Error::new(format!("{}: {error}", path.display()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        let Some(location) = &self.location else {
            return match &self.help {
                Some(help) => write!(f, "\n  = help: {help}"),
                None => Ok(()),
            };
        };

        let number = location.line.to_string();
        let gutter = " ".repeat(number.len());
        write!(f, "\n{gutter}--> {location}")?;
        if let Some((text, span)) = &self.excerpt {
            let before = text[..span.start.min(text.len())].chars().count();
            let width = text.get(span.clone()).map_or(1, |part| part.chars().count().max(1));
            write!(f, "\n{gutter} |\n{number} | {text}\n{gutter} | {}{}", " ".repeat(before), "^".repeat(width))?;
        }
        if let Some(help) = &self.help {
            write!(f, "\n{gutter} = help: {help}")?;
        }

        Ok(())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Error {
        Error::new(error.to_string())
    }
}

/// `Result` with this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Where a path is, relative to a tape's directory unless it is absolute already, with `.` and
/// `..` worked out so it prints the way a person would write it.
pub(crate) fn resolve(base: &Path, path: impl AsRef<Path>) -> PathBuf {
    use std::path::Component;

    let path = path.as_ref();
    let joined = if path.is_absolute() { path.to_path_buf() } else { base.join(path) };
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if normal.file_name().is_some() => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    // A trailing separator marks a directory to fill, so it is kept.
    if path.as_os_str().to_string_lossy().ends_with(['/', '\\']) {
        normal.push("");
    }

    normal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location() -> Location {
        Location { file: Arc::from(Path::new("demo.tape")), line: 12 }
    }

    #[test]
    fn displays_like_a_compiler() {
        let error = Error::new("not a duration: 5x")
            .at(location())
            .excerpt("Sleep 5x", 6..8)
            .help("durations are written 500ms, 2s or 1.5s");

        assert_eq!(
            error.to_string(),
            "not a duration: 5x\n  --> demo.tape:12\n   |\n12 | Sleep 5x\n   |       ^^\n   = help: durations are written 500ms, 2s or 1.5s"
        );
    }

    #[test]
    fn the_first_location_given_is_the_one_kept() {
        let other = Location { file: Arc::from(Path::new("other.tape")), line: 1 };
        assert_eq!(Error::new("x").at(location()).at(other).location, Some(location()));
    }

    #[test]
    fn without_a_location_it_is_the_message_and_any_help() {
        assert_eq!(Error::new("boom").to_string(), "boom");
        assert_eq!(Error::new("boom").help("duck").to_string(), "boom\n  = help: duck");
    }

    #[test]
    fn resolve_works_out_dots_and_keeps_a_trailing_separator() {
        assert_eq!(resolve(Path::new("/a/b"), "../c.gif"), PathBuf::from("/a/c.gif"));
        assert_eq!(resolve(Path::new("/a/b"), "./c/./d.gif"), PathBuf::from("/a/b/c/d.gif"));
        assert_eq!(resolve(Path::new("/a/b"), "/x/../y"), PathBuf::from("/y"));
        assert!(resolve(Path::new("/a"), "frames/").to_string_lossy().ends_with('/'));
    }

    #[test]
    fn an_empty_span_still_underlines_one_character() {
        let error = Error::new("x").at(location()).excerpt("Type", 4..4);
        assert!(error.to_string().ends_with("|     ^"), "{error}");
    }
}
