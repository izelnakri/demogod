//! Keys a tape can press, and what each one is to a terminal and to a browser.

use std::fmt;

/// A key without modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    /// `Enter`
    Enter,
    /// `Tab`
    Tab,
    /// `Backspace`
    Backspace,
    /// `Delete`
    Delete,
    /// `Insert`
    Insert,
    /// `Escape`
    Escape,
    /// `Space`
    Space,
    /// `Up`
    Up,
    /// `Down`
    Down,
    /// `Left`
    Left,
    /// `Right`
    Right,
    /// `Home`
    Home,
    /// `End`
    End,
    /// `PageUp`
    PageUp,
    /// `PageDown`
    PageDown,
    /// A printable character: the `C` in `Ctrl+C`.
    Char(char),
}

/// A key, and the modifiers held while it is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// Which key.
    pub code: KeyCode,
    /// Whether Ctrl is held.
    pub ctrl: bool,
    /// Whether Alt is held.
    pub alt: bool,
    /// Whether Shift is held.
    pub shift: bool,
}

const NAMED: &[(&str, KeyCode)] = &[
    ("Enter", KeyCode::Enter),
    ("Tab", KeyCode::Tab),
    ("Backspace", KeyCode::Backspace),
    ("Delete", KeyCode::Delete),
    ("Insert", KeyCode::Insert),
    ("Escape", KeyCode::Escape),
    ("Space", KeyCode::Space),
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
];

impl Key {
    /// A key with no modifiers held.
    pub fn plain(code: KeyCode) -> Key {
        Key { code, ctrl: false, alt: false, shift: false }
    }

    /// Reads `Enter`, `Ctrl+C`, `Alt+Shift+Left` — modifiers first, the key last, joined by `+`.
    ///
    /// ```
    /// use demogod::{Key, KeyCode};
    ///
    /// let key = Key::parse("Ctrl+C").unwrap();
    /// assert_eq!((key.code, key.ctrl), (KeyCode::Char('c'), true));
    /// assert_eq!(Key::parse("PageDown").unwrap().code, KeyCode::PageDown);
    /// assert!(Key::parse("Ctrl+").is_none());
    /// ```
    pub fn parse(text: &str) -> Option<Key> {
        let mut parts: Vec<&str> = text.split('+').collect();
        let name = parts.pop()?;
        let code = match NAMED.iter().find(|(named, _)| *named == name) {
            Some((_, code)) => *code,
            None => {
                let mut characters = name.chars();
                match (characters.next(), characters.next()) {
                    (Some(character), None) if !parts.is_empty() => KeyCode::Char(character.to_ascii_lowercase()),
                    _ => return None,
                }
            }
        };
        let mut key = Key::plain(code);
        for modifier in parts {
            match modifier {
                "Ctrl" => key.ctrl = true,
                "Alt" => key.alt = true,
                "Shift" => key.shift = true,
                _ => return None,
            }
        }

        Some(key)
    }

    /// Whether this is one of the named keys a tape can write on its own, without a modifier.
    pub fn is_named(text: &str) -> bool {
        NAMED.iter().any(|(named, _)| *named == text)
    }

