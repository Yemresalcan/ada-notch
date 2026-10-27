//! Now playing (Windows media transport controls) and synced lyrics (LRCLIB).
//!
//! A worker thread talks to the system media session: it subscribes to the
//! session events (with a 1 s poll as a fallback), reads the track, timeline
//! and thumbnail, executes the transport commands sent by the UI and fetches
//! lyrics on track change. Results reach the page through an mpsc channel.
//! The collapsed notch shows a live activity (art, title, audio bars) while
//! something plays.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows::Foundation::{EventRegistrationToken, TimeSpan, TypedEventHandler};
use windows::Media::Control::{
    CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProps,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlayStatus, MediaPropertiesChangedEventArgs,
    PlaybackInfoChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::DataReader;

use crate::gfx::{Align, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::http;
use crate::pages::{Activity, Host, Mouse, Page};
use crate::theme;
use crate::worker::{self, Waker};

/// Collapsed notch width while the media activity is shown.
const ACTIVITY_W: f32 = 230.0;
/// How long the activity stays after pausing.
const PAUSE_HOLD: Duration = Duration::from_secs(10);
/// Worker poll interval (fallback next to the session events).
const POLL_EVERY: Duration = Duration::from_secs(1);
/// While the page is on screen and playing, the ticker wakes the shell this often.
const FAST_TICK: Duration = Duration::from_millis(200);
const LYRIC_LINE_H: f32 = 19.0;
const LYRIC_SCROLL_SECS: f32 = 0.35;
const MAX_THUMB_BYTES: u32 = 8 * 1024 * 1024;
const MAX_ART_TRIES: u8 = 6;
const GLYPH_MUSIC: &str = "\u{E8D6}";
const GLYPH_PREV: &str = "\u{E892}";
const GLYPH_NEXT: &str = "\u{E893}";
const GLYPH_PLAY: &str = "\u{E768}";
const GLYPH_PAUSE: &str = "\u{E769}";

// ---------------------------------------------------------------------------
// Shared data between the page, the worker and the ticker
// ---------------------------------------------------------------------------

struct Shared {
    epoch: Instant,
    /// Milliseconds since `epoch` (+1) of the last `draw`; 0 = never.
    last_draw_ms: AtomicU64,
    playing: AtomicBool,
    closed: AtomicBool,
}

impl Shared {
    fn touch(&self) {
        self.last_draw_ms.store(self.epoch.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
    }

    fn drawn_recently(&self) -> bool {
        let last = self.last_draw_ms.load(Ordering::Relaxed);
        let now = self.epoch.elapsed().as_millis() as u64 + 1;
        last != 0 && now.saturating_sub(last) < 700
    }

    /// True while the expanded page is visible and something plays, so it
    /// should redraw several times per second (progress bar, lyrics).
    fn wants_fast_redraw(&self) -> bool {
        self.playing.load(Ordering::Relaxed) && self.drawn_recently()
    }
}

#[derive(Clone)]
enum Lyrics {
    Loading,
    Missing,
    /// (seconds, text) sorted by time.
    Synced(Vec<(f32, String)>),
    Plain(Vec<String>),
}

struct Snapshot {
    key: u64,
    title: String,
    artist: String,
    album: String,
    source: String,
    playing: bool,
    /// Position in seconds at `sampled`.
    position: f32,
    /// Track length in seconds; 0 when unknown (live streams).
    duration: f32,
    sampled: Instant,
}

impl Snapshot {
    /// Position extrapolated to now while playing.
    fn position_now(&self) -> f32 {
        let mut pos = self.position;
        if self.playing {
            pos += self.sampled.elapsed().as_secs_f32();
        }
        if self.duration > 0.0 {
            pos = pos.min(self.duration);
        }
        pos.max(0.0)
    }
}

enum Msg {
    Session(Option<Snapshot>),
    Art { key: u64, bytes: Vec<u8> },
    Lyrics { key: u64, lyrics: Lyrics },
}

enum Cmd {
    Refresh,
    Toggle,
    Next,
    Prev,
}

fn track_key(title: &str, artist: &str) -> u64 {
    let mut h = DefaultHasher::new();
    title.hash(&mut h);
    artist.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// Worker: media session
// ---------------------------------------------------------------------------

struct Subscription {
    session: Session,
    id: String,
    tokens: [Option<EventRegistrationToken>; 3],
}

impl Subscription {
    fn remove(&self) {
        if let Some(t) = self.tokens[0] {
            let _ = self.session.RemoveMediaPropertiesChanged(t);
        }
        if let Some(t) = self.tokens[1] {
            let _ = self.session.RemovePlaybackInfoChanged(t);
        }
        if let Some(t) = self.tokens[2] {
            let _ = self.session.RemoveTimelinePropertiesChanged(t);
        }
    }
}

fn subscribe(session: &Session, tx: &Sender<Cmd>) -> [Option<EventRegistrationToken>; 3] {
    let t0 = tx.clone();
    let a = session
        .MediaPropertiesChanged(&TypedEventHandler::<Session, MediaPropertiesChangedEventArgs>::new(move |_, _| {
            let _ = t0.send(Cmd::Refresh);
            Ok(())
        }))
        .ok();
    let t1 = tx.clone();
    let b = session
        .PlaybackInfoChanged(&TypedEventHandler::<Session, PlaybackInfoChangedEventArgs>::new(move |_, _| {
            let _ = t1.send(Cmd::Refresh);
            Ok(())
        }))
        .ok();
    let t2 = tx.clone();
    let c = session
        .TimelinePropertiesChanged(&TypedEventHandler::<Session, TimelinePropertiesChangedEventArgs>::new(move |_, _| {
            let _ = t2.send(Cmd::Refresh);
            Ok(())
        }))
        .ok();
    [a, b, c]
}

struct WorkerState {
    sub: Option<Subscription>,
    key: Option<u64>,
    art_loaded: bool,
    art_tries: u8,
    cache: Arc<Mutex<HashMap<u64, Lyrics>>>,
}

impl WorkerState {
    /// Keeps the session event subscriptions on the current session.
    fn sync_subscription(&mut self, session: Option<&Session>, tx: &Sender<Cmd>) {
        let id = session.and_then(|s| s.SourceAppUserModelId().ok()).map(|s| s.to_string_lossy());
        let same = match (&self.sub, &id) {
            (Some(sub), Some(id)) => &sub.id == id,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if let Some(sub) = self.sub.take() {
            sub.remove();
        }
        if let (Some(session), Some(id)) = (session, id) {
            let tokens = subscribe(session, tx);
            self.sub = Some(Subscription { session: session.clone(), id, tokens });
        }
    }
}

fn run_worker(cmd_rx: Receiver<Cmd>, cmd_tx: Sender<Cmd>, out: Sender<Msg>, shared: Arc<Shared>, waker: Waker) {
    let manager = loop {
        if shared.closed.load(Ordering::Relaxed) {
            return;
        }
        match Manager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => break m,
            Err(_) => {
                thread::sleep(Duration::from_secs(5));
            }
        }
    };

    let tx = cmd_tx.clone();
    let manager_token = manager
        .CurrentSessionChanged(&TypedEventHandler::<Manager, CurrentSessionChangedEventArgs>::new(move |_, _| {
            let _ = tx.send(Cmd::Refresh);
            Ok(())
        }))
        .ok();

    let mut state = WorkerState {
        sub: None,
        key: None,
        art_loaded: false,
        art_tries: 0,
        cache: Arc::new(Mutex::new(HashMap::new())),
    };

    loop {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        let mut cmds = Vec::new();
        match cmd_rx.recv_timeout(POLL_EVERY) {
            Ok(c) => cmds.push(c),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(c) = cmd_rx.try_recv() {
            cmds.push(c);
        }
        for cmd in cmds {
            run_control(&manager, cmd);
        }
        if !refresh(&mut state, &manager, &cmd_tx, &out, &waker) {
            break;
        }
    }

    if let Some(sub) = state.sub.take() {
        sub.remove();
    }
    if let Some(token) = manager_token {
        let _ = manager.RemoveCurrentSessionChanged(token);
    }
}

fn run_control(manager: &Manager, cmd: Cmd) {
    if matches!(cmd, Cmd::Refresh) {
        return;
    }
    let Ok(session) = manager.GetCurrentSession() else { return };
    match cmd {
        Cmd::Toggle => {
            if let Ok(op) = session.TryTogglePlayPauseAsync() {
                let _ = op.get();
            }
        }
        Cmd::Next => {
            if let Ok(op) = session.TrySkipNextAsync() {
                let _ = op.get();
            }
        }
        Cmd::Prev => {
            if let Ok(op) = session.TrySkipPreviousAsync() {
                let _ = op.get();
            }
        }
        Cmd::Refresh => {}
    }
}

/// Reads the current session and sends updates. Returns false when the page is gone.
fn refresh(state: &mut WorkerState, manager: &Manager, cmd_tx: &Sender<Cmd>, out: &Sender<Msg>, waker: &Waker) -> bool {
    let session = manager.GetCurrentSession().ok();
    state.sync_subscription(session.as_ref(), cmd_tx);

    let Some(session) = session else {
        state.key = None;
        let alive = out.send(Msg::Session(None)).is_ok();
        waker.wake();
        return alive;
    };

    let props = match session.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
        Ok(p) => p,
        // Transient failure: keep whatever the page shows.
        Err(_) => return true,
    };
    let text = |r: windows::core::Result<windows::core::HSTRING>| r.map(|s| s.to_string_lossy()).unwrap_or_default();
    let title = text(props.Title());
    let artist = text(props.Artist());
    let album = text(props.AlbumTitle());
    let source = session.SourceAppUserModelId().map(|s| s.to_string_lossy()).unwrap_or_default();
    let playing = session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .map(|s| s == PlayStatus::Playing)
        .unwrap_or(false);
    let (position, duration) = read_timeline(&session, playing);
    let key = track_key(&title, &artist);
    let new_track = state.key != Some(key);

    let snapshot = Snapshot {
        key,
        title: title.clone(),
        artist: artist.clone(),
        album: album.clone(),
        source,
        playing,
        position,
        duration,
        sampled: Instant::now(),
    };
    if out.send(Msg::Session(Some(snapshot))).is_err() {
        return false;
    }

    if new_track {
        state.key = Some(key);
        state.art_loaded = false;
        state.art_tries = 0;
        start_lyrics(state, key, title, artist, album, duration, out, waker);
    }

    if !state.art_loaded && state.art_tries < MAX_ART_TRIES {
        state.art_tries += 1;
        if let Some(bytes) = read_thumbnail(&props) {
            state.art_loaded = true;
            if out.send(Msg::Art { key, bytes }).is_err() {
                return false;
            }
        }
    }

    waker.wake();
    true
}

/// (position, duration) in seconds. While playing, the position is advanced by
/// the time since the app last reported it.
fn read_timeline(session: &Session, playing: bool) -> (f32, f32) {
    let Ok(tl) = session.GetTimelineProperties() else { return (0.0, 0.0) };
    let secs = |r: windows::core::Result<TimeSpan>| r.map(|t| t.Duration as f64 / 1e7).unwrap_or(0.0);
    let start = secs(tl.StartTime());
    let end = secs(tl.EndTime());
    let mut pos = secs(tl.Position()) - start;
    let dur = (end - start).max(0.0);
    if playing {
        if let Ok(updated) = tl.LastUpdatedTime() {
            let updated = updated.UniversalTime as f64 / 1e7;
            if updated > 0.0 {
                // FILETIME epoch (1601) is 11 644 473 600 s before the Unix epoch.
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
                    + 11_644_473_600.0;
                let age = now - updated;
                if (0.0..=600.0).contains(&age) {
                    pos += age;
                }
            }
        }
    }
    pos = if dur > 0.0 { pos.clamp(0.0, dur) } else { pos.max(0.0) };
    (pos as f32, dur as f32)
}

fn read_thumbnail(props: &MediaProps) -> Option<Vec<u8>> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok()?;
    if size == 0 || size > MAX_THUMB_BYTES {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader.LoadAsync(size).ok()?.get().ok()?;
    if loaded == 0 {
        return None;
    }
    let mut buf = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

// ---------------------------------------------------------------------------
// Lyrics (LRCLIB)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn start_lyrics(
    state: &WorkerState,
    key: u64,
    title: String,
    artist: String,
    album: String,
    duration: f32,
    out: &Sender<Msg>,
    waker: &Waker,
) {
    if title.is_empty() {
        let _ = out.send(Msg::Lyrics { key, lyrics: Lyrics::Missing });
        return;
    }
    let cached = state.cache.lock().ok().and_then(|c| c.get(&key).cloned());
    if let Some(lyrics) = cached {
        let _ = out.send(Msg::Lyrics { key, lyrics });
        return;
    }
    let cache = state.cache.clone();
    let out = out.clone();
    let waker = *waker;
    worker::spawn("media-lyrics", move || {
        let (lyrics, cacheable) = fetch_lyrics(&title, &artist, &album, duration);
        if cacheable {
            if let Ok(mut c) = cache.lock() {
                if c.len() > 64 {
                    c.clear();
                }
                c.insert(key, lyrics.clone());
            }
        }
        let _ = out.send(Msg::Lyrics { key, lyrics });
        waker.wake();
    });
}

/// Returns the lyrics and whether the answer is definitive (cacheable).
fn fetch_lyrics(title: &str, artist: &str, album: &str, duration: f32) -> (Lyrics, bool) {
    let mut url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}",
        http::encode(title),
        http::encode(artist)
    );
    if !album.is_empty() {
        url.push_str(&format!("&album_name={}", http::encode(album)));
    }
    if duration >= 1.0 {
        url.push_str(&format!("&duration={}", duration.round() as u32));
    }
    let mut definitive = false;
    if let Ok(value) = http::get_json(&url) {
        definitive = true;
        if let Some(lyrics) = lyrics_from_value(&value) {
            return (lyrics, true);
        }
    }

    let url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        http::encode(title),
        http::encode(artist)
    );
    if let Ok(value) = http::get_json(&url) {
        definitive = true;
        if let Some(items) = value.as_array() {
            // First result with synced lyrics, else the first with plain ones.
            let mut plain = None;
            for item in items {
                match lyrics_from_value(item) {
                    Some(l @ Lyrics::Synced(_)) => return (l, true),
                    Some(l) => {
                        if plain.is_none() {
                            plain = Some(l);
                        }
                    }
                    None => {}
                }
            }
            if let Some(l) = plain {
                return (l, true);
            }
        }
    }
    (Lyrics::Missing, definitive)
}

fn lyrics_from_value(v: &Value) -> Option<Lyrics> {
    if let Some(s) = v.get("syncedLyrics").and_then(Value::as_str) {
        let lines = parse_lrc(s);
        if !lines.is_empty() {
            return Some(Lyrics::Synced(lines));
        }
    }
    if let Some(p) = v.get("plainLyrics").and_then(Value::as_str) {
        let mut lines: Vec<String> = p.lines().map(|l| l.trim().to_string()).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let first = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
        lines.drain(..first);
        if !lines.is_empty() {
            return Some(Lyrics::Plain(lines));
        }
    }
    None
}

/// Parses `[mm:ss.xx] text` lines (several timestamps per line allowed);
/// metadata tags such as `[ar:..]` are skipped.
fn parse_lrc(text: &str) -> Vec<(f32, String)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_timestamp(&stripped[..end]) {
                Some(t) => {
                    times.push(t);
                    rest = &stripped[end + 1..];
                }
                None => break,
            }
        }
        if times.is_empty() {
            continue;
        }
        let line = rest.trim().to_string();
        for t in times {
            out.push((t, line.clone()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn parse_timestamp(s: &str) -> Option<f32> {
    let (m, sec) = s.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let sec: f32 = sec.trim().parse().ok()?;
    if !sec.is_finite() || sec < 0.0 {
        return None;
    }
    Some(m as f32 * 60.0 + sec)
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Art,
    Prev,
    Play,
    Next,
    LyricsToggle,
}

struct Layout {
    art: Rect,
    title: Rect,
    artist: Rect,
    bar: Rect,
    times: Rect,
    /// Lyrics replace title/artist/progress in this rect.
    lyrics: Rect,
    row: Rect,
    prev: Rect,
    play: Rect,
    next: Rect,
    pill: Rect,
}

fn layout(area: Rect) -> Layout {
    let art = Rect::new(area.x, area.y + 12.0, 64.0, 64.0);
    let body = Rect::new(area.x + 76.0, area.y, (area.w - 76.0).max(0.0), area.h);
    let row = Rect::new(body.x, area.y + 60.0, body.w, 24.0);
    let cx = body.x + (body.w - 52.0).max(0.0) / 2.0;
    let button = |center: f32| Rect::new(center - 13.0, row.y, 26.0, 24.0);
    Layout {
        art,
        title: Rect::new(body.x, area.y + 2.0, body.w, 18.0),
        artist: Rect::new(body.x, area.y + 20.0, body.w, 16.0),
        bar: Rect::new(body.x, area.y + 42.0, body.w, 3.0),
        times: Rect::new(body.x, area.y + 46.0, body.w, 12.0),
        lyrics: Rect::new(body.x, area.y, body.w, 58.0),
        row,
        prev: button(cx - 34.0),
        play: button(cx),
        next: button(cx + 34.0),
        pill: Rect::new(body.right() - 44.0, row.y + 3.0, 44.0, 18.0),
    }
}

pub struct MediaPage {
    rx: Receiver<Msg>,
    cmd_tx: Sender<Cmd>,
    shared: Arc<Shared>,
    snap: Option<Snapshot>,
    art: Option<(u64, Vec<u8>)>,
    lyrics: Lyrics,
    show_lyrics: bool,
    plain_scroll: usize,
    hover: Option<Hit>,
    /// Set when playback stops; the activity lingers for `PAUSE_HOLD`.
    paused_since: Option<Instant>,
    epoch: Instant,
}

impl MediaPage {
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            epoch: Instant::now(),
            last_draw_ms: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });

        {
            let shared = shared.clone();
            let cmd_tx = cmd_tx.clone();
            worker::spawn("media", move || run_worker(cmd_rx, cmd_tx, tx, shared, waker));
        }
        {
            let shared = shared.clone();
            worker::spawn("media-ticker", move || loop {
                thread::sleep(FAST_TICK);
                if shared.closed.load(Ordering::Relaxed) {
                    break;
                }
                if shared.wants_fast_redraw() {
                    waker.wake();
                }
            });
        }

        Self {
            rx,
            cmd_tx,
            shared,
            snap: None,
            art: None,
            lyrics: Lyrics::Loading,
            show_lyrics: false,
            plain_scroll: 0,
            hover: None,
            paused_since: None,
            epoch: Instant::now(),
        }
    }

    fn apply_session(&mut self, new: Option<Snapshot>) -> bool {
        let old = self.snap.take();
        let mut changed = old.is_some() != new.is_some();
        let old_playing = old.as_ref().is_some_and(|s| s.playing);
        let new_playing = new.as_ref().is_some_and(|s| s.playing);

        if let Some(n) = &new {
            match &old {
                Some(o) => {
                    changed |= o.key != n.key
                        || o.title != n.title
                        || o.artist != n.artist
                        || o.album != n.album
                        || o.playing != n.playing
                        || (o.duration - n.duration).abs() > 0.5
                        || (o.position_now() - n.position).abs() > 1.0;
                    if o.key != n.key {
                        self.art = None;
                        self.lyrics = Lyrics::Loading;
                        self.plain_scroll = 0;
                    }
                }
                None => {
                    self.art = None;
                    self.lyrics = Lyrics::Loading;
                    self.plain_scroll = 0;
                }
            }
        } else {
            self.art = None;
            self.lyrics = Lyrics::Loading;
            self.plain_scroll = 0;
            self.hover = None;
        }

        if new_playing {
            self.paused_since = None;
        } else if old_playing {
            self.paused_since = Some(Instant::now());
        } else if new.is_none() {
            self.paused_since = None;
        }
        self.shared.playing.store(new_playing, Ordering::Relaxed);
        self.snap = new;
        changed
    }

    fn has_track(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| !s.title.is_empty() || !s.artist.is_empty())
    }

    fn hit(&self, p: Point, area: Rect) -> Option<Hit> {
        if !self.has_track() {
            return None;
        }
        let l = layout(area);
        if l.pill.contains(p) {
            Some(Hit::LyricsToggle)
        } else if l.play.contains(p) {
            Some(Hit::Play)
        } else if l.prev.contains(p) {
            Some(Hit::Prev)
        } else if l.next.contains(p) {
            Some(Hit::Next)
        } else if l.art.contains(p) {
            Some(Hit::Art)
        } else {
            None
        }
    }

    /// Flips the play state immediately; the next snapshot corrects it if needed.
    fn toggle_optimistic(&mut self) {
        let Some(s) = self.snap.as_mut() else { return };
        s.position = s.position_now();
        s.sampled = Instant::now();
        s.playing = !s.playing;
        if s.playing {
            self.paused_since = None;
        } else {
            self.paused_since = Some(Instant::now());
        }
        self.shared.playing.store(s.playing, Ordering::Relaxed);
    }

    /// Album art (or a placeholder) in `r` with rounded corners.
    fn draw_art(&self, ctx: &mut DrawCtx, r: Rect, radius: f32) {
        match &self.art {
            Some((key, bytes)) => {
                ctx.image_dynamic(*key, bytes, r, 1.0);
                // Round the corners by painting the notch background over them.
                let w = radius * 0.42 + 0.6;
                ctx.stroke_rect(r.inset(-w, -w), radius + w / 2.0, theme::BG, w, false);
            }
            None => {
                ctx.fill_rect(r, radius, theme::CARD);
                ctx.text(GLYPH_MUSIC, r, TextStyle::icon((r.h * 0.42).max(8.0), theme::TEXT_FAINT));
            }
        }
    }

    fn draw_lyrics(&self, ctx: &mut DrawCtx, view: Rect, pos: f32) {
        let state_style = TextStyle::new(12.0, Weight::Medium, theme::TEXT_FAINT).align(Align::Center);
        match &self.lyrics {
            Lyrics::Loading => ctx.text("Loading lyrics…", view, state_style),
            Lyrics::Missing => ctx.text("Lyrics not available", view, state_style),
            Lyrics::Synced(lines) => {
                let count = lines.len() as i32;
                let k = lines.partition_point(|(t, _)| *t <= pos) as i32 - 1;
                let since = if k >= 0 { pos - lines[k as usize].0 } else { f32::MAX };
                let e = ease_out_cubic((since / LYRIC_SCROLL_SECS).clamp(0.0, 1.0));
                let scroll = (k - 1) as f32 + e;
                let cy = view.y + view.h / 2.0;
                let lo = (scroll.floor() as i32 - 2).max(0);
                let hi = (scroll.ceil() as i32 + 2).min(count - 1);
                ctx.push_clip(view);
                for i in lo..=hi {
                    let rel = i as f32 - scroll;
                    let d = rel.abs();
                    let alpha = (1.0 - 0.62 * d.min(1.0)) * (1.0 - (d - 1.0).max(0.0) * 0.5).max(0.0);
                    if alpha <= 0.01 {
                        continue;
                    }
                    let text = &lines[i as usize].1;
                    let text = if text.is_empty() { "\u{266A}" } else { text.as_str() };
                    let style = if d < 0.5 {
                        TextStyle::new(13.0, Weight::Bold, theme::TEXT.with_alpha(alpha))
                    } else {
                        TextStyle::new(12.0, Weight::Medium, theme::TEXT.with_alpha(alpha))
                    };
                    ctx.text(text, Rect::new(view.x, cy + rel * LYRIC_LINE_H - 10.0, view.w, 20.0), style);
                }
                ctx.pop_clip();
            }
            Lyrics::Plain(lines) => {
                ctx.push_clip(view);
                for (n, line) in lines.iter().skip(self.plain_scroll).take(4).enumerate() {
                    let r = Rect::new(view.x, view.y + 10.0 + n as f32 * LYRIC_LINE_H - 10.0, view.w, 20.0);
                    ctx.text(line, r, TextStyle::new(12.0, Weight::Medium, theme::TEXT_DIM));
                }
                ctx.pop_clip();
            }
        }
    }

    fn draw_button(&self, ctx: &mut DrawCtx, r: Rect, hit: Hit, glyph: &str, size: f32) {
        if self.hover == Some(hit) {
            ctx.fill_rect(r, 12.0, theme::CARD_HOVER);
        }
        ctx.text(glyph, r, TextStyle::icon(size, theme::TEXT));
    }
}

