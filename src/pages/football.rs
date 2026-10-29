//! Follows one football team: live score, next match and last result, drawn
//! like a match header with the two team crests.
//!
//! Data comes from ESPN's public JSON (no key): the team schedule (results and
//! fixtures) of a few leagues, plus the per-match summary while a match is on.
//! The team is the ESPN team id in settings.txt (`football_team`, default 1895
//! = Besiktas). A worker thread refreshes the schedule every 15 minutes and
//! polls the live match every 30 s (from 10 minutes before kick-off until the
//! match is over). Network errors keep the last good data. Goals, kick-off and
//! full time raise a peek; the collapsed notch shows a live activity while a
//! match is on. Crests (ESPN logos) are downloaded by the worker once and
//! cached under the local Ada data folder (`football`). The worker stops when
//! the page is dropped.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::gfx::{Align, Color, DrawCtx, Rect, TextStyle, Weight};
use crate::http;
use crate::pages::{Activity, Page};
use crate::store;
use crate::theme;
use crate::worker::{self, Peek, Waker};

const SETTINGS_FILE: &str = "settings.txt";
const SETTINGS_KEY: &str = "football_team";
/// "0" turns off keeping the notch open during a live match.
const KEEP_OPEN_KEY: &str = "football_keep_open";
const CREST_RETRY: Duration = Duration::from_secs(30 * 60);
const DEFAULT_TEAM: &str = "1895";
const LEAGUES: [&str; 5] = [
    "tur.1",
    "uefa.champions",
    "uefa.europa",
    "uefa.europa.conf",
    "club.friendly",
];

const SCHEDULE_EVERY: Duration = Duration::from_secs(15 * 60);
const RETRY_EVERY: Duration = Duration::from_secs(60);
const LIVE_POLL_SECS: u64 = 30;
const IDLE_POLL_SECS: u64 = 60;
/// Start polling the match this long before kick-off, and for this long after.
const PRE_WINDOW_SECS: i64 = 10 * 60;
const POST_WINDOW_SECS: i64 = 150 * 60;

const ACTIVITY_PRIORITY: u8 = 40;
const ACTIVITY_WIDTH: f32 = 200.0;
const PEEK_SECONDS: f32 = 6.0;
const GLYPH_BALL: &str = "\u{26BD}";
const GLYPH_FLAG: &str = "\u{1F3C1}";
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

// ---------------------------------------------------------------------------
// Data model and parsing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatchState {
    Pre,
    In,
    Post,
}

impl MatchState {
    fn rank(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Side {
    id: String,
    abbr: String,
    name: String,
    color: Option<String>,
    alt: Option<String>,
    score: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
struct Event {
    id: String,
    /// League code from `LEAGUES`.
    league: &'static str,
    /// Kick-off, unix seconds (UTC).
    ts: i64,
    state: MatchState,
    /// ESPN `shortDetail`: "FT", "HT", "67'".
    detail: String,
    clock: String,
    home: Side,
    away: Side,
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    team_id: String,
    name: Option<String>,
    last: Option<Event>,
    next: Option<Event>,
    live: Option<Event>,
    /// PNG crests by team id, for the two teams of the shown match.
    crests: BTreeMap<String, Arc<Vec<u8>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Live,
    Next,
    Last,
}

/// The match the page shows: live if any, else the next one, else the last.
fn shown(snap: &Snapshot) -> Option<(&Event, Shown)> {
    snap.live
        .as_ref()
        .map(|e| (e, Shown::Live))
        .or_else(|| snap.next.as_ref().map(|e| (e, Shown::Next)))
        .or_else(|| snap.last.as_ref().map(|e| (e, Shown::Last)))
}

fn league_name(code: &str) -> &'static str {
    match code {
        "tur.1" => "Süper Lig",
        "uefa.champions" => "Champions League",
        "uefa.europa" => "Europa League",
        "uefa.europa.conf" => "Conference League",
        _ => "Friendly",
    }
}

fn text_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `score` is either a string ("2") or an object ({"displayValue": "2"}).
fn parse_score(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(n) => n.as_u64().map(|n| n as u32),
        Value::Object(o) => match o.get("displayValue").or_else(|| o.get("value"))? {
            Value::String(s) => s.trim().parse().ok(),
            Value::Number(n) => n.as_f64().filter(|n| *n >= 0.0).map(|n| n as u32),
            _ => None,
        },
        _ => None,
    }
}

fn parse_side(c: &Value) -> Option<Side> {
    let team = c.get("team")?;
    let id = text_of(team, "id").or_else(|| text_of(c, "id"))?;
    let abbr = text_of(team, "abbreviation").unwrap_or_else(|| id.clone());
    let name = text_of(team, "shortDisplayName")
        .or_else(|| text_of(team, "displayName"))
        .unwrap_or_else(|| abbr.clone());
    Some(Side {
        id,
        abbr,
        name,
        color: text_of(team, "color"),
        alt: text_of(team, "alternateColor"),
        score: parse_score(c.get("score")),
    })
}

/// (state, shortDetail, displayClock) from a competition's `status`.
fn parse_status(comp: &Value) -> Option<(MatchState, String, String)> {
    let status = comp.get("status")?;
    let kind = status.get("type")?;
    let state = match kind.get("state")?.as_str()? {
        "pre" => MatchState::Pre,
        "in" => MatchState::In,
        "post" => MatchState::Post,
        _ => return None,
    };
    Some((
        state,
        text_of(kind, "shortDetail").unwrap_or_default(),
        text_of(status, "displayClock").unwrap_or_default(),
    ))
}

fn parse_sides(comp: &Value) -> Option<(Side, Side)> {
    let mut home = None;
    let mut away = None;
    for c in comp.get("competitors")?.as_array()? {
        match c.get("homeAway").and_then(Value::as_str) {
            Some("home") => home = parse_side(c),
            Some("away") => away = parse_side(c),
            _ => {}
        }
    }
    Some((home?, away?))
}

fn parse_event(v: &Value, league: &'static str) -> Option<Event> {
    let comp = v.get("competitions")?.as_array()?.first()?;
    let ts = parse_iso(v.get("date").or_else(|| comp.get("date"))?.as_str()?)?;
    let (state, detail, clock) = parse_status(comp)?;
    let (home, away) = parse_sides(comp)?;
    Some(Event {
        id: text_of(v, "id")?,
        league,
        ts,
        state,
        detail,
        clock,
        home,
        away,
    })
}

/// Updates a schedule event with the `header.competitions[0]` of a summary.
fn apply_summary(base: &Event, json: &Value) -> Option<Event> {
    let comp = json
        .get("header")?
        .get("competitions")?
        .as_array()?
        .first()?;
    let (state, detail, clock) = parse_status(comp)?;
    let mut ev = Event {
        state,
        detail,
        clock,
        ..base.clone()
    };
    if let Some((home, away)) = parse_sides(comp) {
        if home.id == base.home.id && away.id == base.away.id {
            ev.home = home;
            ev.away = away;
        }
    }
    Some(ev)
}

/// (our side, opponent, we are home).
fn ours<'a>(ev: &'a Event, team: &str) -> (&'a Side, &'a Side, bool) {
    if ev.away.id == team {
        (&ev.away, &ev.home, false)
    } else {
        (&ev.home, &ev.away, true)
    }
}

