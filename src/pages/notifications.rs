//! Notifications page: mirrors Windows toast notifications into the notch.
//!
//! A worker thread asks `UserNotificationListener` for access, then watches
//! the notification list (the `NotificationChanged` event when it works for
//! this unpackaged exe, otherwise a 2 s poll) and diffs by notification id.
//! New toasts are shown as a peek and sent to the page (last 20 are kept).
//! Notifications that already existed at startup are listed without a peek.

use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds};

use crate::gfx::{Align, DrawCtx, Rect, TextStyle, Weight};
use crate::pages::{Host, Mouse, Page};
use crate::theme;
use crate::worker::{self, Peek, Waker};

const MAX_ITEMS: usize = 20;
const POLL_EVERY: Duration = Duration::from_secs(2);
/// With a working `NotificationChanged` event, polling is only a safety net.
const SAFETY_POLL_EVERY: Duration = Duration::from_secs(30);
/// Seconds between the Windows FILETIME epoch (1601) and the Unix epoch.
const FILETIME_TO_UNIX_SECS: i64 = 11_644_473_600;

const TOP_H: f32 = 18.0;
const GAP: f32 = 2.0;
const ROW_H: f32 = 36.0;
const CLEAR_W: f32 = 44.0;
const CLEAR_H: f32 = 16.0;

struct Item {
    app: String,
    title: String,
    body: String,
    /// Unix seconds.
    time: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Checking,
    Active,
    Unsupported,
    Denied,
}

enum Msg {
    /// Liveness probe from the worker; ignored by the page.
    Ping,
    Status(Status),
    New(Item),
}

pub struct NotificationsPage {
    rx: Receiver<Msg>,
    status: Status,
    /// Newest first.
    items: Vec<Item>,
    scroll: f32,
    list_h: f32,
    hover_clear: bool,
    /// Minute the relative times were last drawn for (to refresh labels).
    last_minute: i64,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn layout(area: Rect) -> (Rect, Rect, Rect) {
    let (top, rest) = area.split_top(TOP_H);
    let clear = Rect::new(top.right() - CLEAR_W, top.y + (top.h - CLEAR_H) / 2.0, CLEAR_W, CLEAR_H);
    let list = Rect::new(rest.x, rest.y + GAP, rest.w, (rest.h - GAP).max(0.0));
    (top, clear, list)
}

/// "now", "5m", "3h", "2d".
fn relative(time: i64, now: i64) -> String {
    let secs = (now - time).max(0);
    match secs {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

impl NotificationsPage {
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = channel();
        worker::spawn("notifications", move || watch(tx, waker));
        Self {
            rx,
            status: Status::Checking,
            items: Vec::new(),
            scroll: 0.0,
            list_h: ROW_H,
            hover_clear: false,
            last_minute: now_secs() / 60,
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.items.len() as f32 * ROW_H - self.list_h).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }
}

impl Page for NotificationsPage {
    fn title(&self) -> String {
        "Notifications".to_string()
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Ping => continue,
                Msg::Status(status) => {
                    changed = true;
                    self.status = status;
                }
                Msg::New(item) => {
                    changed = true;
                    self.items.push(item);
                    // Stable sort keeps arrival order for equal times.
                    self.items.sort_by(|a, b| b.time.cmp(&a.time));
                    self.items.truncate(MAX_ITEMS);
                }
            }
        }
        if changed {
            self.clamp_scroll();
        }
        changed
    }

