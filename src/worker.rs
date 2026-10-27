//! Background threads and the bridge back to the UI thread.
//!
//! Pages never touch UI state from a worker. A worker sends its results over an
//! `std::sync::mpsc` channel owned by the page and then calls [`Waker::wake`],
//! which posts a message so the UI thread calls `Page::poll` on every page.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::gfx::Color;

pub const WM_APP_WAKE: u32 = WM_APP + 1;
pub const WM_APP_CMD: u32 = WM_APP + 3;

#[derive(Clone, Copy)]
pub struct Waker {
    hwnd: isize,
}

impl Waker {
    pub fn new(hwnd: HWND) -> Self {
        SHELL_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        Self { hwnd: hwnd.0 as isize }
    }

    pub fn wake(&self) {
        unsafe {
            let _ = PostMessageW(HWND(self.hwnd as _), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

/// A short message shown by briefly growing the collapsed notch ("peek"),
/// e.g. a notification or "Focus time is over".
#[derive(Clone, Debug)]
pub struct Peek {
    /// Segoe Fluent Icons glyph or a color emoji (see `emoji`).
    pub icon: String,
    /// True if `icon` is a color emoji rather than a Fluent Icons glyph.
    pub emoji: bool,
    pub accent: Color,
    pub title: String,
    pub body: String,
    /// How long it stays before the notch shrinks back.
    pub seconds: f32,
}

/// Requests to the shell that may come from any thread.
pub enum ShellCmd {
    Peek(Peek),
    /// Settings changed: re-read settings.txt and rebuild the page list.
    ReloadSettings,
}

static SHELL_HWND: AtomicIsize = AtomicIsize::new(0);
static COMMANDS: Mutex<VecDeque<ShellCmd>> = Mutex::new(VecDeque::new());

/// Queues a command for the shell (callable from any thread).
pub fn send(cmd: ShellCmd) {
    if let Ok(mut q) = COMMANDS.lock() {
        q.push_back(cmd);
    }
    let hwnd = SHELL_HWND.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(HWND(hwnd as _), WM_APP_CMD, WPARAM(0), LPARAM(0));
        }
    }
}

/// Shows a peek (callable from any thread).
pub fn peek(peek: Peek) {
    send(ShellCmd::Peek(peek));
}

/// Takes all queued commands (shell side).
pub fn drain_commands() -> Vec<ShellCmd> {
    COMMANDS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default()
}

/// Spawns a named thread with the Windows Runtime initialized (MTA), so WinRT
/// calls such as `HttpClient` and `.get()` on async operations work inside it.
pub fn spawn<F>(name: &str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            f()
        })
        .expect("failed to spawn worker thread");
}