/// 'W' / 'D' / 'L' with (ours, theirs) for a finished match.
fn result_of(ev: &Event, team: &str) -> Option<(char, u32, u32)> {
    if ev.state != MatchState::Post {
        return None;
    }
    let (me, opp, _) = ours(ev, team);
    let (a, b) = (me.score?, opp.score?);
    Some((
        if a > b {
            'W'
        } else if a < b {
            'L'
        } else {
            'D'
        },
        a,
        b,
    ))
}

/// "BES 2–1 GS".
fn scoreline(ev: &Event) -> String {
    format!(
        "{} {}\u{2013}{} {}",
        ev.home.abbr,
        ev.home.score.unwrap_or(0),
        ev.away.score.unwrap_or(0),
        ev.away.abbr
    )
}

/// "67'" / "HT" while a match is on.
fn minute_label(ev: &Event) -> &str {
    if ev.detail.is_empty() {
        &ev.clock
    } else {
        &ev.detail
    }
}

// ---------------------------------------------------------------------------
// Picking what to show
// ---------------------------------------------------------------------------

/// Dedupes by event id (the more advanced state wins) and applies the fresher
/// live versions from `overrides`.
fn merge(events: Vec<Event>, overrides: &HashMap<String, Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    for ev in events {
        match out.iter_mut().find(|e| e.id == ev.id) {
            Some(existing) => {
                if ev.state.rank() > existing.state.rank() {
                    *existing = ev;
                }
            }
            None => out.push(ev),
        }
    }
    for ev in &mut out {
        if let Some(o) = overrides.get(&ev.id) {
            if o.state.rank() >= ev.state.rank() {
                *ev = o.clone();
            }
        }
    }
    out
}

/// Postponed, cancelled or abandoned: ESPN marks these "post" without a result.
fn is_void(ev: &Event) -> bool {
    let d = ev.detail.to_ascii_lowercase();
    ["postp", "cancel", "abandon", "suspend"]
        .iter()
        .any(|w| d.contains(w))
}

/// (last result, next match, live match).
fn pick(events: &[Event], now: i64) -> (Option<&Event>, Option<&Event>, Option<&Event>) {
    let last = events
        .iter()
        .filter(|e| e.state == MatchState::Post && !is_void(e))
        .max_by_key(|e| e.ts);
    let next = events
        .iter()
        .filter(|e| e.state == MatchState::Pre && e.ts > now)
        .min_by_key(|e| e.ts);
    let live = events.iter().find(|e| e.state == MatchState::In);
    (last, next, live)
}

/// The match whose summary must be polled: a live one, or one that kicks off
/// within 10 minutes / started less than 2.5 h ago and is not yet over.
fn tracked(events: &[Event], now: i64) -> Option<&Event> {
    events
        .iter()
        .find(|e| e.state == MatchState::In)
        .or_else(|| {
            events.iter().find(|e| {
                e.state == MatchState::Pre
                    && now >= e.ts - PRE_WINDOW_SECS
                    && now < e.ts + POST_WINDOW_SECS
            })
        })
}