    fn tick(&mut self) -> bool {
        let minute = now_secs() / 60;
        if minute != self.last_minute {
            self.last_minute = minute;
            return !self.items.is_empty();
        }
        false
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let message = match self.status {
            Status::Checking => Some("Checking notification access\u{2026}"),
            Status::Unsupported => Some("Not supported on this PC"),
            Status::Denied => Some("Access denied \u{2014} enable in Windows Settings \u{203A} Privacy \u{203A} Notifications"),
            Status::Active => None,
        };
        if let Some(message) = message {
            let style = TextStyle::new(12.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Center);
            ctx.text(message, area, style);
            return;
        }

        let (top, clear, list) = layout(area);
        self.list_h = list.h;
        self.clamp_scroll();

        let count = self.items.len();
        let summary = match count {
            0 => String::new(),
            1 => "1 recent".to_string(),
            n => format!("{n} recent"),
        };
        ctx.text(&summary, Rect::new(top.x + 2.0, top.y, (top.w - CLEAR_W - 6.0).max(0.0), top.h), TextStyle::new(10.0, Weight::Regular, theme::TEXT_FAINT));
        if count > 0 {
            ctx.fill_rect(clear, CLEAR_H / 2.0, if self.hover_clear { theme::CARD_HOVER } else { theme::CARD });
            ctx.text("Clear", clear, TextStyle::new(10.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center));
        }

        if self.items.is_empty() {
            ctx.text("No notifications", list, TextStyle::new(12.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Center));
            return;
        }

        let now = now_secs();
        ctx.push_clip(list);
        let first = (self.scroll / ROW_H).floor().max(0.0) as usize;
        for i in first..self.items.len() {
            let y = list.y + i as f32 * ROW_H - self.scroll;
            if y >= list.bottom() {
                break;
            }
            let item = &self.items[i];
            let row = Rect::new(list.x, y, list.w, ROW_H - 2.0);
            ctx.fill_rect(row, 8.0, theme::CARD);

            let inner = row.inset(8.0, 1.0);
            let time_w = 28.0;
            let line1 = Rect::new(inner.x, inner.y, inner.w, 11.0);
            ctx.text(
                &item.app,
                Rect::new(line1.x, line1.y, (line1.w - time_w - 4.0).max(0.0), line1.h),
                TextStyle::new(10.0, Weight::Regular, theme::TEXT_FAINT),
            );
            ctx.text(
                &relative(item.time, now),
                Rect::new(line1.right() - time_w, line1.y, time_w, line1.h),
                TextStyle::new(10.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Right),
            );
            ctx.text(
                &item.title,
                Rect::new(inner.x, inner.y + 10.0, inner.w, 14.0),
                TextStyle::new(12.5, Weight::Bold, theme::TEXT),
            );
            if !item.body.is_empty() {
                ctx.text(
                    &item.body,
                    Rect::new(inner.x, inner.y + 22.0, inner.w, 12.0),
                    TextStyle::new(11.5, Weight::Regular, theme::TEXT_DIM),
                );
            }
        }
        ctx.pop_clip();
    }

    fn on_mouse(&mut self, ev: Mouse, area: Rect, _host: &mut Host) -> bool {
        if self.status != Status::Active {
            return false;
        }
        let (_, clear, list) = layout(area);
        self.list_h = list.h;
        match ev {
            Mouse::Move(p) => {
                let hover = !self.items.is_empty() && clear.contains(p);
                let changed = hover != self.hover_clear;
                self.hover_clear = hover;
                changed
            }
            Mouse::Leave => std::mem::take(&mut self.hover_clear),
            Mouse::Down(p) => {
                if !self.items.is_empty() && clear.contains(p) {
                    self.items.clear();
                    self.scroll = 0.0;
                    self.hover_clear = false;
                    true
                } else {
                    false
                }
            }
            Mouse::Wheel(p, delta) => {
                if !area.contains(p) {
                    return false;
                }
                let before = self.scroll;
                self.scroll -= delta * ROW_H;
                self.clamp_scroll();
                self.scroll != before
            }
            Mouse::DoubleClick(_) | Mouse::Up(_) => false,
        }
    }
}

// ---- worker ----

fn report(tx: &Sender<Msg>, waker: Waker, status: Status) {
    let _ = tx.send(Msg::Status(status));
    waker.wake();
}

/// Unregisters the `NotificationChanged` handler when the worker ends.
struct EventGuard<'a> {
    listener: &'a UserNotificationListener,
    token: windows::Foundation::EventRegistrationToken,
}

impl Drop for EventGuard<'_> {
    fn drop(&mut self) {
        let _ = self.listener.RemoveNotificationChanged(self.token);
    }
}

