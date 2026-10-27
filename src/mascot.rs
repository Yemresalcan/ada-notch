//! The notch mascot: after the user has been idle for a while the collapsed
//! notch shows a little character (colored glow, two eyes, a waving hand)
//! instead of the clock. It first writes "Merhaba" in handwriting, then the
//! eyes appear; while asleep a 3D waving-hand emoji pops up and waves.
//!
//! Moods: Greeting ("Merhaba") -> LookingAround -> Sleeping (waves) ->
//! Waking (on input, looks at the cursor) -> Hidden.

use crate::anim::Spring;
use crate::gfx::{Color, DrawCtx, Point, Rect, TextStyle};
use std::f32::consts::PI;

/// Fluent Emoji 3D "waving hand" (MIT), 96x96 RGBA.
const HAND_PNG: &[u8] = include_bytes!("../assets/waving_hand_96.png");

/// Idle time (ms) before the mascot shows up.
const IDLE_TRIGGER_MS: u32 = 15_000;
/// How long the eyes wander around before falling asleep.
const LOOK_SECS: f32 = 5.5;
/// How long the "user is back" reaction lasts.
const WAKE_SECS: f32 = 5.2;
/// Greeting: "Merhaba" is written out, held, then faded away.
const GREET_WRITE_SECS: f32 = 1.6;
const GREET_HOLD_SECS: f32 = 1.0;
const GREET_FADE_SECS: f32 = 0.5;
const GREET_TOTAL_SECS: f32 = GREET_WRITE_SECS + GREET_HOLD_SECS + GREET_FADE_SECS;
/// While sleeping the hand waves for the first `HAND_WAVE_SECS` of every cycle.
const HAND_CYCLE_SECS: f32 = 4.0;
const HAND_WAVE_SECS: f32 = 1.8;
const HAND_POP_IN_SECS: f32 = 0.25;
const HAND_POP_OUT_SECS: f32 = 0.2;
/// Swings per second and peak angle (degrees) of the wave.
const HAND_WAVE_HZ: f32 = 2.6;
const HAND_WAVE_DEG: f32 = 20.0;
/// Emoji size and offset of its left edge from the notch center (DIPs).
const HAND_SIZE: f32 = 17.0;
const HAND_DX: f32 = 19.0;
/// Blink length and random gap between blinks.
const BLINK_SECS: f32 = 0.15;
const BLINK_GAP_MIN: f32 = 1.8;
const BLINK_GAP_RANGE: f32 = 2.5;

/// Eye geometry (DIPs). These are ellipse radii, as in the
/// `D2D1_ELLIPSE` (rx 4.2, ry up to 6.2, minimum 0.9).
const EYE_DX: f32 = 11.0;
const EYE_RX: f32 = 4.2;
const EYE_RY_MAX: f32 = 6.2;
const EYE_RY_MIN: f32 = 0.9;
/// Eye line sits slightly below the notch center.
const EYE_Y_OFFSET: f32 = 1.0;
/// Radius of the notch's bottom corners (collapsed), used to keep the glow
/// inside the black silhouette.
const NOTCH_BOTTOM_RADIUS: f32 = 14.0;