fn snapshot(events: &[Event], team: &str, now: i64) -> Snapshot {
    let (last, next, live) = pick(events, now);
    let name = events
        .iter()
        .map(|e| ours(e, team).0)
        .find(|s| s.id == team)
        .map(|s| s.name.clone());
    Snapshot {
        team_id: team.to_string(),
        name,
        last: last.cloned(),
        next: next.cloned(),
        live: live.cloned(),
        crests: BTreeMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Change detection (peeks)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Goal { ours: bool },
    KickOff,
    FullTime,
}

/// What changed between two polls of the same match. Nothing on the first
/// observation (`prev` is `None`), so no stale peeks at startup.
fn changes(prev: Option<&Event>, cur: &Event, team: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let Some(prev) = prev.filter(|p| p.id == cur.id) else {
        return out;
    };
    if prev.state == MatchState::Pre && cur.state == MatchState::In {
        out.push(Change::KickOff);
    }
    let (p_me, p_opp, _) = ours(prev, team);
    let (c_me, c_opp, _) = ours(cur, team);
    if cur.state != MatchState::Pre {
        if c_me.score.unwrap_or(0) > p_me.score.unwrap_or(0) {
            out.push(Change::Goal { ours: true });
        }
        if c_opp.score.unwrap_or(0) > p_opp.score.unwrap_or(0) {
            out.push(Change::Goal { ours: false });
        }
    }
    if prev.state == MatchState::In && cur.state == MatchState::Post {
        out.push(Change::FullTime);
    }
    out
}

fn peek_for(change: Change, ev: &Event, team: &str) -> Peek {
    let (me, opp, _) = ours(ev, team);
    let minute = minute_label(ev);
    let live_line = if minute.is_empty() {
        scoreline(ev)
    } else {
        format!("{} \u{b7} {minute}", scoreline(ev))
    };
    let (icon, accent, title, body) = match change {
        Change::Goal { ours: true } => (
            GLYPH_BALL,
            theme::ACCENT,
            format!("GOAL! {}", me.name),
            live_line,
        ),
        Change::Goal { ours: false } => (
            GLYPH_BALL,
            theme::TEXT_DIM,
            format!("{} scored", opp.name),
            live_line,
        ),
        Change::FullTime => (
            GLYPH_FLAG,
            theme::BLUE,
            "Full time".to_string(),
            scoreline(ev),
        ),
        Change::KickOff => (
            GLYPH_BALL,
            theme::WARN,
            "Kick-off".to_string(),
            format!("{} \u{2013} {}", ev.home.abbr, ev.away.abbr),
        ),
    };
    Peek {
        icon: icon.into(),
        emoji: true,
        accent,
        title,
        body,
        seconds: PEEK_SECONDS,
    }
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// Civil date (proleptic Gregorian) to days since 1970-01-01.
fn ymd_to_days(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Days since 1970-01-01 to (year, month 1-12, day 1-31).
fn days_to_ymd(days: i64) -> (i64, usize, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m as usize, d)
}

/// "2026-11-01T16:00Z" (also "...T16:00:30Z") to unix seconds.
fn parse_iso(s: &str) -> Option<i64> {
    let num = |a: usize, b: usize| -> Option<i64> { s.get(a..b)?.parse().ok() };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, min) = (num(11, 13)?, num(14, 16)?);
    let sec = if s.get(16..17) == Some(":") {
        num(17, 19)?
    } else {
        0
    };
    let ok = s.get(4..5) == Some("-")
        && s.get(7..8) == Some("-")
        && matches!(s.get(10..11), Some("T") | Some(" "))
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (0..24).contains(&hour)
        && (0..60).contains(&min)
        && (0..61).contains(&sec);
    ok.then(|| {
        ymd_to_days(year as i32, month as u32, day as u32) * 86_400 + hour * 3_600 + min * 60 + sec
    })
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Local UTC offset in seconds, rounded to the nearest 15 minutes.
fn local_offset_secs() -> i64 {
    let st = unsafe { GetLocalTime() };
    let local_secs = ymd_to_days(st.wYear as i32, st.wMonth as u32, st.wDay as u32) * 86_400
        + st.wHour as i64 * 3_600
        + st.wMinute as i64 * 60
        + st.wSecond as i64;
    let raw = local_secs - unix_now();
    let q = 900_i64;
    if raw >= 0 {
        ((raw + q / 2) / q) * q
    } else {
        ((raw - q / 2) / q) * q
    }
}

/// "01.11.2026" (local date).
fn fmt_date(ts: i64, offset: i64) -> String {
    let (y, m, d) = days_to_ymd((ts + offset).div_euclid(86_400));
    format!("{d:02}.{m:02}.{y}")
}

/// "Today 19:00", "Tomorrow 19:00" or "Sun 19:00" (local time).
fn fmt_day_time(ts: i64, now: i64, offset: i64) -> String {
    let local = ts + offset;
    let day = local.div_euclid(86_400);
    let today = (now + offset).div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let clock = format!("{:02}:{:02}", secs / 3_600, (secs / 60) % 60);
    if day == today {
        format!("Today {clock}")
    } else if day == today + 1 {
        format!("Tomorrow {clock}")
    } else {
        format!("{} {clock}", DAYS[(day + 4).rem_euclid(7) as usize])
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

fn fetch_league(league: &'static str, team: &str, fixtures: bool) -> Result<Vec<Event>, String> {
    let url = format!(
        "https://site.api.espn.com/apis/site/v2/sports/soccer/{league}/teams/{team}/schedule{}",
        if fixtures { "?fixture=true" } else { "" }
    );
    let json = http::get_json(&url)?;
    let events = json.get("events").and_then(Value::as_array);
    Ok(events
        .map(|list| list.iter().filter_map(|e| parse_event(e, league)).collect())
        .unwrap_or_default())
}

fn fetch_summary(base: &Event) -> Result<Event, String> {
    let url = format!(
        "https://site.api.espn.com/apis/site/v2/sports/soccer/{}/summary?event={}",
        base.league,
        http::encode(&base.id)
    );
    apply_summary(base, &http::get_json(&url)?).ok_or_else(|| "bad summary".to_string())
}

fn crest_key(team_id: &str) -> u64 {
    let mut h = DefaultHasher::new();
    ("football-crest", team_id).hash(&mut h);
    h.finish()
}

/// Crest PNG for an ESPN team id: from the disk cache, else downloaded (small
/// size via ESPN's image combiner) and cached.
fn load_crest(team_id: &str) -> Option<Vec<u8>> {
    const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G'];
    if team_id.is_empty() || !team_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let dir = store::local_dir().join("football");
    let path = dir.join(format!("{team_id}.png"));
    if let Ok(bytes) = fs::read(&path) {
        if bytes.starts_with(PNG_MAGIC) {
            return Some(bytes);
        }
    }
    let url = format!(
        "https://a.espncdn.com/combiner/i?img=/i/teamlogos/soccer/500/{team_id}.png&h=96&w=96"
    );
    let bytes = http::get_bytes(&url).ok().filter(|b| b.starts_with(PNG_MAGIC))?;
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&path, &bytes);
    Some(bytes)
}

/// In-memory crest cache; ids that failed (e.g. 404) are retried only rarely.
#[derive(Default)]
struct Crests {
    have: HashMap<String, Arc<Vec<u8>>>,
    missing: HashMap<String, Instant>,
}

impl Crests {
    fn get(&mut self, team_id: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(bytes) = self.have.get(team_id) {
            return Some(bytes.clone());
        }
        if self.missing.get(team_id).is_some_and(|t| t.elapsed() < CREST_RETRY) {
            return None;
        }
        match load_crest(team_id) {
            Some(bytes) => {
                let bytes = Arc::new(bytes);
                self.have.insert(team_id.to_string(), bytes.clone());
                Some(bytes)
            }
            None => {
                self.missing.insert(team_id.to_string(), Instant::now());
                None
            }
        }
    }

    /// Fills `snap.crests` for the two teams of the shown match.
    fn attach(&mut self, snap: &mut Snapshot) {
        let ids: Vec<String> = shown(snap)
            .map(|(e, _)| vec![e.home.id.clone(), e.away.id.clone()])
            .unwrap_or_default();
        for id in ids {
            if let Some(bytes) = self.get(&id) {
                snap.crests.insert(id, bytes);
            }
        }
    }
}

struct Tracker {
    team: String,
    closed: Arc<AtomicBool>,
    /// Schedule events per (league index, fixtures?); a failed request keeps the old list.
    cache: BTreeMap<(usize, bool), Vec<Event>>,
    /// Fresher live versions of events, from the summary endpoint.
    overrides: HashMap<String, Event>,
    /// The match as seen by the previous live poll.
    watched: Option<Event>,
    fetched: Option<Instant>,
    fetch_ok: bool,
    loaded: bool,
    crests: Crests,
}

impl Tracker {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn events(&self) -> Vec<Event> {
        merge(
            self.cache.values().flatten().cloned().collect(),
            &self.overrides,
        )
    }

    fn fetch_due(&self) -> bool {
        match self.fetched {
            None => true,
            Some(t) => {
                t.elapsed()
                    >= if self.fetch_ok {
                        SCHEDULE_EVERY
                    } else {
                        RETRY_EVERY
                    }
            }
        }
    }

    fn refresh_schedule(&mut self) {
        let mut any_ok = false;
        for (i, &league) in LEAGUES.iter().enumerate() {
            for fixtures in [false, true] {
                if self.is_closed() {
                    return;
                }
                // HTTP 400 is normal for a league the team is not in.
                if let Ok(list) = fetch_league(league, &self.team, fixtures) {
                    any_ok = true;
                    self.cache.insert((i, fixtures), list);
                }
            }
        }
        self.fetched = Some(Instant::now());
        self.fetch_ok = any_ok;
        self.loaded |= any_ok;
    }

    fn poll_live(&mut self, base: &Event) {
        let Ok(cur) = fetch_summary(base) else { return };
        for change in changes(self.watched.as_ref(), &cur, &self.team) {
            worker::peek(peek_for(change, &cur, &self.team));
        }
        if cur.state == MatchState::Post {
            // Pull the final result and the next fixture soon.
            self.fetched = None;
        }
        self.watched = Some(cur.clone());
        self.overrides.insert(cur.id.clone(), cur);
    }

    fn nap(&self, secs: u64) {
        for _ in 0..secs {
            if self.is_closed() {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }

    fn run(mut self, tx: Sender<Result<Snapshot, String>>, waker: Waker) {
        let mut last_msg: Option<Result<Snapshot, String>> = None;
        while !self.is_closed() {
            let now = unix_now();
            let mut events = self.events();
            let mut live_base = tracked(&events, now).cloned();
            if live_base.is_none() && self.fetch_due() {
                self.refresh_schedule();
                events = self.events();
                live_base = tracked(&events, now).cloned();
            }
            if let Some(base) = &live_base {
                self.poll_live(base);
                events = self.events();
            }
            let msg = if self.loaded {
                let mut snap = snapshot(&events, &self.team, now);
                self.crests.attach(&mut snap);
                Ok(snap)
            } else {
                Err("Couldn't load matches".to_string())
            };
            if last_msg.as_ref() != Some(&msg) {
                if tx.send(msg.clone()).is_err() {
                    return;
                }
                waker.wake();
                last_msg = Some(msg);
            }
            self.nap(if live_base.is_some() {
                LIVE_POLL_SECS
            } else {
                IDLE_POLL_SECS
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

fn hex_color(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color::rgba(
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
        1.0,
    ))
}

fn luminance(c: Color) -> f32 {
    0.299 * c.r + 0.587 * c.g + 0.114 * c.b
}

fn hsl(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::rgba(r + m, g + m, b + m, 1.0)
}

/// (badge background, text color): the team color, else its alternate color,
/// else a stable color derived from the team id. Near-white and near-black
/// colors are skipped since they vanish against the black notch / the text.
fn badge_colors(side: &Side) -> (Color, Color) {
    let usable = [&side.color, &side.alt]
        .into_iter()
        .flatten()
        .filter_map(|s| hex_color(s))
        .find(|c| (0.06..=0.8).contains(&luminance(*c)));
    let bg = usable.unwrap_or_else(|| {
        let h = side
            .id
            .bytes()
            .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        hsl((h % 360) as f32, 0.5, 0.36)
    });
    let fg = if luminance(bg) > 0.6 {
        Color::rgba(0.06, 0.06, 0.06, 1.0)
    } else {
        theme::TEXT
    };
    (bg, fg)
}

/// Team crest in `r`; without a downloaded crest, a colored badge with the
/// team abbreviation.
fn draw_crest(ctx: &mut DrawCtx, r: Rect, side: &Side, crest: Option<&Arc<Vec<u8>>>) {
    if let Some(bytes) = crest {
        ctx.image_dynamic(crest_key(&side.id), bytes, r, 1.0);
        return;
    }
    let (bg, fg) = badge_colors(side);
    ctx.fill_rect(r, r.h / 2.0, bg);
    ctx.text(
        &side.abbr,
        r,
        TextStyle::new((r.h * 0.3).clamp(9.5, 14.0), Weight::Bold, fg).align(Align::Center),
    );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

enum Load {
    Loading,
    Failed,
    Ready(Snapshot),
}

pub struct FootballPage {
    rx: Receiver<Result<Snapshot, String>>,
    closed: Arc<AtomicBool>,
    state: Load,
    keep_open: bool,
}

/// Development aid (`ADA_FOOTBALL_DEMO=1`): a fake live match plus a next and
/// a last match, with real crests, and a goal peek.
fn run_demo(tx: Sender<Result<Snapshot, String>>, waker: Waker) {
    let side = |id: &str, abbr: &str, name: &str, score| Side {
        id: id.into(),
        abbr: abbr.into(),
        name: name.into(),
        color: None,
        alt: None,
        score,
    };
    let event = |id: &str, ts, state, detail: &str, home, away| Event {
        id: id.into(),
        league: "tur.1",
        ts,
        state,
        detail: detail.into(),
        clock: detail.into(),
        home,
        away,
    };
    let now = unix_now();
    let live = event(
        "demo",
        now - 70 * 60,
        MatchState::In,
        "67'",
        side(DEFAULT_TEAM, "BES", "Besiktas", Some(2)),
        side("432", "GS", "Galatasaray", Some(1)),
    );
    let next = event(
        "demo-next",
        now + 26 * 3_600,
        MatchState::Pre,
        "",
        side("6870", "KAS", "Kasimpasa", None),
        side(DEFAULT_TEAM, "BES", "Besiktas", None),
    );
    let last = event(
        "demo-last",
        now - 6 * 86_400,
        MatchState::Post,
        "FT",
        side(DEFAULT_TEAM, "BES", "Besiktas", Some(2)),
        side("7914", "IBFK", "Istanbul BB", Some(0)),
    );
    worker::peek(peek_for(Change::Goal { ours: true }, &live, DEFAULT_TEAM));
    let mut crests = Crests::default();
    let mut snap = Snapshot {
        team_id: DEFAULT_TEAM.into(),
        name: Some("Besiktas".into()),
        last: Some(last),
        next: Some(next),
        live: Some(live),
        crests: BTreeMap::new(),
    };
    for id in ["1895", "432", "6870", "7914"] {
        if let Some(bytes) = crests.get(id) {
            snap.crests.insert(id.to_string(), bytes);
        }
    }
    let _ = tx.send(Ok(snap));
    waker.wake();
}

impl FootballPage {
    pub fn new(waker: Waker) -> Self {
        let settings = store::read_kv(SETTINGS_FILE);
        let team = settings
            .get(SETTINGS_KEY)
            .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
            .cloned()
            .unwrap_or_else(|| DEFAULT_TEAM.to_string());
        let keep_open = settings.get(KEEP_OPEN_KEY).map_or(true, |v| v != "0");
        let (tx, rx) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let tracker = Tracker {
            team,
            closed: closed.clone(),
            cache: BTreeMap::new(),
            overrides: HashMap::new(),
            watched: None,
            fetched: None,
            fetch_ok: false,
            loaded: false,
            crests: Crests::default(),
        };
        if std::env::var_os("ADA_FOOTBALL_DEMO").is_some() {
            worker::spawn("football-demo", move || run_demo(tx, waker));
        } else {
            worker::spawn("football", move || tracker.run(tx, waker));
        }
        Self {
            rx,
            closed,
            state: Load::Loading,
            keep_open,
        }
    }

    fn live(&self) -> Option<(&Snapshot, &Event)> {
        match &self.state {
            Load::Ready(s) => s.live.as_ref().map(|e| (s, e)),
            _ => None,
        }
    }
}

/// Sofascore-style match header: crests and names left/right, details centered.
fn draw_match(ctx: &mut DrawCtx, area: Rect, snap: &Snapshot, ev: &Event, kind: Shown) {
    const SIDE_W: f32 = 90.0;
    let (now, offset) = (unix_now(), local_offset_secs());
    let columns = [
        (&ev.home, Rect::new(area.x, area.y, SIDE_W, area.h)),
        (&ev.away, Rect::new(area.right() - SIDE_W, area.y, SIDE_W, area.h)),
    ];
    for (side, col) in columns {
        let crest = Rect::new(col.x + (col.w - 40.0) / 2.0, area.y + 6.0, 40.0, 40.0);
        draw_crest(ctx, crest, side, snap.crests.get(&side.id));
        ctx.text(
            &side.name,
            Rect::new(col.x + 2.0, area.y + 51.0, col.w - 4.0, 16.0),
            TextStyle::new(12.0, Weight::Medium, theme::TEXT).align(Align::Center),
        );
    }

    let mid = Rect::new(area.x + SIDE_W, area.y, (area.w - 2.0 * SIDE_W).max(0.0), area.h);
    let row = |y: f32, h: f32| Rect::new(mid.x, area.y + y, mid.w, h);
    let center = |size: f32, weight: Weight, color: Color| {
        TextStyle::new(size, weight, color).align(Align::Center)
    };
    let faint = center(10.5, Weight::Regular, theme::TEXT_FAINT);
    match kind {
        Shown::Next => {
            ctx.text(
                &fmt_date(ev.ts, offset),
                row(6.0, 22.0),
                center(15.0, Weight::Bold, theme::TEXT),
            );
            ctx.text(
                &fmt_day_time(ev.ts, now, offset),
                row(28.0, 16.0),
                center(12.0, Weight::Medium, theme::TEXT_DIM),
            );
            ctx.text(league_name(ev.league), row(46.0, 14.0), faint);
            if let Some(last) = &snap.last {
                draw_last_line(ctx, row(64.0, 14.0), last, &snap.team_id);
            }
        }
        Shown::Live | Shown::Last => {
            let score = format!(
                "{} \u{2013} {}",
                ev.home.score.unwrap_or(0),
                ev.away.score.unwrap_or(0)
            );
            ctx.text(&score, row(4.0, 34.0), center(26.0, Weight::Bold, theme::TEXT));
            let minute = minute_label(ev);
            let (label, color) = match kind {
                Shown::Live if minute == "HT" => ("\u{25CF} HT".to_string(), theme::TEXT_DIM),
                Shown::Live => (
                    format!("\u{25CF} LIVE {minute}").trim_end().to_string(),
                    theme::DANGER,
                ),
                _ => ("FT".to_string(), theme::TEXT_DIM),
            };
            ctx.text(&label, row(40.0, 16.0), center(12.0, Weight::Semibold, color));
            let comp = if kind == Shown::Last {
                format!("{} \u{b7} {}", league_name(ev.league), fmt_date(ev.ts, offset))
            } else {
                league_name(ev.league).to_string()
            };
            ctx.text(&comp, row(58.0, 14.0), faint);
        }
    }
}

/// "Last: W 2-0 Istanbul BB" with a colored result; dropped if it does not fit.
fn draw_last_line(ctx: &mut DrawCtx, r: Rect, last: &Event, team: &str) {
    let Some((res, a, b)) = result_of(last, team) else { return };
    let (_, opp, _) = ours(last, team);
    let color = match res {
        'W' => theme::ACCENT,
        'L' => theme::DANGER,
        _ => theme::TEXT_DIM,
    };
    let parts = [
        ("Last: ".to_string(), theme::TEXT_FAINT),
        (format!("{res} {a}\u{2013}{b}"), color),
        (format!(" {}", opp.name), theme::TEXT_FAINT),
    ];
    let styles: Vec<TextStyle> = parts
        .iter()
        .map(|(_, c)| TextStyle::new(10.5, Weight::Medium, *c))
        .collect();
    let widths: Vec<f32> = parts
        .iter()
        .zip(&styles)
        .map(|((t, _), st)| ctx.measure_text(t, st))
        .collect();
    let total: f32 = widths.iter().sum();
    if total > r.w {
        return;
    }
    let mut x = r.x + (r.w - total) / 2.0;
    for (((text, _), style), w) in parts.iter().zip(styles).zip(widths) {
        ctx.text(text, Rect::new(x, r.y, w + 2.0, r.h), style);
        x += w;
    }
}

impl Drop for FootballPage {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

impl Page for FootballPage {
    fn title(&self) -> String {
        match &self.state {
            Load::Ready(Snapshot { name: Some(n), .. }) => n.clone(),
            _ => "Football".to_string(),
        }
    }

    fn poll(&mut self) -> bool {
        let mut redraw = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Ok(snap) => {
                    self.state = Load::Ready(snap);
                    redraw = true;
                }
                Err(_) => {
                    // Keep the last good data; only show the error if nothing was loaded.
                    if matches!(self.state, Load::Loading) {
                        self.state = Load::Failed;
                        redraw = true;
                    }
                }
            }
        }
        redraw
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let dim = TextStyle::new(13.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center);
        let snap = match &self.state {
            Load::Loading => return ctx.text("Loading matches\u{2026}", area, dim),
            Load::Failed => return ctx.text("Couldn't load matches", area, dim),
            Load::Ready(s) => s,
        };
        match shown(snap) {
            Some((ev, kind)) => draw_match(ctx, area, snap, ev, kind),
            None => ctx.text("No matches found", area, dim),
        }
    }

    fn keep_open(&self) -> bool {
        self.keep_open && self.live().is_some()
    }

    fn activity(&self) -> Option<Activity> {
        self.live().map(|_| Activity {
            priority: ACTIVITY_PRIORITY,
            width: ACTIVITY_WIDTH,
        })
    }

    fn draw_activity(&mut self, ctx: &mut DrawCtx, notch: Rect) {
        let Some((_, ev)) = self.live() else { return };
        let cy = notch.y + theme::COLLAPSED_H / 2.0;
        let minute = minute_label(ev);
        let color = if minute == "HT" {
            theme::TEXT_DIM
        } else {
            theme::DANGER
        };
        let time_w = 48.0;
        let time = Rect::new(notch.right() - 12.0 - time_w, cy - 8.0, time_w, 16.0);
        ctx.text(
            minute,
            time,
            TextStyle::new(11.0, Weight::Semibold, color).align(Align::Right),
        );
        let left = Rect::new(
            notch.x + 12.0,
            cy - 8.0,
            (time.x - 6.0 - (notch.x + 12.0)).max(0.0),
            16.0,
        );
        ctx.text(
            &scoreline(ev),
            left,
            TextStyle::new(12.0, Weight::Bold, theme::TEXT),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn side(id: &str, abbr: &str, score: Option<u32>) -> Side {
        Side {
            id: id.into(),
            abbr: abbr.into(),
            name: abbr.into(),
            color: None,
            alt: None,
            score,
        }
    }

    fn ev(id: &str, ts: i64, state: MatchState, home: u32, away: u32) -> Event {
        Event {
            id: id.into(),
            league: "tur.1",
            ts,
            state,
            detail: String::new(),
            clock: String::new(),
            home: side("1895", "BES", Some(home)),
            away: side("2", "GS", Some(away)),
        }
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso("1970-01-01T00:00Z"), Some(0));
        // Leap day.
        assert_eq!(parse_iso("2024-02-29T12:30Z"), Some(1_709_209_800));
        assert_eq!(parse_iso("2024-02-29T12:30:15Z"), Some(1_709_209_815));
        assert_eq!(parse_iso("2026-11-01T16:00Z"), Some(1_793_548_800));
        assert_eq!(parse_iso("2026-13-01T16:00Z"), None);
        assert_eq!(parse_iso("nonsense"), None);
        assert_eq!(days_to_ymd(ymd_to_days(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn parses_string_and_object_scores() {
        let make = |home: Value, away: Value| {
            json!({
                "id": "401", "date": "2026-10-26T16:00Z",
                "competitions": [{
                    "status": { "displayClock": "90'", "type": { "state": "post", "shortDetail": "FT" } },
                    "competitors": [
                        { "homeAway": "home", "score": home,
                          "team": { "id": "1895", "abbreviation": "BES", "shortDisplayName": "Besiktas", "color": "ffffff", "alternateColor": "000000" } },
                        { "homeAway": "away", "score": away,
                          "team": { "id": "7914", "abbreviation": "IBFK", "shortDisplayName": "Istanbul BB" } }
                    ]
                }]
            })
        };
        let a = parse_event(&make(json!("2"), json!("0")), "tur.1").unwrap();
        let b = parse_event(
            &make(
                json!({ "displayValue": "2", "value": 2.0 }),
                json!({ "displayValue": "0" }),
            ),
            "tur.1",
        )
        .unwrap();
        assert_eq!(a, b);
        assert_eq!((a.home.score, a.away.score), (Some(2), Some(0)));
        assert_eq!((a.state, a.detail.as_str()), (MatchState::Post, "FT"));
        assert_eq!(a.home.color.as_deref(), Some("ffffff"));
        assert_eq!(result_of(&a, "1895"), Some(('W', 2, 0)));
        assert_eq!(result_of(&a, "7914"), Some(('L', 0, 2)));
        // A missing score is not a result.
        let c = parse_event(&make(Value::Null, json!("1")), "tur.1").unwrap();
        assert_eq!(result_of(&c, "1895"), None);
    }

    #[test]
    fn picks_last_next_live() {
        let now = 1_000_000;
        let events = vec![
            ev("a", now - 900_000, MatchState::Post, 1, 0),
            ev("b", now - 100_000, MatchState::Post, 0, 2),
            ev("c", now + 100_000, MatchState::Pre, 0, 0),
            ev("d", now + 50_000, MatchState::Pre, 0, 0),
            ev("e", now - 100, MatchState::Pre, 0, 0),
        ];
        let (last, next, live) = pick(&events, now);
        assert_eq!(last.map(|e| e.id.as_str()), Some("b"));
        assert_eq!(next.map(|e| e.id.as_str()), Some("d"));
        assert!(live.is_none());
        // "e" kicked off 100 s ago and is not over yet, so it is tracked.
        assert_eq!(tracked(&events, now).map(|e| e.id.as_str()), Some("e"));

        let mut with_live = events.clone();
        with_live.push(ev("f", now - 3_000, MatchState::In, 1, 1));
        assert_eq!(pick(&with_live, now).2.map(|e| e.id.as_str()), Some("f"));
        assert_eq!(tracked(&with_live, now).map(|e| e.id.as_str()), Some("f"));
        assert!(tracked(&events[..4], now).is_none());

        // The more advanced state wins when the same event is in both lists,
        // and a live override replaces a stale schedule entry.
        let dup = vec![
            ev("x", now, MatchState::Pre, 0, 0),
            ev("x", now, MatchState::Post, 1, 0),
        ];
        assert_eq!(merge(dup, &HashMap::new())[0].state, MatchState::Post);
        let mut over = HashMap::new();
        over.insert(
            "c".to_string(),
            ev("c", now + 100_000, MatchState::In, 1, 0),
        );
        assert_eq!(
            merge(vec![ev("c", now + 100_000, MatchState::Pre, 0, 0)], &over)[0].state,
            MatchState::In
        );
    }

    #[test]
    fn detects_goals_kickoff_and_full_time() {
        let t = "1895";
        let live = |h, a| ev("m", 0, MatchState::In, h, a);
        // First observation never peeks.
        assert!(changes(None, &live(2, 1), t).is_empty());
        assert!(changes(Some(&ev("other", 0, MatchState::In, 0, 0)), &live(2, 1), t).is_empty());
        // Our goal (we are home) and theirs.
        assert_eq!(
            changes(Some(&live(1, 1)), &live(2, 1), t),
            vec![Change::Goal { ours: true }]
        );
        assert_eq!(
            changes(Some(&live(1, 1)), &live(1, 2), t),
            vec![Change::Goal { ours: false }]
        );
        // Away perspective flips it.
        assert_eq!(
            changes(Some(&live(1, 1)), &live(2, 1), "2"),
            vec![Change::Goal { ours: false }]
        );
        // No change, and a VAR-style decrease, give nothing.
        assert!(changes(Some(&live(1, 1)), &live(1, 1), t).is_empty());
        assert!(changes(Some(&live(2, 1)), &live(1, 1), t).is_empty());
        // Kick-off and full time.
        let pre = ev("m", 0, MatchState::Pre, 0, 0);
        assert_eq!(changes(Some(&pre), &live(0, 0), t), vec![Change::KickOff]);
        let done = ev("m", 0, MatchState::Post, 2, 1);
        assert_eq!(changes(Some(&live(2, 1)), &done, t), vec![Change::FullTime]);
        assert_eq!(
            changes(Some(&live(1, 1)), &done, t),
            vec![Change::Goal { ours: true }, Change::FullTime]
        );

        let p = peek_for(Change::Goal { ours: true }, &done, t);
        assert_eq!(
            (p.title.as_str(), p.body.as_str()),
            ("GOAL! BES", "BES 2\u{2013}1 GS")
        );
    }

    #[test]
    fn formats_local_times() {
        let ts = parse_iso("2026-11-01T16:00Z").unwrap();
        let three_h = 3 * 3_600;
        assert_eq!(fmt_date(ts, three_h), "01.11.2026");
        // 22:00 UTC is already the next day in UTC+3.
        assert_eq!(fmt_date(ts + 6 * 3_600, three_h), "02.11.2026");
        assert_eq!(fmt_day_time(ts, ts - 30 * 86_400, three_h), "Sun 19:00");
        assert_eq!(fmt_day_time(ts, ts - 3_600, three_h), "Today 19:00");
        assert_eq!(fmt_day_time(ts, ts - 86_400, three_h), "Tomorrow 19:00");
    }

    #[test]
    fn shows_live_then_next_then_last() {
        let mut snap = Snapshot {
            team_id: "1895".into(),
            name: None,
            last: Some(ev("l", 1, MatchState::Post, 2, 0)),
            next: Some(ev("n", 9, MatchState::Pre, 0, 0)),
            live: Some(ev("v", 5, MatchState::In, 1, 1)),
            crests: BTreeMap::new(),
        };
        let id_kind = |s: &Snapshot| shown(s).map(|(e, k)| (e.id.clone(), k));
        assert_eq!(id_kind(&snap), Some(("v".to_string(), Shown::Live)));
        snap.live = None;
        assert_eq!(id_kind(&snap), Some(("n".to_string(), Shown::Next)));
        snap.next = None;
        assert_eq!(id_kind(&snap), Some(("l".to_string(), Shown::Last)));
        snap.last = None;
        assert!(shown(&snap).is_none());
    }

    #[test]
    fn crest_ids_are_safe_and_keys_stable() {
        assert!(load_crest("../x").is_none());
        assert!(load_crest("").is_none());
        assert_eq!(crest_key("1895"), crest_key("1895"));
        assert_ne!(crest_key("1895"), crest_key("432"));
    }
}
