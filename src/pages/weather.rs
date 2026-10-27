//! Current weather and short-term hourly forecast for the IP-detected location.

use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::gfx::{Align, DrawCtx, Rect, TextStyle, Weight};
use crate::http;
use crate::pages::Page;
use crate::theme;
use crate::worker::{self, Waker};

const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(60);
const OFFLINE_AFTER: Duration = Duration::from_secs(60 * 60);
const HOURLY_COUNT: usize = 5;

// Color emoji (Segoe UI Emoji); Segoe Fluent Icons has no weather set.
const GLYPH_CLEAR_DAY: &str = "\u{2600}\u{FE0F}";
const GLYPH_CLEAR_NIGHT: &str = "\u{1F319}";
const GLYPH_PARTLY_CLOUDY: &str = "\u{26C5}";
const GLYPH_CLOUDY: &str = "\u{2601}\u{FE0F}";
const GLYPH_FOG: &str = "\u{1F32B}\u{FE0F}";
const GLYPH_DRIZZLE: &str = "\u{1F326}\u{FE0F}";
const GLYPH_RAIN: &str = "\u{1F327}\u{FE0F}";
const GLYPH_SNOW: &str = "\u{2744}\u{FE0F}";
const GLYPH_THUNDERSTORM: &str = "\u{26C8}\u{FE0F}";

fn symbol_info(code: &str) -> (&'static str, &'static str) {
    let (base, is_night) = if let Some(b) = code.strip_suffix("_night") {
        (b, true)
    } else if let Some(b) = code.strip_suffix("_day") {
        (b, false)
    } else if let Some(b) = code.strip_suffix("_polartwilight") {
        (b, false)
    } else {
        (code, false)
    };

    if base.starts_with("clearsky") {
        (if is_night { GLYPH_CLEAR_NIGHT } else { GLYPH_CLEAR_DAY }, "Clear")
    } else if base.starts_with("fair") {
        (if is_night { GLYPH_CLEAR_NIGHT } else { GLYPH_CLEAR_DAY }, "Fair")
    } else if base.starts_with("partlycloudy") {
        (GLYPH_PARTLY_CLOUDY, "Partly cloudy")
    } else if base.starts_with("cloudy") {
        (GLYPH_CLOUDY, "Cloudy")
    } else if base.starts_with("fog") {
        (GLYPH_FOG, "Fog")
    } else if base.contains("thunder") {
        (GLYPH_THUNDERSTORM, "Thunderstorm")
    } else if base.contains("drizzle") {
        (GLYPH_DRIZZLE, "Drizzle")
    } else if base.contains("snow") {
        (GLYPH_SNOW, "Snow")
    } else if base.contains("sleet") {
        (GLYPH_SNOW, "Sleet")
    } else if base.contains("rain") {
        (GLYPH_RAIN, "Rain")
    } else {
        (GLYPH_CLOUDY, "")
    }
}

struct Location {
    city: String,
    latitude: f64,
    longitude: f64,
}

struct HourForecast {
    hour: String,
    temp: f32,
    symbol: String,
}

struct Weather {
    city: String,
    temp: f32,
    feels_like: f32,
    symbol: String,
    high: f32,
    low: f32,
    hourly: Vec<HourForecast>,
    fetched_at: Instant,
}

enum State {
    Locating,
    Failed,
    Ready(Weather),
}

pub struct WeatherPage {
    rx: Receiver<Result<Weather, String>>,
    state: State,
    offline: bool,
}

impl WeatherPage {
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = mpsc::channel();
        worker::spawn("weather", move || {
            let mut location: Option<Location> = None;
            loop {
                let res = fetch_weather(&mut location);
                let ok = res.is_ok();
                if tx.send(res).is_err() {
                    break;
                }
                waker.wake();
                thread::sleep(if ok { REFRESH_INTERVAL } else { RETRY_INTERVAL });
            }
        });
        Self { rx, state: State::Locating, offline: false }
    }
}

impl Page for WeatherPage {
    fn title(&self) -> String {
        "Weather".to_string()
    }

