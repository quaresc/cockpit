//! What moves: a step turning done (a spinner, then a teal flash fading out through tachyonfx, a spark for the first
//! second), and values that glide to their new place, the progress gauge, the scroll and the tab underline.

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tachyonfx::{ColorSpace, Effect, EffectTimer, Interpolation, fx};

/// Each done step starts CASCADE after the previous; it spins for SPIN, then sparks until SPARK and fades until FLASH.
pub const CASCADE: f32 = 0.3;
const SPIN: f32 = 0.5;
const SPIN_FRAME: f32 = 0.07;
const SPARK: f32 = 1.5;
const FLASH: f32 = 2.6;
const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
const FLASH_FROM: Color = Color::Rgb(40, 100, 80);

pub enum Phase {
    /// Not started yet, or over: the row as it is.
    Still,
    Spin(char),
    Spark,
}

/// One row playing its done animation.
pub struct RowFx {
    start: Instant,
    flash: Option<(Effect, Instant)>,
}

impl RowFx {
    pub fn new(delay: f32) -> Self {
        Self { start: Instant::now() + Duration::from_secs_f32(delay), flash: None }
    }

    fn t(&self) -> f32 {
        let now = Instant::now();
        if now < self.start { -(self.start - now).as_secs_f32() } else { (now - self.start).as_secs_f32() }
    }

    pub fn phase(&self) -> Phase {
        match self.t() {
            t if (0.0..SPIN).contains(&t) => Phase::Spin(SPINNER[(t / SPIN_FRAME) as usize % SPINNER.len()]),
            t if (SPIN..SPARK).contains(&t) => Phase::Spark,
            _ => Phase::Still,
        }
    }

    pub fn is_over(&self) -> bool {
        self.t() >= FLASH
    }

    /// Tints the row's cells, drawn already, from teal back to their own background with an ease-out.
    pub fn paint(&mut self, buf: &mut Buffer, area: Rect) {
        if self.t() < SPIN {
            return;
        }
        let now = Instant::now();
        let (effect, last) = self.flash.get_or_insert_with(|| {
            let timer = EffectTimer::from_ms(((FLASH - SPIN) * 1000.0) as u32, Interpolation::CubicOut);
            let fade = fx::effect_fn((), timer, |_, ctx, cells| {
                let alpha = ctx.alpha();
                for (_, cell) in cells {
                    let to = cell.bg;
                    cell.set_bg(ColorSpace::Rgb.lerp(&FLASH_FROM, &to, alpha));
                }
            });
            (fade, now)
        });
        let step = tachyonfx::Duration::from_millis((now - *last).as_millis() as u32);
        *last = now;
        effect.process(step, buf, area.intersection(buf.area));
    }
}

pub fn out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A number that eases from where it was to where it is going.
#[derive(Clone, Copy)]
pub struct Glide {
    from: f32,
    to: f32,
    at: Instant,
    secs: f32,
}

impl Glide {
    pub fn new(value: f32, secs: f32) -> Self {
        Self { from: value, to: value, at: Instant::now(), secs }
    }

    pub fn now(&self) -> f32 {
        let t = self.at.elapsed().as_secs_f32() / self.secs;
        self.from + (self.to - self.from) * out_cubic(t)
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn go(&mut self, to: f32) {
        if (to - self.to).abs() > f32::EPSILON {
            self.from = self.now();
            self.to = to;
            self.at = Instant::now();
        }
    }

    /// Straight there, no easing: a jump the eye should not follow.
    pub fn set(&mut self, to: f32) {
        self.from = to;
        self.to = to;
    }

    pub fn is_moving(&self) -> bool {
        self.at.elapsed().as_secs_f32() < self.secs && (self.to - self.from).abs() > f32::EPSILON
    }
}
