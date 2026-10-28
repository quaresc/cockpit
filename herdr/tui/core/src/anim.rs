//! Status changes made visible: a row flashing a colour that fades back, a pulse, a sweep in from the left; and values
//! that glide to their new place. Effects are keyed by what they show (a PR URL, a step), so they follow it when the
//! list moves, and each app draws them over the area that thing has in the frame.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tachyonfx::{ColorSpace, Effect, EffectTimer, Interpolation, fx};

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// The background starts at this colour and fades back over `secs`.
    Flash(Color, f32),
    /// The background beats to this colour and back, `beats` times in `secs`.
    Pulse(Color, u32, f32),
    /// The content sweeps in from the left over `secs`.
    SweepIn(f32),
}

impl Kind {
    pub const BROKEN: Kind = Kind::Flash(Color::Rgb(110, 40, 40), 2.4);
    pub const GOOD: Kind = Kind::Flash(Color::Rgb(40, 100, 80), 2.4);
    pub const NEW: Kind = Kind::SweepIn(0.45);
    pub const DONE: Kind = Kind::Pulse(Color::Rgb(70, 60, 40), 2, 1.6);

    fn secs(&self) -> f32 {
        match *self {
            Kind::Flash(_, s) | Kind::Pulse(_, _, s) | Kind::SweepIn(s) => s,
        }
    }

    fn effect(self) -> Effect {
        let timer = EffectTimer::from_ms((self.secs() * 1000.0) as u32, Interpolation::Linear);
        match self {
            Kind::Flash(from, _) => fx::effect_fn((), EffectTimer::from_ms((self.secs() * 1000.0) as u32, Interpolation::CubicOut), move |_, ctx, cells| {
                let alpha = ctx.alpha();
                for (_, cell) in cells {
                    let to = cell.bg;
                    cell.set_bg(ColorSpace::Rgb.lerp(&from, &to, alpha));
                }
            }),
            Kind::Pulse(to, beats, _) => fx::effect_fn((), timer, move |_, ctx, cells| {
                let wave = (ctx.alpha() * beats as f32 * std::f32::consts::PI).sin().abs();
                for (_, cell) in cells {
                    let from = cell.bg;
                    cell.set_bg(ColorSpace::Rgb.lerp(&from, &to, wave));
                }
            }),
            Kind::SweepIn(_) => fx::effect_fn((), EffectTimer::from_ms((self.secs() * 1000.0) as u32, Interpolation::CubicOut), |_, ctx, cells| {
                let edge = ctx.alpha();
                let area = ctx.area;
                for (pos, cell) in cells {
                    let x = (pos.x - area.x) as f32 / area.width.max(1) as f32;
                    if x > edge {
                        cell.set_symbol(" ");
                    }
                }
            }),
        }
    }
}

/// The colour a status kind flashes with: red for broken, teal for good, orange for anything new.
pub fn tone(kind: &str) -> Kind {
    match kind {
        "broken" => Kind::BROKEN,
        "good" => Kind::GOOD,
        "new" => Kind::NEW,
        _ => Kind::Flash(ORANGE_FLASH, 2.0),
    }
}

const ORANGE_FLASH: Color = Color::Rgb(90, 70, 50);

#[derive(Default)]
pub struct Effects {
    running: HashMap<String, (Effect, Instant, Instant)>,
}

impl Effects {
    /// Starts an effect on this key, replacing one already running there.
    pub fn start(&mut self, key: &str, kind: Kind) {
        let now = Instant::now();
        self.running.insert(key.to_string(), (kind.effect(), now, now + Duration::from_secs_f32(kind.secs())));
    }

    /// Draws the key's effect over the area its thing has in this frame; nothing when none runs.
    pub fn paint(&mut self, key: &str, buf: &mut Buffer, area: Rect) {
        let now = Instant::now();
        if let Some((effect, last, _)) = self.running.get_mut(key) {
            let step = tachyonfx::Duration::from_millis((now - *last).as_millis() as u32);
            *last = now;
            effect.process(step, buf, area.intersection(buf.area));
        }
    }

    /// True while any effect runs, for the loop to tick fast; drops the finished ones.
    pub fn active(&mut self) -> bool {
        let now = Instant::now();
        self.running.retain(|_, (_, _, end)| *end > now);
        !self.running.is_empty()
    }
}

pub fn out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A number that eases from where it was to where it is going.
#[derive(Clone, Copy, Debug)]
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
