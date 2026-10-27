//! Remaining Claude Code rate-limit windows read from `claude-usage.json`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::gfx::{Align, Color, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::Page;
use crate::theme;
use crate::worker::Waker;

/// Default location of the usage JSON (inside Ada's data folder).
const DEFAULT_USAGE_FILE: &str = "claude-usage.json";
/// settings.txt key that overrides the usage file path.
const SETTINGS_KEY: &str = "claude_usage_file";
const STALE_MS: i64 = 6 * 3_600_000;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[derive(Clone, Copy)]
struct WindowLimit {
    remaining: f32,
    resets_at: Option<i64>,
}

struct UsageData {
    five_hour: Option<WindowLimit>,
    seven_day: Option<WindowLimit>,
    model: Option<String>,
    ts_ms: Option<i64>,
}

pub struct ClaudePage {
    path: PathBuf,
    last_stamp: Option<(SystemTime, u64)>,
    data: Option<UsageData>,
    ticks: u32,
    last_minute: u16,
}

impl ClaudePage {
    pub fn new(_waker: Waker) -> Self {
        let path = std::env::var_os("ADA_CLAUDE_USAGE_FILE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                crate::store::read_kv("settings.txt")
                    .get(SETTINGS_KEY)
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| crate::store::data_dir().join(DEFAULT_USAGE_FILE));
        let last_minute = unsafe { GetLocalTime() }.wMinute;
        let mut page = Self {
            path,
            last_stamp: None,
            data: None,
            ticks: 0,
            last_minute,
        };
        page.refresh_file();
        page
    }

    fn refresh_file(&mut self) -> bool {
        let Ok(meta) = fs::metadata(&self.path) else {
            return false;
        };
        let Ok(modified) = meta.modified() else {
            return false;
        };
        let stamp = (modified, meta.len());
        if self.last_stamp == Some(stamp) {
            return false;
        }
        let Some(parsed) = fs::read_to_string(&self.path).ok().and_then(|s| parse_usage(&s)) else {
            return false;
        };
        self.last_stamp = Some(stamp);
        self.data = Some(parsed);
        true
    }
}

impl Page for ClaudePage {
    fn title(&self) -> String {
        "Claude".to_string()
    }

