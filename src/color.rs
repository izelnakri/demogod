//! Colors, as the three bytes everything in here ends up as.

use std::fmt;

/// An sRGB color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Reads `#rrggbb` or `#rgb`; the `#` is optional.
    ///
    /// ```
    /// use demogod::Rgb;
    ///
    /// assert_eq!(Rgb::parse("#ff8000"), Some(Rgb(255, 128, 0)));
    /// assert_eq!(Rgb::parse("f80"), Some(Rgb(255, 136, 0)));
    /// assert_eq!(Rgb::parse("orange"), None);
    /// ```
    pub fn parse(hex: &str) -> Option<Rgb> {
        let hex = hex.trim().trim_start_matches('#');
        let digit = |index: usize| u8::from_str_radix(hex.get(index..index + 1)?, 16).ok();
        let pair = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();

        match hex.len() {
            3 => Some(Rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
            6 => Some(Rgb(pair(0)?, pair(2)?, pair(4)?)),
            _ => None,
        }
    }

    /// `self` blended toward `other` by `amount` out of 255.
    ///
    /// ```
    /// use demogod::Rgb;
    ///
    /// assert_eq!(Rgb(0, 0, 0).mix(Rgb(255, 255, 255), 255), Rgb(255, 255, 255));
    /// assert_eq!(Rgb(0, 0, 0).mix(Rgb(200, 100, 0), 128), Rgb(100, 50, 0));
    /// ```
    pub fn mix(self, other: Rgb, amount: u8) -> Rgb {
        let blend = |from: u8, to: u8| {
            let (from, to, amount) = (from as u32, to as u32, amount as u32);
            ((from * (255 - amount) + to * amount + 127) / 255) as u8
        };

        Rgb(blend(self.0, other.0), blend(self.1, other.1), blend(self.2, other.2))
    }

    /// Perceived brightness, 0 to 255, which is what decides whether a theme is light or dark.
    pub fn luma(self) -> u8 {
        ((self.0 as u32 * 299 + self.1 as u32 * 587 + self.2 as u32 * 114) / 1000) as u8
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_what_is_not_hex() {
        for bad in ["", "#", "#12", "#1234", "#gggggg", "#12345678"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        let color = Rgb(1, 171, 255);
        assert_eq!(color.to_string(), "#01abff");
        assert_eq!(Rgb::parse(&color.to_string()), Some(color));
    }

    #[test]
    fn luma_tells_light_from_dark() {
        assert!(Rgb(255, 255, 255).luma() > 250);
        assert!(Rgb(40, 42, 54).luma() < 60);
    }
}
