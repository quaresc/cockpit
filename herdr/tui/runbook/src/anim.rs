//! A step turning done: a spinner in its status cell, then a spark beside it while core's teal flash fades the row; each
//! step starting CASCADE after the previous. Time comes from a clock tests can stop.

use std::time::{Duration, Instant};

/// Each done step starts CASCADE after the previous; it spins for SPIN, then sparks until SPARK.
pub const CASCADE: f32 = 0.3;
const SPIN: f32 = 0.5;
const SPIN_FRAME: f32 = 0.07;
const SPARK: f32 = 1.5;
pub const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];

/// The time the animations read, and the day a step is marked done on; both can be fixed for a snapshot.
#[derive(Clone, Debug, Default)]
pub struct Clock {
    frozen: Option<Instant>,
    day: Option<String>,
}

impl Clock {
    pub fn now(&self) -> Instant {
        self.frozen.unwrap_or_else(Instant::now)
    }

    /// Stops time at this instant, for a test to look at one frame of an animation.
    pub fn freeze(&mut self, at: Instant) {
        self.frozen = Some(at);
    }

    /// The day as `runbook.mark_done` writes it, "28 Oct".
    pub fn today(&self) -> String {
        self.day.clone().unwrap_or_else(|| chrono::Local::now().format("%d %b").to_string())
    }

    pub fn set_today(&mut self, day: &str) {
        self.day = Some(day.into());
    }
}

pub enum Phase {
    /// Over, or not playing: the row as it is.
    Still,
    /// Its turn in the cascade not come yet.
    Waiting,
    Spin(char),
    Spark,
}

/// One row's place in its done animation.
pub struct RowFx {
    start: Instant,
    /// Whether core's flash was started on the row, once its spin ended.
    pub flashed: bool,
}

impl RowFx {
    pub fn new(now: Instant, delay: f32) -> Self {
        Self { start: now + Duration::from_secs_f32(delay), flashed: false }
    }

    fn t(&self, now: Instant) -> f32 {
        if now < self.start { -(self.start - now).as_secs_f32() } else { (now - self.start).as_secs_f32() }
    }

    pub fn phase(&self, now: Instant) -> Phase {
        match self.t(now) {
            t if t < 0.0 => Phase::Waiting,
            t if (0.0..SPIN).contains(&t) => Phase::Spin(SPINNER[(t / SPIN_FRAME) as usize % SPINNER.len()]),
            t if (SPIN..SPARK).contains(&t) => Phase::Spark,
            _ => Phase::Still,
        }
    }

    /// True once the spin is over and the flash not started yet.
    pub fn wants_flash(&self, now: Instant) -> bool {
        !self.flashed && self.t(now) >= SPIN
    }

    pub fn is_over(&self, now: Instant) -> bool {
        self.t(now) >= SPARK
    }
}