/// Worker body: request access, then forward new toasts until the page is dropped.
fn watch(tx: Sender<Msg>, waker: Waker) {
    let listener = match UserNotificationListener::Current() {
        Ok(l) => l,
        Err(_) => return report(&tx, waker, Status::Unsupported),
    };
    let access = listener.RequestAccessAsync().and_then(|op| op.get());
    match access {
        Ok(UserNotificationListenerAccessStatus::Allowed) => {}
        Ok(UserNotificationListenerAccessStatus::Denied) => return report(&tx, waker, Status::Denied),
        // Unspecified (or an error): an unpackaged exe usually cannot be prompted.
        Ok(_) | Err(_) => return report(&tx, waker, Status::Unsupported),
    }

    // Try the change event; it often fails for unpackaged apps, then we poll.
    let (kick_tx, kick_rx) = channel::<()>();
    let event_token = {
        let handler = windows::Foundation::TypedEventHandler::<UserNotificationListener, windows::UI::Notifications::UserNotificationChangedEventArgs>::new(move |_, _| {
            let _ = kick_tx.send(());
            Ok(())
        });
        listener.NotificationChanged(&handler).ok()
    };
    let wait = if event_token.is_some() { SAFETY_POLL_EVERY } else { POLL_EVERY };
    let _event_guard = event_token.map(|token| EventGuard { listener: &listener, token });

    // A first scan that cannot be read at all means this PC does not support it.
    let mut seen: HashSet<u32> = HashSet::new();
    let initial = match scan(&listener) {
        Some(list) => list,
        None => return report(&tx, waker, Status::Unsupported),
    };
    let mut existing: Vec<Item> = Vec::new();
    for (id, item) in initial {
        seen.insert(id);
        if let Some(item) = item {
            existing.push(item);
        }
    }
    existing.sort_by_key(|i| i.time);
    report(&tx, waker, Status::Active);
    for item in existing {
        let _ = tx.send(Msg::New(item));
    }
    waker.wake();

    loop {
        match kick_rx.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            // Cannot happen while `event_ok` keeps the sender alive in the handler; if the
            // handler was dropped, fall back to plain sleeping.
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
        }
        // Stop when the page (receiver) is gone.
        if tx.send(Msg::Ping).is_err() {
            return;
        }
        let Some(current) = scan(&listener) else { continue };
        let ids: HashSet<u32> = current.iter().map(|(id, _)| *id).collect();
        let mut fresh: Vec<Item> = Vec::new();
        for (id, item) in current {
            if seen.insert(id) {
                if let Some(item) = item {
                    fresh.push(item);
                }
            }
        }
        // Forget dismissed notifications so the set does not grow without bound.
        seen.retain(|id| ids.contains(id));
        if fresh.is_empty() {
            continue;
        }
        fresh.sort_by_key(|i| i.time);
        for item in fresh {
            worker::peek(Peek {
                icon: "\u{E7E7}".to_string(),
                emoji: false,
                accent: theme::BLUE,
                title: if item.title.is_empty() { item.app.clone() } else { format!("{}: {}", item.app, item.title) },
                body: item.body.clone(),
                seconds: 5.0,
            });
            let _ = tx.send(Msg::New(item));
        }
        waker.wake();
    }
}

/// All current toasts as (id, parsed item). The item is `None` when the toast
/// has no readable text. Returns `None` if the list itself cannot be read.
fn scan(listener: &UserNotificationListener) -> Option<Vec<(u32, Option<Item>)>> {
    let list = listener.GetNotificationsAsync(NotificationKinds::Toast).ok()?.get().ok()?;
    let mut out = Vec::new();
    for n in list {
        let Ok(id) = n.Id() else { continue };
        out.push((id, parse(&n)));
    }
    Some(out)
}

fn parse(n: &windows::UI::Notifications::UserNotification) -> Option<Item> {
    let app = n
        .AppInfo()
        .and_then(|a| a.DisplayInfo())
        .and_then(|d| d.DisplayName())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut texts: Vec<String> = Vec::new();
    if let Ok(binding_name) = KnownNotificationBindings::ToastGeneric() {
        let binding = n.Notification().and_then(|x| x.Visual()).and_then(|v| v.GetBinding(&binding_name));
        if let Ok(elements) = binding.and_then(|b| b.GetTextElements()) {
            for el in elements {
                if let Ok(t) = el.Text() {
                    let t = t.to_string();
                    let t = t.trim();
                    if !t.is_empty() {
                        texts.push(t.to_string());
                    }
                }
            }
        }
    }
    let mut texts = texts.into_iter();
    let title = texts.next().unwrap_or_default();
    let body = texts.collect::<Vec<_>>().join(" ");
    if title.is_empty() && body.is_empty() {
        return None;
    }
    // CreationTime is 100 ns ticks since 1601-01-01.
    let time = n
        .CreationTime()
        .map(|t| t.UniversalTime / 10_000_000 - FILETIME_TO_UNIX_SECS)
        .unwrap_or_else(|_| now_secs());
    let app = if app.is_empty() { "Notification".to_string() } else { app };
    Some(Item { app, title, body, time })
}
