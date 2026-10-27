//! Pomodoro focus timer with collapsed live activity and daily session count.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::gfx::{Align, Color, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::{Activity, Host, Mouse, Page};
use crate::store;
use crate::theme;
use crate::worker::{self, Peek, Waker};

const FILE: &str = "focus.txt";
const FOCUS_SECS: u64 = 25 * 60;
const SHORT_BREAK_SECS: u64 = 5 * 60;
const LONG_BREAK_SECS: u64 = 15 * 60;
const SESSIONS_PER_CYCLE: u32 = 4;

const ACTIVITY_PRIORITY: u8 = 20;
const ACTIVITY_WIDTH: f32 = 200.0;
const PEEK_SECONDS: f32 = 6.0;
const TOMATO: &str = "\u{1F345}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Phase::Focus => FOCUS_SECS,
            Phase::ShortBreak => SHORT_BREAK_SECS,
            Phase::LongBreak => LONG_BREAK_SECS,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus time",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus",
            Phase::ShortBreak | Phase::LongBreak => "Break",
        }
    }

    fn ring_color(self) -> Color {
        match self {
            Phase::Focus => theme::ACCENT,
            Phase::ShortBreak | Phase::LongBreak => theme::BLUE,
        }
    }

    fn peek_info(self) -> (&'static str, &'static str, Color) {
        match self {
            Phase::Focus => ("Focus Time!", "Back to work, 25 min", theme::DANGER),
            Phase::ShortBreak => ("Break Time!", "5 minute short break", theme::ACCENT),
            Phase::LongBreak => ("Long Break!", "15 min, you earned it", theme::ACCENT),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum State {
    Idle,
    Running { phase: Phase, ends_at: Instant },
    Paused { phase: Phase, remaining: Duration },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalDate {
    y: u16,
    m: u16,
    d: u16,
}

impl LocalDate {
    fn now() -> Self {
        let st = unsafe { GetLocalTime() };
        Self { y: st.wYear, m: st.wMonth, d: st.wDay }
    }

    fn format(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Primary = 0,
    Reset = 1,
    Skip = 2,
}

const ALL_BTNS: [Btn; 3] = [Btn::Primary, Btn::Reset, Btn::Skip];

struct Layout {
    left: Rect,
    buttons: [Rect; 3],
    info: Rect,
}

fn layout(area: Rect) -> Layout {
    let rx = area.x + 102.0;
    let rw = (area.right() - rx - 4.0).max(0.0);
    let btn_h = 26.0;
    let gap = 8.0;
    let btn_y = area.y + 16.0;
    let avail = (rw - gap * 2.0).max(0.0);
    let w0 = (avail * 0.36).round();
    let w1 = ((avail - w0) * 0.5).round();
    let w2 = (avail - w0 - w1).max(0.0);
    let b0 = Rect::new(rx, btn_y, w0, btn_h);
    let b1 = Rect::new(b0.right() + gap, btn_y, w1, btn_h);
    let b2 = Rect::new(b1.right() + gap, btn_y, w2, btn_h);
    Layout {
        left: Rect::new(area.x, area.y, 88.0, area.h),
        buttons: [b0, b1, b2],
        info: Rect::new(rx, btn_y + btn_h + 10.0, rw, 18.0),
    }
}

fn format_mm_ss(rem: Duration, max_secs: u64) -> String {
    let secs = rem.as_secs().saturating_add(u64::from(rem.subsec_nanos() > 0)).min(max_secs);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

pub struct FocusPage {
    waker: Waker,
    state: State,
    completed_focus: u32,
    done_today: u32,
    date: LocalDate,
    buttons: [Rect; 3],
    hover: Option<Btn>,
}

impl FocusPage {
    pub fn new(waker: Waker) -> Self {
        let date = LocalDate::now();
        let today_str = date.format();
        let kv = store::read_kv(FILE);
        let done_today = if kv.get("date").map(String::as_str) == Some(today_str.as_str()) {
            kv.get("done").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0)
        } else {
            0
        };
        let page = Self {
            waker,
            state: State::Idle,
            completed_focus: 0,
            done_today,
            date,
            buttons: [Rect::default(); 3],
            hover: None,
        };
        if kv.contains_key("date") && kv.get("date").map(String::as_str) != Some(today_str.as_str()) {
            page.save();
        }
        page
    }

    fn save(&self) {
        let mut map = BTreeMap::new();
        map.insert("date".to_string(), self.date.format());
        map.insert("done".to_string(), self.done_today.to_string());
        let _ = store::write_kv(FILE, &map);
    }

    fn sync_date(&mut self) -> bool {
        let now = LocalDate::now();
        if now == self.date {
            return false;
        }
        self.date = now;
        let changed = self.done_today != 0;
        self.done_today = 0;
        self.save();
        changed
    }

    fn next_phase(&mut self, current: Phase, count_done: bool) -> Phase {
        match current {
            Phase::Focus => {
                self.completed_focus = self.completed_focus.wrapping_add(1);
                if count_done {
                    self.sync_date();
                    self.done_today = self.done_today.saturating_add(1);
                    self.save();
                }
                if self.completed_focus % SESSIONS_PER_CYCLE == 0 {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            }
            Phase::ShortBreak | Phase::LongBreak => Phase::Focus,
        }
    }

    fn session_number(&self) -> u32 {
        match self.state {
            State::Idle | State::Running { phase: Phase::Focus, .. } | State::Paused { phase: Phase::Focus, .. } => {
                (self.completed_focus % SESSIONS_PER_CYCLE) + 1
            }
            State::Running { .. } | State::Paused { .. } => {
                (self.completed_focus.saturating_sub(1) % SESSIONS_PER_CYCLE) + 1
            }
        }
    }

    fn hit(&self, p: Point, area: Rect) -> Option<Btn> {
        let rects = if self.buttons[0].w > 0.0 {
            self.buttons
        } else {
            layout(area).buttons
        };
        ALL_BTNS.into_iter().find(|&b| rects[b as usize].contains(p))
    }

    fn on_click(&mut self, btn: Btn) -> bool {
        let was_active = self.activity().is_some();
        match btn {
            Btn::Primary => {
                self.state = match self.state {
                    State::Idle => State::Running {
                        phase: Phase::Focus,
                        ends_at: Instant::now() + Phase::Focus.duration(),
                    },
                    State::Running { phase, ends_at } => State::Paused {
                        phase,
                        remaining: ends_at.saturating_duration_since(Instant::now()),
                    },
                    State::Paused { phase, remaining } => State::Running {
                        phase,
                        ends_at: Instant::now() + remaining,
                    },
                };
            }
            Btn::Reset => {
                if matches!(self.state, State::Idle) {
                    return false;
                }
                self.state = State::Idle;
            }
            Btn::Skip => {
                self.state = match self.state {
                    State::Running { phase, .. } => {
                        let next = self.next_phase(phase, false);
                        State::Running { phase: next, ends_at: Instant::now() + next.duration() }
                    }
                    State::Paused { phase, .. } => {
                        let next = self.next_phase(phase, false);
                        State::Paused { phase: next, remaining: next.duration() }
                    }
                    // Nothing to skip before a session has started.
                    State::Idle => return false,
                };
            }
        }
        if self.activity().is_some() != was_active {
            self.waker.wake();
        }
        true
    }
}

impl Page for FocusPage {
    fn title(&self) -> String {
        "Focus".to_string()
    }

    fn tick(&mut self) -> bool {
        let mut redraw = self.sync_date();
        if let State::Running { phase, ends_at } = self.state {
            let now = Instant::now();
            if now >= ends_at {
                let next = self.next_phase(phase, true);
                // Chain from the scheduled end so the 1 s tick does not add drift;
                // after a long sleep start from now instead.
                let start = if now.saturating_duration_since(ends_at) < next.duration() { ends_at } else { now };
                self.state = State::Running { phase: next, ends_at: start + next.duration() };
                let (title, body, accent) = next.peek_info();
                worker::peek(Peek {
                    icon: TOMATO.into(),
                    emoji: true,
                    accent,
                    title: title.into(),
                    body: body.into(),
                    seconds: PEEK_SECONDS,
                });
                unsafe {
                    let _ = PlaySoundW(w!("SystemNotification"), None, SND_ALIAS | SND_ASYNC);
                }
            }
            redraw = true;
        }
        redraw
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let lay = layout(area);
        self.buttons = lay.buttons;

        let (rem, total, label, ring_color, primary_label) = match self.state {
            State::Idle => {
                let d = Phase::Focus.duration();
                (d, d, "Pomodoro", theme::ACCENT, "Start")
            }
            State::Running { phase, ends_at } => (
                ends_at.saturating_duration_since(Instant::now()),
                phase.duration(),
                phase.label(),
                phase.ring_color(),
                "Pause",
            ),
            State::Paused { phase, remaining } => (
                remaining,
                phase.duration(),
                phase.label(),
                theme::WARN,
                "Resume",
            ),
        };

        let fraction = (rem.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0);
        let center = Point { x: lay.left.x + lay.left.w / 2.0, y: area.y + 35.5 };
        ctx.stroke_circle(center, 32.0, ring_color.with_alpha(0.25), 5.0);
        ctx.arc(center, 32.0, fraction, ring_color, 5.0);

        let time_str = format_mm_ss(rem, total.as_secs());
        ctx.text(
            &time_str,
            Rect::new(center.x - 30.0, center.y - 13.0, 60.0, 26.0),
            TextStyle::new(18.0, Weight::Bold, theme::TEXT).align(Align::Center),
        );
        ctx.text(
            label,
            Rect::new(lay.left.x, area.y + 72.0, lay.left.w, 16.0),
            TextStyle::new(11.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
        );

        let labels = [primary_label, "Reset", "Skip"];
        for btn in ALL_BTNS {
            let idx = btn as usize;
            let rect = lay.buttons[idx];
            let hovered = self.hover == Some(btn);
            ctx.fill_rect(rect, 12.0, if hovered { theme::CARD_HOVER } else { theme::CARD });
            let weight = if btn == Btn::Primary { Weight::Semibold } else { Weight::Medium };
            ctx.text(
                labels[idx],
                rect,
                TextStyle::new(12.5, weight, theme::TEXT).align(Align::Center),
            );
        }

        let session_line = format!("Session {} of {} \u{00B7} {} done today", self.session_number(), SESSIONS_PER_CYCLE, self.done_today);
        ctx.text(
            &session_line,
            lay.info,
            TextStyle::new(11.5, Weight::Regular, theme::TEXT_DIM).align(Align::Center),
        );
    }

    fn on_mouse(&mut self, ev: Mouse, area: Rect, _host: &mut Host) -> bool {
        match ev {
            Mouse::Move(p) => {
                let hit = self.hit(p, area);
                let changed = hit != self.hover;
                self.hover = hit;
                changed
            }
            Mouse::Leave => self.hover.take().is_some(),
            Mouse::Down(p) | Mouse::DoubleClick(p) => {
                let hit = self.hit(p, area);
                self.hover = hit;
                match hit {
                    Some(btn) => self.on_click(btn),
                    None => false,
                }
            }
            Mouse::Up(_) | Mouse::Wheel(_, _) => false,
        }
    }

    fn activity(&self) -> Option<Activity> {
        match self.state {
            State::Idle => None,
            State::Running { .. } | State::Paused { .. } => {
                Some(Activity { priority: ACTIVITY_PRIORITY, width: ACTIVITY_WIDTH })
            }
        }
    }

    fn draw_activity(&mut self, ctx: &mut DrawCtx, notch: Rect) {
        let (phase, rem, paused) = match self.state {
            State::Idle => return,
            State::Running { phase, ends_at } => (phase, ends_at.saturating_duration_since(Instant::now()), false),
            State::Paused { phase, remaining } => (phase, remaining, true),
        };
        let total = phase.duration();
        let fraction = (rem.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0);
        let color = if paused { theme::WARN } else { phase.ring_color() };

        let center = Point { x: notch.x + 18.0, y: notch.h / 2.0 };
        ctx.stroke_circle(center, 7.0, color.with_alpha(0.25), 2.0);
        ctx.arc(center, 7.0, fraction, color, 2.0);

        let time_str = format_mm_ss(rem, total.as_secs());
        let time_style = TextStyle::new(13.0, Weight::Bold, theme::TEXT).align(Align::Right);
        let time_w = (ctx.measure_text(&time_str, &time_style) + 4.0).max(40.0);
        let time_x = (notch.right() - 14.0 - time_w).max(notch.x);
        ctx.text(&time_str, Rect::new(time_x, notch.y, time_w, notch.h), time_style);

        let mid_x = notch.x + 30.0;
        let mid_w = (time_x - 6.0 - mid_x).max(0.0);
        if mid_w > 16.0 {
            ctx.text(
                phase.short_label(),
                Rect::new(mid_x, notch.y, mid_w, notch.h),
                TextStyle::new(11.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
            );
        }
    }
}