    /// The bytes a terminal program reads when this key is pressed, the way xterm sends them.
    ///
    /// ```
    /// use demogod::{Key, KeyCode};
    ///
    /// assert_eq!(Key::parse("Ctrl+C").unwrap().bytes(), b"\x03");
    /// assert_eq!(Key::plain(KeyCode::Up).bytes(), b"\x1b[A");
    /// assert_eq!(Key::parse("Alt+B").unwrap().bytes(), b"\x1bb");
    /// assert_eq!(Key::parse("Shift+Tab").unwrap().bytes(), b"\x1b[Z");
    /// ```
    pub fn bytes(&self) -> Vec<u8> {
        let modifier = 1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8;
        let csi = |final_byte: char| {
            if modifier == 1 { format!("\x1b[{final_byte}") } else { format!("\x1b[1;{modifier}{final_byte}") }
        };
        let tilde = |number: u8| {
            if modifier == 1 { format!("\x1b[{number}~") } else { format!("\x1b[{number};{modifier}~") }
        };
        let sequence = match self.code {
            KeyCode::Enter => "\r".to_string(),
            KeyCode::Tab if self.shift => "\x1b[Z".to_string(),
            KeyCode::Tab => "\t".to_string(),
            KeyCode::Backspace => "\x7f".to_string(),
            KeyCode::Escape => "\x1b".to_string(),
            KeyCode::Space if self.ctrl => "\0".to_string(),
            KeyCode::Space => " ".to_string(),
            KeyCode::Up => csi('A'),
            KeyCode::Down => csi('B'),
            KeyCode::Right => csi('C'),
            KeyCode::Left => csi('D'),
            KeyCode::Home => csi('H'),
            KeyCode::End => csi('F'),
            KeyCode::Insert => tilde(2),
            KeyCode::Delete => tilde(3),
            KeyCode::PageUp => tilde(5),
            KeyCode::PageDown => tilde(6),
            KeyCode::Char(character) if self.ctrl => {
                let control = match character.to_ascii_lowercase() {
                    letter @ 'a'..='z' => letter as u8 - b'a' + 1,
                    '@' | '2' => 0,
                    '[' | '3' => 27,
                    '\\' | '4' => 28,
                    ']' | '5' => 29,
                    '^' | '6' => 30,
                    '_' | '7' | '/' => 31,
                    '8' | '?' => 127,
                    other => other as u8,
                };
                return self.with_alt(vec![control]);
            }
            KeyCode::Char(character) if self.shift => character.to_uppercase().collect(),
            KeyCode::Char(character) => character.to_string(),
        };

        // A modified cursor key already says Alt inside its CSI; everything else is prefixed.
        if sequence.starts_with("\x1b[") && modifier > 1 {
            return sequence.into_bytes();
        }
        self.with_alt(sequence.into_bytes())
    }

    fn with_alt(&self, bytes: Vec<u8>) -> Vec<u8> {
        if self.alt { [b"\x1b".as_slice(), &bytes].concat() } else { bytes }
    }

    /// What Chrome calls this key: the `key` and `code` of a `KeyboardEvent`, its legacy key code,
    /// and the text it types, if any.
    pub(crate) fn dom(&self) -> (String, String, u32, Option<String>) {
        let named = |key: &str, code: u32| (key.to_string(), key.to_string(), code, None);
        match self.code {
            KeyCode::Enter => ("Enter".into(), "Enter".into(), 13, Some("\r".into())),
            KeyCode::Tab => named("Tab", 9),
            KeyCode::Backspace => named("Backspace", 8),
            KeyCode::Delete => named("Delete", 46),
            KeyCode::Insert => named("Insert", 45),
            KeyCode::Escape => named("Escape", 27),
            KeyCode::Space => (" ".into(), "Space".into(), 32, Some(" ".into())),
            KeyCode::Up => ("ArrowUp".into(), "ArrowUp".into(), 38, None),
            KeyCode::Down => ("ArrowDown".into(), "ArrowDown".into(), 40, None),
            KeyCode::Left => ("ArrowLeft".into(), "ArrowLeft".into(), 37, None),
            KeyCode::Right => ("ArrowRight".into(), "ArrowRight".into(), 39, None),
            KeyCode::Home => named("Home", 36),
            KeyCode::End => named("End", 35),
            KeyCode::PageUp => named("PageUp", 33),
            KeyCode::PageDown => named("PageDown", 34),
            KeyCode::Char(character) => {
                let upper = character.to_ascii_uppercase();
                let key = if self.shift { upper.to_string() } else { character.to_string() };
                let code = if upper.is_ascii_alphabetic() {
                    format!("Key{upper}")
                } else if upper.is_ascii_digit() {
                    format!("Digit{upper}")
                } else {
                    String::new()
                };
                let text = (!self.ctrl && !self.alt).then(|| key.clone());
                (key, code, upper as u32, text)
            }
        }
    }

