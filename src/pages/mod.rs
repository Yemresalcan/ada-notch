//! Pages shown in the expanded notch. The shell draws the header, arrows and
//! page dots; a page only draws inside the body rectangle it is given.
//!
//! Conventions for page modules:
//! - Constructor `pub fn new(waker: Waker) -> Self`.
//! - Slow work (network, file scans, WinRT) runs on a thread from
//!   `worker::spawn`; results go through the page's own mpsc channel, then
//!   `waker.wake()`. `poll` drains that channel on the UI thread.
//! - Persist with `crate::store` under a file name owned by the page.
//! - Use colors and sizes from `crate::theme`; draw only via `DrawCtx`.

mod calendar;
mod claude;
mod focus;
mod football;
mod media;
mod notifications;
mod settings;
mod shelf;
mod system;
mod textfield;
mod todo;
mod weather;

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;

use crate::gfx::{Align, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::theme;
use crate::worker::Waker;

#[derive(Clone, Copy, Debug)]
pub enum Mouse {
    Move(Point),
    Down(Point),
    Up(Point),
    DoubleClick(Point),
    /// Wheel notches; positive means scrolled up / away from the user.
    Wheel(Point, f32),
    Leave,
}

/// Requests a page can make of the shell while handling input.
pub struct Host {
    pub hwnd: HWND,
    /// Set to take keyboard focus (e.g. a text field was clicked). The shell
    /// activates the window and routes `on_char`/`on_key` until `editing()`
    /// returns false or focus is lost.
    pub want_keyboard: bool,
}

pub trait Page {
    /// Shown at the top-left of the expanded notch.
    fn title(&self) -> String;

    /// Drains background results. Return true if a redraw is needed.
    fn poll(&mut self) -> bool {
        false
    }

    /// Called once per second. Return true if a redraw is needed.
    fn tick(&mut self) -> bool {
        false
    }

    /// Draws the page inside `area` (window DIPs).
    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect);

    /// Mouse input inside `area`. Return true if a redraw is needed.
    fn on_mouse(&mut self, _ev: Mouse, _area: Rect, _host: &mut Host) -> bool {
        false
    }

    fn on_char(&mut self, _ch: char) -> bool {
        false
    }

    /// Virtual-key code of a key press while editing.
    fn on_key(&mut self, _vk: u16) -> bool {
        false
    }

    /// True while a text field is active; keeps the notch expanded.
    fn editing(&self) -> bool {
        false
    }

    fn on_focus_lost(&mut self) -> bool {
        false
    }

    /// True if files dropped on the notch should go to this page.
    fn accepts_files(&self) -> bool {
        false
    }

    fn on_drop_files(&mut self, _paths: &[PathBuf]) -> bool {
        false
    }

    /// A "live activity" for the collapsed notch (e.g. music playing, a timer
    /// running). The highest-priority activity among all pages is shown.
    fn activity(&self) -> Option<Activity> {
        None
    }

    /// Draws this page's live activity inside the collapsed notch `notch`
    /// (window DIPs; its width is at least `Activity::width`). The shell sets
    /// `ctx.opacity` for the cross-fade.
    fn draw_activity(&mut self, _ctx: &mut DrawCtx, _notch: Rect) {}

    /// True while the activity animates (e.g. audio bars); the shell then
    /// redraws every frame instead of once per tick.
    fn activity_animating(&self) -> bool {
        false
    }
}

/// See [`Page::activity`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Activity {
    /// Higher wins. Media 30, timers 20.
    pub priority: u8,
    /// Collapsed notch width needed, in DIPs (the default notch is 150).
    pub width: f32,
}

/// Stand-in until the real page module lands.
pub struct Placeholder {
    title: &'static str,
}

impl Placeholder {
    pub fn new(title: &'static str) -> Self {
        Self { title }
    }
}

impl Page for Placeholder {
    fn title(&self) -> String {
        self.title.to_string()
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        ctx.stroke_rect(area, 12.0, theme::BORDER, 1.0, true);
        ctx.text("Coming soon", area, TextStyle::new(13.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center));
    }
}

/// Every optional feature, in page order: (id stored in settings, label).
/// The Settings page itself is always last and cannot be turned off.
pub const FEATURES: &[(&str, &str)] = &[
    ("media", "Media & lyrics"),
    ("shelf", "File shelf"),
    ("todo", "Todo"),
    ("calendar", "Calendar"),
    ("focus", "Focus timer"),
    ("weather", "Weather"),
    ("football", "Football"),
    ("system", "System monitor"),
    ("claude", "Claude usage"),
    ("notifications", "Notifications"),
];

/// Settings key holding the comma-separated ids of disabled features.
pub const DISABLED_KEY: &str = "disabled";

/// Builds the pages of every enabled feature (disabled ones are never
/// constructed, so their workers never start), with Settings last.
/// Pages already in `existing` (keyed by feature id) are reused, so toggling
/// one feature does not reset the state of the others (e.g. a running timer).
pub fn build(
    waker: Waker,
    enabled: impl Fn(&str) -> bool,
    mut existing: Vec<(&'static str, Box<dyn Page>)>,
) -> Vec<(&'static str, Box<dyn Page>)> {
    let mut pages: Vec<(&'static str, Box<dyn Page>)> = Vec::new();
    let mut take = |id: &str| existing.iter().position(|(k, _)| *k == id).map(|i| existing.swap_remove(i).1);
    for &(id, _) in FEATURES {
        if !enabled(id) {
            continue;
        }
        if let Some(page) = take(id) {
            pages.push((id, page));
            continue;
        }
        pages.push((id, match id {
            "media" => Box::new(media::MediaPage::new(waker)),
            "shelf" => Box::new(shelf::ShelfPage::new(waker)),
            "todo" => Box::new(todo::TodoPage::new(waker)),
            "calendar" => Box::new(calendar::CalendarPage::new(waker)),
            "focus" => Box::new(focus::FocusPage::new(waker)),
            "weather" => Box::new(weather::WeatherPage::new(waker)),
            "football" => Box::new(football::FootballPage::new(waker)),
            "system" => Box::new(system::SystemPage::new(waker)),
            "claude" => Box::new(claude::ClaudePage::new(waker)),
            "notifications" => Box::new(notifications::NotificationsPage::new(waker)),
            _ => continue,
        }));
    }
    let settings = take(SETTINGS_ID).unwrap_or_else(|| Box::new(settings::SettingsPage::new(waker)));
    pages.push((SETTINGS_ID, settings));
    pages
}

/// Id of the always-present Settings page.
pub const SETTINGS_ID: &str = "settings";
