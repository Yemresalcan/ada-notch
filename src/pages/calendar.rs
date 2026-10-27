//! Calendar page: a personal local calendar. A week strip on top, the events of
//! the selected day below. Persisted in `calendar.txt` as
//! `YYYY-MM-DD|HH:MM|title` (empty time = all-day).

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::gfx::{Align, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::textfield::{Outcome, TextField};
use crate::pages::{Host, Mouse, Page};
use crate::store;
use crate::theme;
use crate::worker::Waker;

const FILE: &str = "calendar.txt";
const STRIP_H: f32 = 34.0;
const GAP: f32 = 4.0;
const LABEL_W: f32 = 52.0;
const ADD_H: f32 = 20.0;
const EV_H: f32 = 16.0;
const TIME_W: f32 = 46.0;
const DEL_W: f32 = 20.0;
const PILL_W: f32 = 30.0;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

// ---------------------------------------------------------------- dates ----

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// A calendar date; ordering is chronological.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Date {
    y: i32,
    m: u32,
    d: u32,
}

impl Date {
    /// Days since 1970-01-01 (proleptic Gregorian).
    fn to_days(self) -> i64 {
        let y = i64::from(self.y) - i64::from(self.m <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = i64::from((self.m + 9) % 12);
        let doy = (153 * mp + 2) / 5 + i64::from(self.d) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days(days: i64) -> Date {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = (yoe + era * 400) as i32 + i32::from(m <= 2);
        Date { y, m, d }
    }

    fn add_days(self, n: i64) -> Date {
        Date::from_days(self.to_days() + n)
    }

    /// Day of week, 0 = Sunday (Sakamoto's method).
    fn weekday(self) -> u32 {
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = if self.m < 3 { self.y - 1 } else { self.y };
        (y + y / 4 - y / 100 + y / 400 + T[(self.m - 1) as usize] + self.d as i32).rem_euclid(7) as u32
    }

    /// 0 = Monday ... 6 = Sunday.
    fn monday_index(self) -> u32 {
        (self.weekday() + 6) % 7
    }

    fn parse(s: &str) -> Option<Date> {
        let mut it = s.split('-');
        let y: i32 = it.next()?.parse().ok()?;
        let m: u32 = it.next()?.parse().ok()?;
        let d: u32 = it.next()?.parse().ok()?;
        let valid = it.next().is_none() && (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d);
        valid.then_some(Date { y, m, d })
    }

    fn format(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

fn local_today() -> Date {
    let st = unsafe { GetLocalTime() };
    Date { y: i32::from(st.wYear), m: u32::from(st.wMonth), d: u32::from(st.wDay) }
}

/// Accepts `H:MM` / `HH:MM` and returns it zero-padded.
fn parse_time(s: &str) -> Option<String> {
    let (h, m) = s.split_once(':')?;
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    if !digits(h) || !digits(m) || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

/// Splits user input "14:30 Title" into (time, title); time is "" when absent.
fn parse_input(text: &str) -> (String, String) {
    let text = text.trim();
    if let Some((first, rest)) = text.split_once(char::is_whitespace) {
        let rest = rest.trim();
        if let (Some(time), false) = (parse_time(first), rest.is_empty()) {
            return (time, rest.to_string());
        }
    }
    (String::new(), text.to_string())
}

// --------------------------------------------------------------- events ----

struct Event {
    date: Date,
    /// "HH:MM" or "" for all-day.
    time: String,
    title: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Day(usize),
    /// Month label: jump back to today.
    Label,
    Event(usize),
    Delete(usize),
    Add,
}

struct Layout {
    strip: Rect,
    label: Rect,
    days: Rect,
    lower: Rect,
    list: Rect,
    add: Rect,
}

fn layout(area: Rect) -> Layout {
    let (strip, rest) = area.split_top(STRIP_H);
    let (label, days) = strip.split_left(LABEL_W);
    let lower = Rect::new(rest.x, rest.y + GAP, rest.w, (rest.h - GAP).max(0.0));
    let add = Rect::new(lower.x, lower.bottom() - ADD_H, lower.w, ADD_H);
    let list = Rect::new(lower.x, lower.y, lower.w, (lower.h - ADD_H - 2.0).max(0.0));
    Layout { strip, label, days, lower, list, add }
}

pub struct CalendarPage {
    today: Date,
    selected: Date,
    events: Vec<Event>,
    scroll: f32,
    list_h: f32,
    hover: Option<Hit>,
    field: TextField,
}

impl CalendarPage {
    pub fn new(_waker: Waker) -> Self {
        let mut events: Vec<Event> = store::read_lines(FILE)
            .iter()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '|');
                let date = Date::parse(parts.next()?)?;
                let time = match parts.next()? {
                    "" => String::new(),
                    t => parse_time(t)?,
                };
                let title = parts.next()?.trim();
                (!title.is_empty()).then(|| Event { date, time, title: title.to_string() })
            })
            .collect();
        Self::sort(&mut events);
        let today = local_today();
        Self {
            today,
            selected: today,
            events,
            scroll: 0.0,
            list_h: EV_H * 2.0,
            hover: None,
            field: TextField::default(),
        }
    }

    fn sort(events: &mut [Event]) {
        // All-day ("") sorts before timed events; sort is stable.
        events.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.time.cmp(&b.time)));
    }