impl Drop for MediaPage {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn fmt_time(secs: f32) -> String {
    let t = secs.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

/// "Spotify.exe" / "Pkg_x!App" -> readable app name.
fn source_label(id: &str) -> String {
    let name = id.rsplit('!').next().unwrap_or(id);
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

impl Page for MediaPage {
    fn title(&self) -> String {
        "Media".to_string()
    }

    fn poll(&mut self) -> bool {
        let mut redraw = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Session(s) => redraw |= self.apply_session(s),
                Msg::Art { key, bytes } => {
                    if self.snap.as_ref().is_some_and(|s| s.key == key) {
                        self.art = Some((key, bytes));
                        redraw = true;
                    }
                }
                Msg::Lyrics { key, lyrics } => {
                    if self.snap.as_ref().is_some_and(|s| s.key == key) {
                        self.lyrics = lyrics;
                        self.plain_scroll = 0;
                        redraw = true;
                    }
                }
            }
        }
        redraw || self.shared.wants_fast_redraw()
    }

    fn tick(&mut self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.playing)
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        self.shared.touch();
        let s = match &self.snap {
            Some(s) if self.has_track() => s,
            _ => {
                ctx.text(GLYPH_MUSIC, Rect::new(area.x, area.y + 14.0, area.w, 30.0), TextStyle::icon(24.0, theme::TEXT_FAINT));
                ctx.text(
                    "Nothing playing",
                    Rect::new(area.x, area.y + 46.0, area.w, 22.0),
                    TextStyle::new(13.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
                );
                return;
            }
        };
        let l = layout(area);
        let pos = s.position_now();

        // Art; hover hint for the lyrics toggle.
        self.draw_art(ctx, l.art, 12.0);
        if self.hover == Some(Hit::Art) {
            ctx.stroke_rect(l.art, 12.0, theme::BORDER, 1.0, false);
        }

        if self.show_lyrics {
            self.draw_lyrics(ctx, l.lyrics, pos);
            ctx.text("LRCLIB", Rect::new(l.row.x, l.row.y, 56.0, l.row.h), TextStyle::new(9.0, Weight::Medium, theme::TEXT_FAINT));
        } else {
            ctx.text(
                if s.title.is_empty() { "Unknown title" } else { &s.title },
                l.title,
                TextStyle::new(14.0, Weight::Bold, theme::TEXT),
            );
            let mut artist = if s.artist.is_empty() { source_label(&s.source) } else { s.artist.clone() };
            if !s.album.is_empty() && s.album != s.title {
                if artist.is_empty() {
                    artist = s.album.clone();
                } else {
                    artist = format!("{artist} · {}", s.album);
                }
            }
            ctx.text(&artist, l.artist, TextStyle::new(12.0, Weight::Regular, theme::TEXT_DIM));

            ctx.fill_rect(l.bar, 1.5, theme::CARD_HOVER);
            if s.duration > 0.0 {
                let frac = (pos / s.duration).clamp(0.0, 1.0);
                if frac > 0.0 {
                    ctx.fill_rect(Rect::new(l.bar.x, l.bar.y, (l.bar.w * frac).max(3.0), l.bar.h), 1.5, theme::TEXT);
                }
                let time_style = TextStyle::new(10.0, Weight::Medium, theme::TEXT_FAINT);
                ctx.text(&fmt_time(pos), l.times, time_style);
                ctx.text(&format!("-{}", fmt_time(s.duration - pos)), l.times, time_style.align(Align::Right));
            } else {
                ctx.text(&fmt_time(pos), l.times, TextStyle::new(10.0, Weight::Medium, theme::TEXT_FAINT));
            }
        }

        // Transport controls and the lyrics pill.
        self.draw_button(ctx, l.prev, Hit::Prev, GLYPH_PREV, 12.0);
        self.draw_button(ctx, l.play, Hit::Play, if s.playing { GLYPH_PAUSE } else { GLYPH_PLAY }, 16.0);
        self.draw_button(ctx, l.next, Hit::Next, GLYPH_NEXT, 12.0);

        let pill_bg = if self.show_lyrics {
            theme::TEXT
        } else if self.hover == Some(Hit::LyricsToggle) {
            theme::CARD_HOVER
        } else {
            theme::CARD
        };
        let pill_fg = if self.show_lyrics { theme::BG } else { theme::TEXT_DIM };
        ctx.fill_rect(l.pill, 9.0, pill_bg);
        ctx.text("Lyrics", l.pill, TextStyle::new(10.0, Weight::Semibold, pill_fg).align(Align::Center));
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
            Mouse::Down(p) | Mouse::DoubleClick(p) => match self.hit(p, area) {
                Some(Hit::Art) | Some(Hit::LyricsToggle) => {
                    self.show_lyrics = !self.show_lyrics;
                    true
                }
                Some(Hit::Play) => {
                    let _ = self.cmd_tx.send(Cmd::Toggle);
                    self.toggle_optimistic();
                    true
                }
                Some(Hit::Prev) => {
                    let _ = self.cmd_tx.send(Cmd::Prev);
                    true
                }
                Some(Hit::Next) => {
                    let _ = self.cmd_tx.send(Cmd::Next);
                    true
                }
                None => false,
            },
            Mouse::Wheel(p, delta) => {
                if !self.show_lyrics || !area.contains(p) {
                    return false;
                }
                if let Lyrics::Plain(lines) = &self.lyrics {
                    let max = lines.len().saturating_sub(3);
                    let before = self.plain_scroll;
                    let next = self.plain_scroll as i32 - delta.signum() as i32;
                    self.plain_scroll = next.clamp(0, max as i32) as usize;
                    return self.plain_scroll != before;
                }
                false
            }
            Mouse::Up(_) => false,
        }
    }