const GLOW_LEFT: Color = Color::rgba(1.0, 0.33, 0.35, 1.0); // #FF5459
const GLOW_RIGHT: Color = Color::rgba(0.784, 0.063, 0.180, 1.0); // #C8102E (Turkish red)
const EYE_HALO: Color = Color::rgba(1.0, 0.86, 0.86, 1.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mood {
    Hidden,
    Greeting,
    LookingAround,
    Sleeping,
    Waking,
}

pub struct Mascot {
    mood: Mood,
    /// True when `Hidden` was entered from `Greeting` (no eyes to fade out).
    hidden_from_greeting: bool,
    /// Seconds spent in the current mood.
    timer: f32,
    /// Free-running clock for shimmer / breathing.
    time: f32,
    last_idle_ms: u32,
    vis: Spring,
    open: Spring,
    /// 0 = round eye, 1 = happy "^" arc.
    happy: Spring,
    gaze_x: Spring,
    gaze_y: Spring,
    bounce: Spring,
    /// Seconds until the next blink.
    blink_cd: f32,
    /// 0..1 progress of a running blink, negative when not blinking.
    blink_p: f32,
    rng: u32,
}

impl Mascot {
    pub fn new() -> Self {
        Self {
            mood: Mood::Hidden,
            hidden_from_greeting: false,
            timer: 0.0,
            time: 0.0,
            last_idle_ms: 0,
            vis: Spring::new(0.0, 240.0, 30.0),
            open: Spring::new(0.9, 260.0, 18.0),
            happy: Spring::new(0.0, 200.0, 24.0),
            gaze_x: Spring::new(0.0, 140.0, 18.0),
            gaze_y: Spring::new(0.0, 140.0, 18.0),
            bounce: Spring::new(1.0, 300.0, 14.0),
            blink_cd: 2.5,
            blink_p: -1.0,
            rng: 0x2545_F491,
        }
    }

    /// Advance by `dt` seconds. `idle_ms`: ms since last user input (GetLastInputInfo).
    /// `eligible`: mascot enabled AND notch collapsed AND nothing else shown AND battery ok.
    /// `cursor`: cursor position relative to the notch center, in DIPs.
    /// Returns true while it needs animation frames (anything moving / visible-and-animating).
    pub fn update(&mut self, dt: f32, idle_ms: u32, eligible: bool, cursor: Point) -> bool {
        // Springs are integrated explicitly: keep the step small and stable.
        // Non-finite inputs would poison every spring permanently.
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.05) } else { 0.0 };
        let cursor = if cursor.x.is_finite() && cursor.y.is_finite() { cursor } else { Point::default() };
        // Wrap the clocks so f32 precision never degrades over long sessions
        // (one-hour period; the sleeping timer only matters modulo the hand cycle).
        self.time = (self.time + dt) % 3600.0;
        self.timer += dt;
        if self.mood == Mood::Sleeping && self.timer > 3600.0 {
            self.timer %= HAND_CYCLE_SECS;
        }

        // User input = idle time restarted (dropped since the last call) or is tiny.
        let input = idle_ms < 1000 || idle_ms.saturating_add(250) < self.last_idle_ms;
        self.last_idle_ms = idle_ms;

        if !eligible {
            if self.mood != Mood::Hidden {
                self.set_mood(Mood::Hidden);
            }
        } else {
            match self.mood {
                Mood::Hidden => {
                    if idle_ms >= IDLE_TRIGGER_MS {
                        self.set_mood(Mood::Greeting);
                    }
                }
                Mood::Greeting => {
                    if input {
                        self.set_mood(Mood::Hidden);
                    } else if self.timer > GREET_TOTAL_SECS {
                        self.set_mood(Mood::LookingAround);
                    }
                }
                Mood::LookingAround => {
                    if input {
                        self.set_mood(Mood::Waking);
                    } else if self.timer > LOOK_SECS {
                        self.set_mood(Mood::Sleeping);
                    }
                }
                Mood::Sleeping => {
                    if input {
                        self.set_mood(Mood::Waking);
                    }
                }
                Mood::Waking => {
                    if self.timer > WAKE_SECS {
                        self.set_mood(Mood::Hidden);
                    }
                }
            }
        }

        self.apply_targets(cursor);
        self.update_blink(dt);

        let mut moving = self.vis.step(dt);
        moving |= self.open.step(dt);
        moving |= self.happy.step(dt);
        moving |= self.gaze_x.step(dt);
        moving |= self.gaze_y.step(dt);
        moving |= self.bounce.step(dt);

        // Anything but Hidden animates continuously (wander, breathing glow,
        // shimmer, waving hand); once hidden we only need the fade-out.
        self.mood != Mood::Hidden || self.vis.value > 0.0005 || (moving && self.vis.target > 0.0)
    }

    /// 0..=1 visibility for crossfading the normal collapsed content.
    pub fn visibility(&self) -> f32 {
        self.vis.value.clamp(0.0, 1.0)
    }

    /// Draw inside `notch` (the collapsed notch body rect in window DIPs, ~150x30). No-op when invisible.
    pub fn draw(&self, ctx: &mut DrawCtx, notch: Rect) {
        let vis = self.visibility();
        if vis < 0.01 {
            return;
        }
        let center = notch.center();
        let cx = center.x;
        let cy = center.y + EYE_Y_OFFSET;

        ctx.push_clip(notch);
        self.draw_glow(ctx, notch, vis);

        if self.mood == Mood::Greeting {
            self.draw_greeting(ctx, center, vis);
        } else if !(self.mood == Mood::Hidden && self.hidden_from_greeting) {
            // Eyes (with the little jump when waking up).
            let bounce = self.bounce.value;
            let jump = -(bounce - 1.0) * 10.0;
            ctx.set_transform(bounce, Point { x: cx, y: cy }, Point { x: 0.0, y: jump });
            self.draw_eyes(ctx, cx, cy, vis);
            ctx.reset_transform();
        }

        if self.mood == Mood::Sleeping {
            self.draw_hand(ctx, cx, cy, vis);
        }
        ctx.pop_clip();
    }

    // ---- state ----------------------------------------------------------

    fn set_mood(&mut self, mood: Mood) {
        self.hidden_from_greeting = mood == Mood::Hidden && self.mood == Mood::Greeting;
        self.mood = mood;
        self.timer = 0.0;
        match mood {
            Mood::LookingAround => {
                // The eyes open up from a thin line once the greeting is done.
                self.open.snap(0.0);
                self.happy.snap(0.0);
                self.gaze_x.snap(0.0);
                self.gaze_y.snap(0.0);
                self.bounce.snap(1.0);
                self.blink_p = -1.0;
                self.blink_cd = BLINK_GAP_MIN + self.rand01() * BLINK_GAP_RANGE;
            }
            Mood::Waking => {
                self.blink_p = -1.0;
                self.blink_cd = BLINK_GAP_MIN + self.rand01() * BLINK_GAP_RANGE;
            }
            Mood::Greeting | Mood::Sleeping | Mood::Hidden => {}
        }
    }

    /// Per-mood spring targets.
    fn apply_targets(&mut self, cursor: Point) {
        let t = self.timer;
        match self.mood {
            Mood::Hidden => {
                self.vis.target = 0.0;
            }
            Mood::Greeting => {
                self.vis.target = 1.0;
                self.open.target = 0.0;
                self.happy.target = 0.0;
                self.gaze_x.target = 0.0;
                self.gaze_y.target = 0.0;
                self.bounce.target = 1.0;
            }
            Mood::LookingAround => {
                self.vis.target = 1.0;
                self.open.target = 0.95 + 0.05 * (t * 3.0).sin();
                self.happy.target = 0.0;
                self.gaze_x.target = (t * 1.8).sin() * 3.5;
                self.gaze_y.target = (t * 2.6).sin() * 0.5;
                self.bounce.target = 1.0;
            }
            Mood::Sleeping => {
                self.vis.target = 1.0;
                self.open.target = 0.12;
                self.happy.target = 1.0;
                self.gaze_x.target = 0.0;
                self.gaze_y.target = 0.0;
                self.bounce.target = 1.0;
            }
            Mood::Waking => {
                self.vis.target = 1.0;
                self.open.target = 1.0;
                self.happy.target = 0.0;
                self.gaze_x.target = (cursor.x / 60.0).clamp(-3.5, 3.5);
                self.gaze_y.target = (cursor.y / 80.0).clamp(-1.0, 2.0);
                self.bounce.target = if t < 0.3 { 1.15 } else { 1.0 };
            }
        }
    }

    fn update_blink(&mut self, dt: f32) {
        let awake = matches!(self.mood, Mood::LookingAround | Mood::Waking);
        if !awake {
            self.blink_p = -1.0;
            return;
        }
        if self.blink_p >= 0.0 {
            self.blink_p += dt / BLINK_SECS;
            if self.blink_p >= 1.0 {
                self.blink_p = -1.0;
                self.blink_cd = BLINK_GAP_MIN + self.rand01() * BLINK_GAP_RANGE;
            }
        } else {
            self.blink_cd -= dt;
            if self.blink_cd <= 0.0 {
                self.blink_p = 0.0;
            }
        }
    }

    /// Tiny LCG (same constants as the original), uniform in 0..=1.
    fn rand01(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(0x41C6_4E6D).wrapping_add(0x3039);
        ((self.rng >> 8) & 0xFFFF) as f32 / 65535.0
    }

    // ---- drawing --------------------------------------------------------

    /// Glow alpha per mood, before visibility.
    fn glow_alpha(&self) -> f32 {
        match self.mood {
            Mood::Waking => (0.85 + 0.15 * (self.time * 3.0).sin()).min(1.0),
            Mood::Sleeping => 0.9,
            Mood::Greeting => 0.8,
            Mood::LookingAround => 0.75 + 0.12 * (self.time * 2.5).sin(),
            Mood::Hidden => 0.7,
        }
    }

    /// Horizontal teal -> violet gradient with a shimmer, faded to black
    /// toward the top by a vertical black overlay.
    fn draw_glow(&self, ctx: &mut DrawCtx, notch: Rect, vis: f32) {
        let g = (self.glow_alpha() * vis).clamp(0.0, 1.0);
        let shimmer = 0.12 * notch.w * (self.time * 1.4).sin();
        // Extend above the notch so the rounded top corners are clipped away;
        // the bottom corners match the notch silhouette.
        let r = NOTCH_BOTTOM_RADIUS;
        let rect = Rect::new(notch.x, notch.y - r, notch.w, notch.h + r);
        ctx.fill_rect_gradient(
            rect,
            r,
            Point { x: notch.x + shimmer, y: 0.0 },
            Point { x: notch.right() + shimmer, y: 0.0 },
            &[(0.0, GLOW_LEFT.with_alpha(g)), (1.0, GLOW_RIGHT.with_alpha(g))],
        );
        let black = Color::rgba(0.0, 0.0, 0.0, 1.0);
        ctx.fill_rect_gradient(
            rect,
            r,
            Point { x: 0.0, y: notch.y },
            Point { x: 0.0, y: notch.bottom() },
            &[
                (0.0, black),
                (0.45, black.with_alpha(0.6)),
                (1.0, black.with_alpha(0.0)),
            ],
        );
    }

    fn draw_eyes(&self, ctx: &mut DrawCtx, cx: f32, cy: f32, vis: f32) {
        let blink = if self.blink_p >= 0.0 { 1.0 - (self.blink_p * PI).sin() } else { 1.0 };
        let open = self.open.value.clamp(0.0, 1.2) * blink;
        let ry = (open * EYE_RY_MAX).max(EYE_RY_MIN);
        let norm = (ry / EYE_RY_MAX).clamp(0.0, 1.0);
        let happy = self.happy.value.clamp(0.0, 1.0);
        let ey = cy + self.gaze_y.value;
        // Keep a soft halo around the closed "^" eyes too.
        let halo_k = (0.35 + 0.65 * norm.max(self.happy.value * 0.8)) * vis;

        for sign in [-1.0f32, 1.0] {
            let ex = cx + sign * EYE_DX + self.gaze_x.value;
            let c = Point { x: ex, y: ey };

            // Soft glow behind the eye.
            ctx.glow_ellipse(
                c,
                2.4 * EYE_RX,
                2.0 + 1.9 * ry,
                EYE_HALO.with_alpha(0.5 * halo_k),
                EYE_HALO.with_alpha(0.0),
            );

            if happy < 0.99 {
                ctx.fill_ellipse(c, EYE_RX, ry, Color::rgba(1.0, 1.0, 1.0, (1.0 - happy) * vis));
            }
            if happy > 0.01 {
                // "^" as the top of a circle: chord == eye width, apex a bit above the eye line.
                // Peaked "^": ~10 DIP wide, ~3 DIP tall, thick stroke.
                let r = 5.6 * (0.8 + 0.2 * happy);
                let arc_center = Point { x: ex, y: ey - 2.2 + r };
                ctx.stroke_arc(arc_center, r, 205.0, 130.0, Color::rgba(1.0, 1.0, 1.0, happy * vis), 3.2);
            }
        }
    }

    /// "Merhaba" written out in handwriting (left to right, like a pen), held, then faded.
    fn draw_greeting(&self, ctx: &mut DrawCtx, center: Point, vis: f32) {
        let t = self.timer;
        let write = (t / GREET_WRITE_SECS).clamp(0.0, 1.0);
        let reveal = write * write * (3.0 - 2.0 * write); // smoothstep
        let fade = 1.0 - ((t - GREET_WRITE_SECS - GREET_HOLD_SECS) / GREET_FADE_SECS).clamp(0.0, 1.0);
        let alpha = (vis * fade).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }

        let style = TextStyle::script(15.0, Color::rgba(1.0, 1.0, 1.0, alpha));
        let w = ctx.measure_text("Merhaba", &style) + 4.0;
        let rect = Rect::new(center.x - w / 2.0, center.y - 12.0, w, 24.0);
        // The clip grows from slightly left of the text to slightly right of it.
        let edge = reveal * (w + 4.0);
        ctx.push_clip(Rect::new(rect.x - 2.0, rect.y - 4.0, edge, rect.h + 8.0));
        ctx.text("Merhaba", rect, style);
        ctx.pop_clip();

        // Tiny bright pen dot at the writing edge.
        if write < 1.0 {
            let pen = Point { x: rect.x - 2.0 + edge, y: center.y + 3.0 };
            ctx.fill_ellipse(pen, 1.2, 1.2, Color::rgba(1.0, 1.0, 1.0, 0.8 * alpha));
        }
    }

    /// Fluent 3D waving-hand emoji right of the eyes, popping in and waving while sleeping.
    fn draw_hand(&self, ctx: &mut DrawCtx, cx: f32, cy: f32, vis: f32) {
        let phase = self.timer.rem_euclid(HAND_CYCLE_SECS);
        if phase >= HAND_WAVE_SECS {
            return;
        }

        // Pop in with a little overshoot (back-out), pop out linearly at the end.
        const BACK_C1: f32 = 1.701_58;
        let x = (phase / HAND_POP_IN_SECS).clamp(0.0, 1.0) - 1.0;
        let pop_in = 1.0 + (BACK_C1 + 1.0) * x * x * x + BACK_C1 * x * x;
        let pop_out = ((HAND_WAVE_SECS - phase) / HAND_POP_OUT_SECS).clamp(0.0, 1.0);
        let scale = pop_in.min(pop_out);
        if scale < 0.01 {
            return;
        }

        let left = cx + HAND_DX;
        let top = cy - HAND_SIZE / 2.0;
        // The wrist is the emoji's bottom-left area.
        let pivot = Point { x: left + 0.30 * HAND_SIZE, y: top + 0.85 * HAND_SIZE };
        let angle = HAND_WAVE_DEG * (2.0 * PI * HAND_WAVE_HZ * phase).sin();

        ctx.set_transform_rotated(scale, angle, pivot, Point { x: 0.0, y: 0.0 });
        ctx.image(HAND_PNG, Rect::new(left, top, HAND_SIZE, HAND_SIZE), vis);
        ctx.reset_transform();
    }
}