    fn save(&self) {
        let lines: Vec<String> =
            self.events.iter().map(|e| format!("{}|{}|{}", e.date.format(), e.time, e.title)).collect();
        let _ = store::write_lines(FILE, &lines);
    }

    /// Indices (into `events`) of the selected day's events, in time order.
    fn day_events(&self) -> Vec<usize> {
        (0..self.events.len()).filter(|&i| self.events[i].date == self.selected).collect()
    }

    fn has_events(&self, date: Date) -> bool {
        self.events.iter().any(|e| e.date == date)
    }

    fn week_start(&self) -> Date {
        self.selected.add_days(-i64::from(self.selected.monday_index()))
    }

    fn max_scroll(&self, count: usize) -> f32 {
        (count as f32 * EV_H - self.list_h).max(0.0)
    }

    fn select(&mut self, date: Date) {
        self.selected = date;
        self.scroll = 0.0;
    }

    /// The empty state replaces the list and add row until the user edits.
    fn show_empty(&self, count: usize) -> bool {
        count == 0 && !self.field.is_active()
    }

    fn hit(&self, p: Point, area: Rect) -> Option<Hit> {
        let lay = layout(area);
        if lay.label.contains(p) {
            return Some(Hit::Label);
        }
        if lay.days.contains(p) {
            let cw = lay.days.w / 7.0;
            return Some(Hit::Day((((p.x - lay.days.x) / cw) as usize).min(6)));
        }
        let evs = self.day_events();
        if self.show_empty(evs.len()) {
            return lay.lower.contains(p).then_some(Hit::Add);
        }
        if lay.add.contains(p) {
            return Some(Hit::Add);
        }
        if lay.list.contains(p) {
            let i = ((p.y - lay.list.y + self.scroll) / EV_H).floor();
            if i >= 0.0 && (i as usize) < evs.len() {
                let i = i as usize;
                return Some(if p.x >= lay.list.right() - DEL_W { Hit::Delete(i) } else { Hit::Event(i) });
            }
        }
        None
    }

    fn begin_edit(&mut self, host: &mut Host) {
        self.field.begin("");
        host.want_keyboard = true;
    }

    fn finish_edit(&mut self, commit: bool) {
        let text = self.field.finish();
        if !commit {
            return;
        }
        let (time, title) = parse_input(&text);
        if title.is_empty() {
            return;
        }
        self.events.push(Event { date: self.selected, time, title });
        Self::sort(&mut self.events);
        self.save();
    }

    fn remove(&mut self, pos: usize) {
        let evs = self.day_events();
        if let Some(&idx) = evs.get(pos) {
            self.events.remove(idx);
            self.scroll = self.scroll.min(self.max_scroll(evs.len() - 1));
            self.save();
        }
    }

