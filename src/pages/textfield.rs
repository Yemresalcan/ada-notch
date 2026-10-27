//! Minimal single-line text input shared by pages (todo, calendar).
//!
//! The page owns a `TextField`, forwards `on_char` / `on_key` to it while it is
//! active, and draws it with [`TextField::draw`]. The caret blinks by toggling
//! in the page's `tick()` (see [`TextField::blink`]).

use crate::gfx::{DrawCtx, Point, Rect, TextStyle};
use crate::theme;

const MAX_CHARS: usize = 120;

const VK_BACK: u16 = 0x08;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_DELETE: u16 = 0x2E;

/// What a key press meant for the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still editing (key handled or ignored).
    Pending,
    /// Enter pressed: the page should read [`TextField::finish`] and keep it.
    Commit,
    /// Escape pressed: the page should call `finish` and discard the text.
    Cancel,
}

#[derive(Default)]
pub struct TextField {
    text: String,
    /// Caret position in characters (not bytes).
    caret: usize,
    active: bool,
    caret_on: bool,
}

impl TextField {
    /// Starts editing with `initial` as the content and the caret at the end.
    pub fn begin(&mut self, initial: &str) {
        self.text = initial.chars().take(MAX_CHARS).collect();
        self.caret = self.text.chars().count();
        self.active = true;
        self.caret_on = true;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Stops editing and returns the final text.
    pub fn finish(&mut self) -> String {
        self.active = false;
        self.caret = 0;
        std::mem::take(&mut self.text)
    }

    /// Toggles the caret; returns true if a redraw is needed.
    pub fn blink(&mut self) -> bool {
        if self.active {
            self.caret_on = !self.caret_on;
        }
        self.active
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(b, _)| b)
    }

    /// Inserts a typed character; control characters are ignored.
    pub fn insert(&mut self, ch: char) -> bool {
        if !self.active || ch.is_control() || self.text.chars().count() >= MAX_CHARS {
            return false;
        }
        let at = self.byte_at(self.caret);
        self.text.insert(at, ch);
        self.caret += 1;
        self.caret_on = true;
        true
    }

    /// Handles a virtual-key press (editing keys, Enter, Esc).
    pub fn key(&mut self, vk: u16) -> Outcome {
        if !self.active {
            return Outcome::Pending;
        }
        self.caret_on = true;
        let len = self.text.chars().count();
        match vk {
            VK_RETURN => return Outcome::Commit,
            VK_ESCAPE => return Outcome::Cancel,
            VK_BACK if self.caret > 0 => {
                let at = self.byte_at(self.caret - 1);
                self.text.remove(at);
                self.caret -= 1;
            }
            VK_DELETE if self.caret < len => {
                let at = self.byte_at(self.caret);
                self.text.remove(at);
            }
            VK_LEFT => self.caret = self.caret.saturating_sub(1),
            VK_RIGHT => self.caret = (self.caret + 1).min(len),
            VK_HOME => self.caret = 0,
            VK_END => self.caret = len,
            _ => {}
        }
        Outcome::Pending
    }

    /// Draws the text (or `hint` when empty) in `rect`, scrolling horizontally
    /// so the caret stays visible, plus the blinking caret while active.
    pub fn draw(&self, ctx: &mut DrawCtx, rect: Rect, style: TextStyle, hint: &str) {
        ctx.push_clip(rect);
        if self.text.is_empty() {
            ctx.text(hint, rect, TextStyle { color: theme::TEXT_FAINT, ..style });
        }
        let prefix = &self.text[..self.byte_at(self.caret)];
        let caret_x = ctx.measure_text(prefix, &style);
        let total = ctx.measure_text(&self.text, &style);
        let scroll = (caret_x - (rect.w - 3.0)).max(0.0);
        if !self.text.is_empty() {
            ctx.text(&self.text, Rect::new(rect.x - scroll, rect.y, total + 24.0, rect.h), style);
        }
        if self.active && self.caret_on {
            let x = rect.x + caret_x - scroll;
            let cy = rect.y + rect.h / 2.0;
            let half = (style.size + 3.0) / 2.0;
            ctx.line(Point { x, y: cy - half }, Point { x, y: cy + half }, theme::TEXT, 1.0);
        }
        ctx.pop_clip();
    }
}
