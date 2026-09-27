//! Two clocks: when things happened, and when the film shows them.
//!
//! Everything a recording captures is stamped with the real time it happened. Under tape timing
//! the film is on another clock: each step lasts what the tape says it does, however long it
//! really took. [`Clock`] holds one mark per step boundary — the real moment and the shown one —
//! and moves anything in between proportionally, since a step that took four seconds and is shown
//! for two has things happening all the way through it.

use std::time::Duration;

/// Something that happened at a moment.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Timed<T> {
    /// Since the recording started.
    pub at: Duration,
    pub value: T,
}

/// A real moment, and the moment the film shows it at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mark {
    real: Duration,
    shown: Duration,
}

/// Puts real moments on the film's clock.
#[derive(Clone, Debug)]
pub(crate) struct Clock {
    marks: Vec<Mark>,
}

impl Clock {
    /// A clock that starts with both at `start`.
    pub fn starting_at(start: Duration) -> Clock {
        Clock { marks: vec![Mark { real: start, shown: Duration::ZERO }] }
    }

    /// Ends a step at the real moment `real`, having lasted `shown` in the film.
    pub fn mark(&mut self, real: Duration, shown: Duration) {
        let last = *self.marks.last().expect("a clock starts with a mark");
        self.marks.push(Mark { real: real.max(last.real), shown: last.shown + shown });
    }

    /// The real moment of the last mark.
    pub fn last_real(&self) -> Duration {
        self.marks.last().expect("a clock starts with a mark").real
    }

    /// How long the film is so far.
    pub fn length(&self) -> Duration {
        self.marks.last().expect("a clock starts with a mark").shown
    }

    /// When the film shows what happened at `real`.
    ///
    /// Anything before the first mark is shown at the start, and anything after the last at the
    /// end: what the screen held when filming began, and what it held when it stopped.
    pub fn shown(&self, real: Duration) -> Duration {
        let next = self.marks.partition_point(|mark| mark.real <= real);
        if next == 0 {
            return Duration::ZERO;
        }
        let Some(to) = self.marks.get(next) else {
            return self.length();
        };
        let from = self.marks[next - 1];
        let span = (to.real - from.real).as_secs_f64();
        let part = if span == 0.0 { 1.0 } else { (real - from.real).as_secs_f64() / span };

        from.shown + (to.shown - from.shown).mul_f64(part)
    }

    /// The same events, on the film's clock.
    pub fn place<T>(&self, events: Vec<Timed<T>>) -> Vec<Timed<T>> {
        events.into_iter().map(|event| Timed { at: self.shown(event.at), value: event.value }).collect()
    }
}

/// When each event happened, without what it was.
pub(crate) fn moments<T>(track: &[Timed<T>]) -> Vec<Timed<()>> {
    track.iter().map(|event| Timed { at: event.at, value: () }).collect()
}

/// The last event at or before `at`, by index — what a track shows at that moment.
pub(crate) fn showing<T>(track: &[Timed<T>], at: Duration) -> Option<usize> {
    track.partition_point(|event| event.at <= at).checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn a_slow_step_is_squeezed_into_what_the_tape_says() {
        let mut clock = Clock::starting_at(ms(0));
        clock.mark(ms(9_000), ms(1_000));

        assert_eq!(clock.shown(ms(4_500)), ms(500));
        assert_eq!(clock.length(), ms(1_000));
    }

    #[test]
    fn a_fast_step_is_stretched() {
        let mut clock = Clock::starting_at(ms(100));
        clock.mark(ms(200), ms(1_000));
        clock.mark(ms(1_200), ms(1_000));

        assert_eq!(clock.shown(ms(150)), ms(500));
        assert_eq!(clock.shown(ms(700)), ms(1_500));
    }

    #[test]
    fn before_the_start_is_the_start_and_after_the_end_is_the_end() {
        let mut clock = Clock::starting_at(ms(1_000));
        clock.mark(ms(2_000), ms(500));

        assert_eq!(clock.shown(ms(0)), ms(0));
        assert_eq!(clock.shown(ms(5_000)), ms(500));
    }

    #[test]
    fn a_step_that_took_no_time_jumps() {
        let mut clock = Clock::starting_at(ms(0));
        clock.mark(ms(100), ms(100));
        clock.mark(ms(100), ms(0));
        clock.mark(ms(300), ms(200));

        assert_eq!(clock.shown(ms(100)), ms(100));
        assert_eq!(clock.shown(ms(200)), ms(200));
    }

    #[test]
    fn marks_never_go_back_in_real_time() {
        let mut clock = Clock::starting_at(ms(500));
        clock.mark(ms(100), ms(50));
        assert_eq!(clock.shown(ms(500)), ms(50));
    }

    #[test]
    fn place_moves_every_event() {
        let mut clock = Clock::starting_at(ms(0));
        clock.mark(ms(1_000), ms(2_000));
        let placed = clock.place(vec![Timed { at: ms(250), value: 'a' }, Timed { at: ms(750), value: 'b' }]);

        assert_eq!(placed.iter().map(|event| event.at).collect::<Vec<_>>(), [ms(500), ms(1_500)]);
    }

    #[test]
    fn showing_is_the_last_event_not_after() {
        let track =
            [Timed { at: ms(0), value: () }, Timed { at: ms(100), value: () }, Timed { at: ms(100), value: () }];

        assert_eq!(showing(&track, ms(50)), Some(0));
        assert_eq!(showing(&track, ms(100)), Some(2));
        assert_eq!(showing(&track[1..], ms(50)), None);
    }
}