    fn press(&mut self, p: Point, area: Rect, host: &mut Host) -> bool {
        match self.hit(p, area) {
            Some(Hit::Day(i)) => {
                let date = self.week_start().add_days(i as i64);
                self.select(date);
            }
            Some(Hit::Label) => {
                let today = self.today;
                self.select(today);
            }
            Some(Hit::Add) if !self.field.is_active() => self.begin_edit(host),
            Some(Hit::Delete(i)) => {
                self.remove(i);
                self.hover = self.hit(p, area);
            }
            _ => return false,
        }
        true
    }
}

impl Page for CalendarPage {
    fn title(&self) -> String {
        "Calendar".to_string()
    }

    fn tick(&mut self) -> bool {
        let today = local_today();
        let day_changed = today != self.today;
        self.today = today;
        self.field.blink() | day_changed
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let lay = layout(area);
        self.list_h = lay.list.h;

        // Month label (click = back to today).
        let label_hover = self.hover == Some(Hit::Label);
        let month = MONTHS[(self.selected.m.clamp(1, 12) - 1) as usize];
        let month_color = if label_hover { theme::ACCENT } else { theme::TEXT };
        let (top, bottom) = lay.label.inset(0.0, 4.0).split_top(14.0);
        ctx.text(month, top, TextStyle::new(12.0, Weight::Semibold, month_color));
        ctx.text(&self.selected.y.to_string(), bottom, TextStyle::new(10.0, Weight::Regular, theme::TEXT_FAINT));

        // Week strip.
        let start = self.week_start();
        let cw = lay.days.w / 7.0;
        for i in 0..7usize {
            let date = start.add_days(i as i64);
            let cell = Rect::new(lay.days.x + i as f32 * cw, lay.days.y, cw, lay.days.h);
            let selected = date == self.selected;
            let is_today = date == self.today;
            if selected || self.hover == Some(Hit::Day(i)) {
                let pill = Rect::new(cell.x + (cw - PILL_W) / 2.0, cell.y, PILL_W, cell.h);
                let fill = match (selected, is_today) {
                    (true, true) => theme::ACCENT,
                    (true, false) => theme::TEXT,
                    _ => theme::CARD,
                };
                ctx.fill_rect(pill, 10.0, fill);
            }
            let (letter_color, num_color, dot_color) = if selected {
                (theme::BG.with_alpha(0.6), theme::BG, theme::BG.with_alpha(0.7))
            } else if is_today {
                (theme::ACCENT, theme::ACCENT, theme::ACCENT)
            } else {
                (theme::TEXT_FAINT, theme::TEXT, theme::TEXT_DIM)
            };
            let centered = |size, weight, color| TextStyle::new(size, weight, color).align(Align::Center);
            ctx.text(
                WEEKDAYS[i],
                Rect::new(cell.x, cell.y + 3.0, cw, 11.0),
                centered(9.0, Weight::Medium, letter_color),
            );
            ctx.text(
                &date.d.to_string(),
                Rect::new(cell.x, cell.y + 13.0, cw, 16.0),
                centered(13.0, Weight::Semibold, num_color),
            );
            if self.has_events(date) {
                ctx.fill_circle(Point { x: cell.center().x, y: cell.bottom() - 3.0 }, 1.5, dot_color);
            }
        }

        // Events of the selected day.
        let evs = self.day_events();
        if self.show_empty(evs.len()) {
            let hover = self.hover == Some(Hit::Add);
            ctx.stroke_rect(lay.lower, 12.0, if hover { theme::TEXT_DIM } else { theme::BORDER }, 1.0, true);
            ctx.text(
                "No events \u{00B7} click to add",
                lay.lower,
                TextStyle::new(12.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Center),
            );
            return;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(evs.len()));

        ctx.push_clip(lay.list);
        let hovered_pos = match self.hover {
            Some(Hit::Event(i)) | Some(Hit::Delete(i)) => Some(i),
            _ => None,
        };
        let first = (self.scroll / EV_H).floor().max(0.0) as usize;
        for pos in first..evs.len() {
            let row = Rect::new(lay.list.x, lay.list.y + pos as f32 * EV_H - self.scroll, lay.list.w, EV_H);
            if row.y >= lay.list.bottom() {
                break;
            }
            let hovered = hovered_pos == Some(pos);
            if hovered {
                ctx.fill_rect(row, 5.0, theme::CARD);
            }
            let ev = &self.events[evs[pos]];
            let (time_rect, rest) = row.inset(6.0, 0.0).split_left(TIME_W);
            let (time, time_color) = if ev.time.is_empty() {
                ("All day", theme::TEXT_FAINT)
            } else {
                (ev.time.as_str(), theme::TEXT_DIM)
            };
            ctx.text(time, time_rect, TextStyle::new(11.0, Weight::Regular, time_color));
            let title_rect = Rect::new(rest.x, rest.y, (rest.w - DEL_W).max(0.0), rest.h);
            ctx.text(&ev.title, title_rect, TextStyle::new(12.0, Weight::Regular, theme::TEXT));
            if hovered {
                let del = Rect::new(row.right() - DEL_W, row.y, DEL_W, EV_H);
                let color = if self.hover == Some(Hit::Delete(pos)) { theme::DANGER } else { theme::TEXT_DIM };
                ctx.text("\u{E711}", del, TextStyle::icon(9.0, color));
            }
        }
        ctx.pop_clip();

        // Add-event row.
        let editing = self.field.is_active();
        let hover_add = self.hover == Some(Hit::Add) && !editing;
        ctx.fill_rect(lay.add, 10.0, if hover_add { theme::CARD_HOVER } else { theme::CARD });
        if editing {
            ctx.stroke_rect(lay.add, 10.0, theme::BORDER, 1.0, false);
        }
        let (icon, field) = lay.add.split_left(24.0);
        ctx.text("\u{E710}", icon, TextStyle::icon(10.0, theme::TEXT_DIM));
        let field = Rect::new(field.x, field.y, (field.w - 8.0).max(0.0), field.h);
        if editing {
            self.field.draw(ctx, field, TextStyle::new(12.0, Weight::Regular, theme::TEXT), "14:30 Title");
        } else {
            ctx.text("Add event\u{2026}", field, TextStyle::new(12.0, Weight::Medium, theme::TEXT_DIM));
        }
    }