    fn activity(&self) -> Option<Activity> {
        let s = self.snap.as_ref()?;
        if s.title.is_empty() && s.artist.is_empty() {
            return None;
        }
        let active = s.playing || self.paused_since.is_some_and(|t| t.elapsed() < PAUSE_HOLD);
        active.then_some(Activity { priority: 30, width: ACTIVITY_W })
    }

    fn draw_activity(&mut self, ctx: &mut DrawCtx, notch: Rect) {
        let Some(s) = &self.snap else { return };
        let cy = notch.y + theme::COLLAPSED_H / 2.0;

        let art = Rect::new(notch.x + 11.0, cy - 10.0, 20.0, 20.0);
        self.draw_art(ctx, art, 5.0);

        // Four audio bars, flat while paused.
        const BARS: usize = 4;
        const BAR_W: f32 = 2.5;
        const BAR_GAP: f32 = 2.0;
        const MAX_H: f32 = 14.0;
        const MIN_H: f32 = 2.5;
        let bars_w = BARS as f32 * BAR_W + (BARS as f32 - 1.0) * BAR_GAP;
        let bars_x = notch.right() - 14.0 - bars_w;
        let t = self.epoch.elapsed().as_secs_f32();
        const SPEED: [f32; BARS] = [5.3, 6.7, 4.4, 7.5];
        const PHASE: [f32; BARS] = [0.0, 1.7, 3.1, 4.6];
        for i in 0..BARS {
            let h = if s.playing {
                let a = t * SPEED[i] + PHASE[i];
                let v = 0.5 + 0.5 * (0.65 * a.sin() + 0.35 * (a * 1.9 + PHASE[i] * 2.0).sin());
                MIN_H + (MAX_H - MIN_H) * v
            } else {
                MIN_H
            };
            let x = bars_x + i as f32 * (BAR_W + BAR_GAP);
            ctx.fill_rect(Rect::new(x, cy - h / 2.0, BAR_W, h), BAR_W / 2.0, theme::TEXT);
        }

        let title_x = art.right() + 8.0;
        let title_w = bars_x - 8.0 - title_x;
        if title_w > 24.0 && !s.title.is_empty() {
            ctx.text(
                &s.title,
                Rect::new(title_x, cy - 8.0, title_w, 16.0),
                TextStyle::new(11.0, Weight::Medium, theme::TEXT_DIM),
            );
        }
    }

    fn activity_animating(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.playing)
    }
}