    fn poll(&mut self) -> bool {
        let mut redraw = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Ok(weather) => {
                    self.offline = weather.fetched_at.elapsed() > OFFLINE_AFTER;
                    self.state = State::Ready(weather);
                    redraw = true;
                }
                Err(_) => {
                    if matches!(self.state, State::Locating) {
                        self.state = State::Failed;
                        redraw = true;
                    }
                }
            }
        }
        redraw
    }

    fn tick(&mut self) -> bool {
        if let State::Ready(w) = &self.state {
            let offline = w.fetched_at.elapsed() > OFFLINE_AFTER;
            if offline != self.offline {
                self.offline = offline;
                return true;
            }
        }
        false
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let weather = match &self.state {
            State::Locating => {
                ctx.text("Locating…", area, TextStyle::new(13.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center));
                return;
            }
            State::Failed => {
                ctx.text("Weather unavailable", area, TextStyle::new(13.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center));
                return;
            }
            State::Ready(w) => w,
        };

        let (left, rest) = area.split_left(162.0);
        let (_, card) = rest.split_left(8.0);

        let (glyph, label) = symbol_info(&weather.symbol);
        ctx.text(glyph, Rect::new(left.x, left.y + 2.0, 34.0, 42.0), TextStyle::emoji(28.0));

        let temp_str = format!("{}°", round_deg(weather.temp));
        ctx.text(
            &temp_str,
            Rect::new(left.x + 38.0, left.y + 1.0, (left.w - 38.0).max(0.0), 42.0),
            TextStyle::new(28.0, Weight::Bold, theme::TEXT),
        );

        let text_w = (left.w - 2.0).max(0.0);
        let sub_rect = Rect::new(left.x + 2.0, left.y + 46.0, text_w, 20.0);
        let sub_style = TextStyle::new(12.0, Weight::Medium, theme::TEXT_DIM);
        let cond_line = if label.is_empty() {
            weather.city.clone()
        } else {
            format!("{label} · {}", weather.city)
        };
        if self.offline {
            let off_str = " · offline";
            let off_style = TextStyle::new(12.0, Weight::Regular, theme::TEXT_FAINT);
            let off_w = (ctx.measure_text(off_str, &off_style) + 1.0).min(sub_rect.w);
            let max_cond_w = (sub_rect.w - off_w).max(0.0);
            let raw_cond_w = ctx.measure_text(&cond_line, &sub_style);
            let draw_cond_w = if raw_cond_w <= max_cond_w {
                (raw_cond_w + 2.0).min(sub_rect.w)
            } else {
                max_cond_w
            };
            let cond_advance = raw_cond_w.min(max_cond_w);
            ctx.text(&cond_line, Rect::new(sub_rect.x, sub_rect.y, draw_cond_w, sub_rect.h), sub_style);
            ctx.text(off_str, Rect::new(sub_rect.x + cond_advance, sub_rect.y, off_w, sub_rect.h), off_style);
        } else {
            ctx.text(&cond_line, sub_rect, sub_style);
        }

        let meta_line = format!(
            "H {}° L {}° · Feels {}°",
            round_deg(weather.high),
            round_deg(weather.low),
            round_deg(weather.feels_like),
        );
        ctx.text(
            &meta_line,
            Rect::new(left.x + 2.0, left.y + 67.0, text_w, 20.0),
            TextStyle::new(11.5, Weight::Regular, theme::TEXT_DIM),
        );

        ctx.fill_rect(card, 10.0, theme::CARD);
        if weather.hourly.is_empty() {
            ctx.text(
                "No hourly data",
                card,
                TextStyle::new(11.5, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
            );
        } else {
            let inner = card.inset(4.0, 8.0);
            let col_w = inner.w / HOURLY_COUNT as f32;
            for (i, entry) in weather.hourly.iter().take(HOURLY_COUNT).enumerate() {
                let x = inner.x + i as f32 * col_w;
                ctx.text(
                    &entry.hour,
                    Rect::new(x, inner.y, col_w, 20.0),
                    TextStyle::new(11.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
                );
                let (h_glyph, _) = symbol_info(&entry.symbol);
                ctx.text(
                    h_glyph,
                    Rect::new(x, inner.center().y - 14.0, col_w, 28.0),
                    TextStyle::emoji(15.0),
                );
                let h_temp = format!("{}°", round_deg(entry.temp));
                ctx.text(
                    &h_temp,
                    Rect::new(x, inner.bottom() - 20.0, col_w, 20.0),
                    TextStyle::new(12.0, Weight::Semibold, theme::TEXT).align(Align::Center),
                );
            }
        }
    }
}

fn round_deg(t: f32) -> i32 {
    t.round() as i32
}

fn as_f32(v: &Value) -> Option<f32> {
    v.as_f64().map(|n| n as f32).filter(|n| n.is_finite())
}

fn feels_like_c(temp: f32, wind_ms: Option<f32>) -> f32 {
    if let Some(w) = wind_ms {
        if temp <= 10.0 && w > 1.3 {
            let t = temp as f64;
            let v_pow = (w as f64 * 3.6).powf(0.16);
            return (13.12 + 0.6215 * t - 11.37 * v_pow + 0.3965 * t * v_pow) as f32;
        }
    }
    temp
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
fn local_offset_secs() -> i64 {
    let utc_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
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

fn parse_local_hour(iso_utc: &str, offset_secs: i64) -> Option<String> {
    let hh: i64 = iso_utc.get(11..13)?.parse().ok()?;
    let mm: i64 = iso_utc.get(14..16)?.parse().ok()?;
    if !(0..24).contains(&hh) || !(0..60).contains(&mm) {
        return None;
    }
    let local_secs = (hh * 3_600 + mm * 60 + offset_secs).rem_euclid(86_400);
    Some(format!("{:02}", local_secs / 3_600))
}

fn entry_details(entry: &Value) -> Option<&Value> {
    entry.get("data")?.get("instant")?.get("details")
}

fn entry_temp(entry: &Value) -> Option<f32> {
    entry_details(entry)?.get("air_temperature").and_then(as_f32)
}

fn entry_symbol(entry: &Value) -> &str {
    let Some(data) = entry.get("data") else { return "" };
    let from = |period: &str| data.get(period)?.get("summary")?.get("symbol_code")?.as_str();
    from("next_1_hours")
        .or_else(|| from("next_6_hours"))
        .or_else(|| from("next_12_hours"))
        .unwrap_or("")
}

fn fetch_weather(location: &mut Option<Location>) -> Result<Weather, String> {
    if location.is_none() {
        *location = Some(fetch_location()?);
    }
    let loc = location.as_ref().ok_or_else(|| "missing location".to_string())?;
    fetch_forecast(loc)
}

fn fetch_location() -> Result<Location, String> {
    let json = http::get_json("https://ipwho.is/")?;
    if json.get("success").and_then(Value::as_bool) != Some(true) {
        let msg = json.get("message").and_then(Value::as_str).unwrap_or("location lookup failed");
        return Err(msg.to_string());
    }
    let latitude = json
        .get("latitude")
        .and_then(Value::as_f64)
        .filter(|&n| n >= -90.0 && n <= 90.0)
        .ok_or_else(|| "invalid latitude".to_string())?;
    let longitude = json
        .get("longitude")
        .and_then(Value::as_f64)
        .filter(|&n| n >= -180.0 && n <= 180.0)
        .ok_or_else(|| "invalid longitude".to_string())?;
    let city = ["city", "region", "country"]
        .into_iter()
        .find_map(|k| json.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()))
        .unwrap_or("Local")
        .to_string();
    Ok(Location { city, latitude, longitude })
}

fn fetch_forecast(loc: &Location) -> Result<Weather, String> {
    let url = format!(
        "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={:.4}&lon={:.4}",
        loc.latitude + 0.0,
        loc.longitude + 0.0,
    );
    let json = http::get_json(&url)?;
    let series = json
        .get("properties")
        .and_then(|p| p.get("timeseries"))
        .and_then(Value::as_array)
        .ok_or_else(|| "missing properties.timeseries".to_string())?;
    let first = series.first().ok_or_else(|| "empty timeseries".to_string())?;

    let temp = entry_temp(first).ok_or_else(|| "missing air_temperature".to_string())?;
    let wind_speed = entry_details(first).and_then(|d| d.get("wind_speed")).and_then(as_f32);
    let feels_like = feels_like_c(temp, wind_speed);
    let symbol = entry_symbol(first).to_string();

    let mut high = temp;
    let mut low = temp;
    for entry in series.iter().take(24) {
        if let Some(t) = entry_temp(entry) {
            high = high.max(t);
            low = low.min(t);
        }
    }

    let offset_secs = local_offset_secs();
    let mut hourly = Vec::with_capacity(HOURLY_COUNT);
    for entry in series.iter().skip(1).take(HOURLY_COUNT) {
        let Some(time_str) = entry.get("time").and_then(Value::as_str) else { continue };
        let Some(hour) = parse_local_hour(time_str, offset_secs) else { continue };
        let Some(h_temp) = entry_temp(entry) else { continue };
        hourly.push(HourForecast {
            hour,
            temp: h_temp,
            symbol: entry_symbol(entry).to_string(),
        });
    }

    Ok(Weather {
        city: loc.city.clone(),
        temp,
        feels_like,
        symbol,
        high,
        low,
        hourly,
        fetched_at: Instant::now(),
    })
}