    fn on_mouse(&mut self, ev: Mouse, area: Rect, host: &mut Host) -> bool {
        self.list_h = layout(area).list.h;
        match ev {
            Mouse::Move(p) => {
                let hit = self.hit(p, area);
                let changed = hit != self.hover;
                self.hover = hit;
                changed
            }
            Mouse::Leave => self.hover.take().is_some(),
            Mouse::Down(p) | Mouse::DoubleClick(p) => self.press(p, area, host),
            Mouse::Wheel(p, delta) => {
                let lay = layout(area);
                if lay.strip.contains(p) {
                    // One week per notch: up = earlier, down = later.
                    if delta == 0.0 {
                        return false;
                    }
                    let weeks = if delta > 0.0 { -1 } else { 1 };
                    let date = self.selected.add_days(7 * weeks);
                    self.select(date);
                    return true;
                }
                if !lay.lower.contains(p) {
                    return false;
                }
                let before = self.scroll;
                let max = self.max_scroll(self.day_events().len());
                self.scroll = (self.scroll - delta * EV_H).clamp(0.0, max);
                self.hover = self.hit(p, area);
                self.scroll != before
            }
            Mouse::Up(_) => false,
        }
    }

    fn on_char(&mut self, ch: char) -> bool {
        self.field.insert(ch)
    }

    fn on_key(&mut self, vk: u16) -> bool {
        if !self.field.is_active() {
            return false;
        }
        match self.field.key(vk) {
            Outcome::Commit => self.finish_edit(true),
            Outcome::Cancel => self.finish_edit(false),
            Outcome::Pending => {}
        }
        true
    }

    fn editing(&self) -> bool {
        self.field.is_active()
    }

    fn on_focus_lost(&mut self) -> bool {
        if !self.field.is_active() {
            return false;
        }
        self.finish_edit(true);
        true
    }
}