    /// The modifier bit mask Chrome's `Input.dispatchKeyEvent` takes: Alt 1, Ctrl 2, Shift 8.
    pub(crate) fn dom_modifiers(&self) -> u8 {
        self.alt as u8 | (self.ctrl as u8) << 1 | (self.shift as u8) << 3
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if held {
                f.write_str(name)?;
            }
        }
        match self.code {
            KeyCode::Char(character) => write!(f, "{}", character.to_ascii_uppercase()),
            code => {
                let name = NAMED.iter().find(|(_, named)| *named == code).map(|(name, _)| *name);
                f.write_str(name.unwrap_or("?"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Vec<u8> {
        Key::parse(text).unwrap().bytes()
    }

    #[test]
    fn named_keys_send_what_xterm_sends() {
        assert_eq!(bytes("Enter"), b"\r");
        assert_eq!(bytes("Tab"), b"\t");
        assert_eq!(bytes("Backspace"), b"\x7f");
        assert_eq!(bytes("Escape"), b"\x1b");
        assert_eq!(bytes("Space"), b" ");
        assert_eq!(bytes("Home"), b"\x1b[H");
        assert_eq!(bytes("End"), b"\x1b[F");
        assert_eq!(bytes("Delete"), b"\x1b[3~");
        assert_eq!(bytes("Insert"), b"\x1b[2~");
        assert_eq!(bytes("PageUp"), b"\x1b[5~");
        assert_eq!(bytes("PageDown"), b"\x1b[6~");
    }

    #[test]
    fn modifiers_on_cursor_keys_go_inside_the_sequence() {
        assert_eq!(bytes("Ctrl+Left"), b"\x1b[1;5D");
        assert_eq!(bytes("Shift+Up"), b"\x1b[1;2A");
        assert_eq!(bytes("Alt+Right"), b"\x1b[1;3C");
        assert_eq!(bytes("Ctrl+Delete"), b"\x1b[3;5~");
    }

    #[test]
    fn control_characters() {
        assert_eq!(bytes("Ctrl+A"), b"\x01");
        assert_eq!(bytes("Ctrl+Z"), b"\x1a");
        assert_eq!(bytes("Ctrl+["), b"\x1b");
        assert_eq!(bytes("Ctrl+Space"), b"\0");
        assert_eq!(bytes("Ctrl+Alt+C"), b"\x1b\x03");
        assert_eq!(bytes("Shift+A"), b"A");
        assert_eq!(bytes("Alt+Enter"), b"\x1b\r");
    }

    #[test]
    fn a_bare_letter_is_not_a_key() {
        // `A` alone would read as a typo for `Type "A"`, so it has to be spelled that way.
        assert_eq!(Key::parse("A"), None);
        assert_eq!(Key::parse("Hyper+A"), None);
        assert_eq!(Key::parse("Ctrl+Nope"), None);
    }

    #[test]
    fn display_is_what_parse_reads() {
        for text in ["Enter", "Ctrl+C", "Ctrl+Alt+Shift+Left", "Alt+PageDown"] {
            assert_eq!(Key::parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn dom_names() {
        assert_eq!(Key::parse("Enter").unwrap().dom().2, 13);
        assert_eq!(Key::parse("Ctrl+A").unwrap().dom(), ("a".into(), "KeyA".into(), 65, None));
        assert_eq!(Key::parse("Shift+A").unwrap().dom().3, Some("A".into()));
        assert_eq!(Key::parse("Ctrl+Shift+Left").unwrap().dom_modifiers(), 10);
        assert_eq!(Key::parse("Up").unwrap().dom().0, "ArrowUp");
    }
}