    fn tick(&mut self) -> bool {
        self.ticks = self.ticks.wrapping_add(1);
        let mut redraw = false;
        if self.ticks % 10 == 0 {
            redraw |= self.refresh_file();
        }
        let minute = unsafe { GetLocalTime() }.wMinute;
        if minute != self.last_minute {
            self.last_minute = minute;
            redraw = true;
        }
        redraw
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let Some(data) = &self.data else {
            ctx.stroke_rect(area.inset(0.0, 2.0), 10.0, theme::BORDER, 1.0, true);
            ctx.text(
                "No usage data yet",
                area,
                TextStyle::new(12.5, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
            );
            return;
        };

        let now_ms = now_utc_ms();
        let now_secs = now_ms / 1_000;
        let offset_secs = local_offset_secs(now_secs);

        let stale = data.ts_ms.and_then(|ts| {
            let age_ms = now_ms - ts;
            if age_ms > STALE_MS {
                let hours = age_ms / 3_600_000;
                if hours >= 48 {
                    Some(format!("{}d ago", hours / 24))
                } else {
                    Some(format!("{hours}h ago"))
                }
            } else {
                None
            }
        });

        // The file only updates while Claude Code runs; a window whose reset time
        // has passed is back to full even though the file still shows old usage.
        let is_reset = |w: &Option<WindowLimit>| w.and_then(|w| w.resets_at).is_some_and(|t| t <= now_secs);
        let current = |w: Option<WindowLimit>| {
            if is_reset(&w) {
                Some(WindowLimit { remaining: 100.0, resets_at: None })
            } else {
                w
            }
        };
        let five_reset = if is_reset(&data.five_hour) {
            "window reset".to_string()
        } else {
            data.five_hour.and_then(|w| w.resets_at).map(|t| format_countdown(t, now_secs)).unwrap_or_default()
        };
        let week_reset = if is_reset(&data.seven_day) {
            "window reset".to_string()
        } else {
            data.seven_day.and_then(|w| w.resets_at).map(|t| format_weekly_reset(t, offset_secs)).unwrap_or_default()
        };
        let five_hour = current(data.five_hour);
        let seven_day = current(data.seven_day);

        let gap = 12.0;
        let card_w = ((area.w - gap) / 2.0).max(0.0);
        let card_h = (area.h - 4.0).max(0.0);
        let left_card = Rect::new(area.x, area.y + 2.0, card_w, card_h);
        let right_card = Rect::new(area.x + card_w + gap, area.y + 2.0, card_w, card_h);

        draw_window_card(
            ctx,
            left_card,
            "5-hour",
            stale.as_deref(),
            None,
            five_hour,
            &five_reset,
        );
        draw_window_card(
            ctx,
            right_card,
            "Weekly",
            stale.as_deref(),
            data.model.as_deref(),
            seven_day,
            &week_reset,
        );
    }
}

fn remaining_color(rem: f32) -> Color {
    if rem > 40.0 {
        theme::ACCENT
    } else if rem >= 15.0 {
        theme::WARN
    } else {
        theme::DANGER
    }
}

fn draw_window_card(
    ctx: &mut DrawCtx,
    card: Rect,
    base_label: &str,
    stale_note: Option<&str>,
    badge: Option<&str>,
    limit: Option<WindowLimit>,
    reset_text: &str,
) {
    ctx.fill_rect(card, 10.0, theme::CARD);

    let radius = 25.0;
    let stroke = 5.0;
    let ring_center = Point { x: card.x + 11.0 + radius, y: card.center().y };
    let ring_rect = Rect::new(
        ring_center.x - radius,
        ring_center.y - radius,
        radius * 2.0,
        radius * 2.0,
    );

    let col_x = ring_center.x + radius + 10.0;
    let col_w = (card.right() - 10.0 - col_x).max(0.0);

    let label = match stale_note {
        Some(note) => format!("{base_label} · {note}"),
        None => base_label.to_string(),
    };
    let label_style = TextStyle::new(11.0, Weight::Medium, theme::TEXT_DIM);
    let label_rect = Rect::new(col_x, card.y + 10.0, col_w, 15.0);
    ctx.text(&label, label_rect, label_style);

    if let Some(model) = badge {
        let used_w = ctx.measure_text(&label, &label_style);
        let avail = col_w - used_w - 8.0;
        if avail >= 40.0 {
            ctx.text(
                model,
                Rect::new(col_x + used_w + 8.0, label_rect.y, avail, label_rect.h),
                TextStyle::new(10.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Right),
            );
        }
    }

    if let Some(win) = limit {
        let color = remaining_color(win.remaining);
        let fraction = (win.remaining / 100.0).clamp(0.0, 1.0);
        ctx.stroke_circle(ring_center, radius, color.with_alpha(0.18), stroke);
        ctx.arc(ring_center, radius, fraction, color, stroke);

        let pct_str = format!("{:.0}%", win.remaining);
        ctx.text(
            &pct_str,
            ring_rect,
            TextStyle::new(13.5, Weight::Bold, theme::TEXT).align(Align::Center),
        );

        ctx.text(
            "remaining",
            Rect::new(col_x, card.y + 28.0, col_w, 22.0),
            TextStyle::new(11.5, Weight::Medium, theme::TEXT_DIM),
        );
        ctx.text(
            reset_text,
            Rect::new(col_x, card.y + 52.0, col_w, 30.0),
            TextStyle::new(11.0, Weight::Regular, theme::TEXT_DIM),
        );
    } else {
        ctx.stroke_circle(ring_center, radius, theme::TEXT_FAINT.with_alpha(0.25), stroke);
        ctx.text(
            "—",
            ring_rect,
            TextStyle::new(13.5, Weight::Bold, theme::TEXT_FAINT).align(Align::Center),
        );
        ctx.text(
            "No data",
            Rect::new(col_x, card.y + 52.0, col_w, 30.0),
            TextStyle::new(11.0, Weight::Regular, theme::TEXT_FAINT),
        );
    }
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

fn parse_window(v: &Value) -> Option<WindowLimit> {
    let used = v.get("used_percentage")?.as_f64()?;
    let remaining = (100.0 - used).clamp(0.0, 100.0) as f32;
    let resets_at = v.get("resets_at").and_then(json_i64).filter(|&t| t > 0);
    Some(WindowLimit { remaining, resets_at })
}

fn parse_usage(raw: &str) -> Option<UsageData> {
    let root: Value = serde_json::from_str(raw).ok()?;
    let limits = root.get("rate_limits")?;
    let five_hour = limits.get("five_hour").and_then(parse_window);
    let seven_day = limits.get("seven_day").and_then(parse_window);
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    let model = root
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let ts_ms = root.get("ts").and_then(json_i64).filter(|&t| t > 0);
    Some(UsageData { five_hour, seven_day, model, ts_ms })
}

fn now_utc_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Civil date (proleptic Gregorian) to days since 1970-01-01.
fn ymd_to_days(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Local UTC offset in seconds, rounded to the nearest 15 minutes.
fn local_offset_secs(utc_secs: i64) -> i64 {
    let st = unsafe { GetLocalTime() };
    let local_days = ymd_to_days(st.wYear as i32, st.wMonth as u32, st.wDay as u32);
    let local_secs = local_days * 86_400
        + st.wHour as i64 * 3_600
        + st.wMinute as i64 * 60
        + st.wSecond as i64;
    let raw = local_secs - utc_secs;
    let q = 900_i64;
    if raw >= 0 {
        ((raw + q / 2) / q) * q
    } else {
        ((raw - q / 2) / q) * q
    }
}

fn format_countdown(resets_at: i64, now_secs: i64) -> String {
    let diff = resets_at.saturating_sub(now_secs);
    if diff <= 0 {
        return "resets soon".to_string();
    }
    let total_mins = diff / 60;
    if total_mins == 0 {
        return "resets in <1m".to_string();
    }
    let hours = total_mins / 60;
    let mins = total_mins % 60;
    if hours > 0 {
        format!("resets in {hours}h {mins}m")
    } else {
        format!("resets in {mins}m")
    }
}

fn format_weekly_reset(resets_at: i64, offset_secs: i64) -> String {
    let local_ts = resets_at.saturating_add(offset_secs);
    let days = local_ts.div_euclid(86_400);
    let secs_of_day = local_ts.rem_euclid(86_400);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let dow = (days + 4).rem_euclid(7) as usize;
    format!("resets {} {:02}:{:02}", DAYS[dow], hour, minute)
}
