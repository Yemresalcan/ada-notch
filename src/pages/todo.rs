//! Todo page: add / edit / check off / delete tasks, persisted in `todo.txt`
//! (one task per line, `0|title` or `1|title`, 1 = done).


use crate::gfx::{Align, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::{Host, Mouse, Page};
use crate::store;
use crate::theme;
use crate::worker::Waker;

use crate::pages::textfield::{Outcome, TextField};

const FILE: &str = "todo.txt";
const TOP_H: f32 = 24.0;
const GAP: f32 = 4.0;
const ROW_H: f32 = 20.0;
const COUNTER_W: f32 = 52.0;
/// Width of the checkbox column and of the delete button column in a row.
const SIDE_W: f32 = 22.0;
const ADD_ICON_W: f32 = 26.0;

struct Task {
    done: bool,
    title: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Add,
    Check(usize),
    Title(usize),
    Delete(usize),
}

impl Hit {
    fn row(self) -> Option<usize> {
        match self {
            Hit::Add => None,
            Hit::Check(i) | Hit::Title(i) | Hit::Delete(i) => Some(i),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Add,
    Rename(usize),
}

struct Layout {
    add: Rect,
    counter: Rect,
    list: Rect,
}

fn layout(area: Rect) -> Layout {
    let (top, rest) = area.split_top(TOP_H);
    let (add, counter) = top.split_left((top.w - COUNTER_W).max(0.0));
    let list = Rect::new(rest.x, rest.y + GAP, rest.w, (rest.h - GAP).max(0.0));
    Layout { add, counter, list }
}

pub struct TodoPage {
    tasks: Vec<Task>,
    scroll: f32,
    list_h: f32,
    hover: Option<Hit>,
    mode: Mode,
    field: TextField,
}

impl TodoPage {
    pub fn new(_waker: Waker) -> Self {
        let tasks = store::read_lines(FILE)
            .iter()
            .filter_map(|line| {
                let (flag, title) = line.split_once('|')?;
                let title = title.trim();
                (!title.is_empty()).then(|| Task { done: flag == "1", title: title.to_string() })
            })
            .collect();
        Self {
            tasks,
            scroll: 0.0,
            list_h: TOP_H * 2.0,
            hover: None,
            mode: Mode::Idle,
            field: TextField::default(),
        }
    }

    fn save(&self) {
        let lines: Vec<String> =
            self.tasks.iter().map(|t| format!("{}|{}", u8::from(t.done), t.title)).collect();
        let _ = store::write_lines(FILE, &lines);
    }

    fn max_scroll(&self) -> f32 {
        (self.tasks.len() as f32 * ROW_H - self.list_h).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    fn ensure_visible(&mut self, i: usize) {
        let top = i as f32 * ROW_H;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW_H > self.scroll + self.list_h {
            self.scroll = top + ROW_H - self.list_h;
        }
        self.clamp_scroll();
    }

    fn row_rect(list: Rect, i: usize, scroll: f32) -> Rect {
        Rect::new(list.x, list.y + i as f32 * ROW_H - scroll, list.w, ROW_H)
    }

    fn hit(&self, p: Point, area: Rect) -> Option<Hit> {
        let lay = layout(area);
        if lay.add.contains(p) {
            return Some(Hit::Add);
        }
        if !lay.list.contains(p) {
            return None;
        }
        let i = ((p.y - lay.list.y + self.scroll) / ROW_H).floor();
        if i < 0.0 || i as usize >= self.tasks.len() {
            return None;
        }
        let i = i as usize;
        let row = Self::row_rect(lay.list, i, self.scroll);
        Some(if p.x < row.x + SIDE_W {
            Hit::Check(i)
        } else if p.x >= row.right() - SIDE_W {
            Hit::Delete(i)
        } else {
            Hit::Title(i)
        })
    }

    fn begin_edit(&mut self, mode: Mode, initial: &str, host: &mut Host) {
        if self.mode != Mode::Idle {
            self.finish_edit(true);
        }
        self.mode = mode;
        self.field.begin(initial);
        host.want_keyboard = true;
    }

    /// Ends the current edit; `commit` keeps non-empty text.
    fn finish_edit(&mut self, commit: bool) {
        let mode = std::mem::replace(&mut self.mode, Mode::Idle);
        let text = self.field.finish().trim().to_string();
        if !commit || text.is_empty() {
            return;
        }
        match mode {
            Mode::Add => {
                self.tasks.push(Task { done: false, title: text });
                self.ensure_visible(self.tasks.len() - 1);
                self.save();
            }
            Mode::Rename(i) => {
                if let Some(task) = self.tasks.get_mut(i) {
                    task.title = text;
                    self.save();
                }
            }
            Mode::Idle => {}
        }
    }

    fn remove(&mut self, i: usize) {
        if i >= self.tasks.len() {
            return;
        }
        self.tasks.remove(i);
        match self.mode {
            Mode::Rename(r) if r == i => self.finish_edit(false),
            Mode::Rename(r) if r > i => self.mode = Mode::Rename(r - 1),
            _ => {}
        }
        self.clamp_scroll();
        self.save();
    }

    /// Handles a press (also used for the second click of a double click that
    /// does not land on a title).
    fn press(&mut self, p: Point, area: Rect, host: &mut Host) -> bool {
        match self.hit(p, area) {
            Some(Hit::Add) if self.mode != Mode::Add => self.begin_edit(Mode::Add, "", host),
            Some(Hit::Check(i)) => {
                self.tasks[i].done = !self.tasks[i].done;
                self.save();
            }
            Some(Hit::Delete(i)) => {
                self.remove(i);
                self.hover = self.hit(p, area);
            }
            _ => return false,
        }
        true
    }
}

impl Page for TodoPage {
    fn title(&self) -> String {
        "Todo".to_string()
    }

    fn tick(&mut self) -> bool {
        self.field.blink()
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        let lay = layout(area);
        self.list_h = lay.list.h;
        self.clamp_scroll();

        // Add field + counter.
        let adding = self.mode == Mode::Add;
        let hover_add = self.hover == Some(Hit::Add) && !adding;
        ctx.fill_rect(lay.add, 12.0, if hover_add { theme::CARD_HOVER } else { theme::CARD });
        if adding {
            ctx.stroke_rect(lay.add, 12.0, theme::BORDER, 1.0, false);
        }
        let (icon, field) = lay.add.split_left(ADD_ICON_W);
        ctx.text("\u{E710}", icon, TextStyle::icon(11.0, theme::TEXT_DIM));
        let field = Rect::new(field.x, field.y, (field.w - 8.0).max(0.0), field.h);
        let field_style = TextStyle::new(12.0, Weight::Regular, theme::TEXT);
        if adding {
            self.field.draw(ctx, field, field_style, "Add a task\u{2026}");
        } else {
            ctx.text("Add a task\u{2026}", field, TextStyle::new(12.0, Weight::Medium, theme::TEXT_DIM));
        }
        let done = self.tasks.iter().filter(|t| t.done).count();
        ctx.text(
            &format!("{done} done"),
            lay.counter,
            TextStyle::new(11.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Right),
        );

        // Task list.
        if self.tasks.is_empty() {
            ctx.text("No tasks.", lay.list, TextStyle::new(12.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Center));
            return;
        }
        ctx.push_clip(lay.list);
        let first = (self.scroll / ROW_H).floor().max(0.0) as usize;
        let hovered_row = self.hover.and_then(Hit::row);
        for i in first..self.tasks.len() {
            let row = Self::row_rect(lay.list, i, self.scroll);
            if row.y >= lay.list.bottom() {
                break;
            }
            let renaming = self.mode == Mode::Rename(i);
            let hovered = hovered_row == Some(i);
            if renaming || hovered {
                ctx.fill_rect(row, 6.0, if renaming { theme::CARD_HOVER } else { theme::CARD });
            }
            let task = &self.tasks[i];

            // Round checkbox.
            let c = Point { x: row.x + 11.0, y: row.y + ROW_H / 2.0 };
            if task.done {
                ctx.fill_circle(c, 7.0, theme::ACCENT);
                ctx.text("\u{E73E}", Rect::new(c.x - 8.0, c.y - 8.0, 16.0, 16.0), TextStyle::icon(9.0, theme::BG));
            } else {
                let color = if self.hover == Some(Hit::Check(i)) { theme::TEXT_DIM } else { theme::BORDER };
                ctx.stroke_circle(c, 6.5, color, 1.5);
            }

            // Title (or in-place editor).
            let title_rect = Rect::new(row.x + SIDE_W + 4.0, row.y, (row.w - SIDE_W * 2.0 - 4.0).max(0.0), ROW_H);
            if renaming {
                self.field.draw(ctx, title_rect, TextStyle::new(12.0, Weight::Regular, theme::TEXT), "");
            } else {
                let color = if task.done { theme::TEXT_FAINT } else { theme::TEXT };
                let style = TextStyle::new(12.0, Weight::Regular, color);
                ctx.text(&task.title, title_rect, style);
                if task.done {
                    let w = ctx.measure_text(&task.title, &style).min(title_rect.w);
                    let y = row.y + ROW_H / 2.0;
                    ctx.line(Point { x: title_rect.x, y }, Point { x: title_rect.x + w, y }, theme::TEXT_FAINT, 1.0);
                }
            }

            // Delete button on hover.
            if hovered {
                let del = Rect::new(row.right() - SIDE_W, row.y, SIDE_W, ROW_H);
                let color = if self.hover == Some(Hit::Delete(i)) { theme::DANGER } else { theme::TEXT_DIM };
                ctx.text("\u{E711}", del, TextStyle::icon(9.0, color));
            }
        }
        ctx.pop_clip();
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
            Mouse::Down(p) => self.press(p, area, host),
            Mouse::DoubleClick(p) => match self.hit(p, area) {
                Some(Hit::Title(i)) if self.mode != Mode::Rename(i) => {
                    let title = self.tasks[i].title.clone();
                    self.begin_edit(Mode::Rename(i), &title, host);
                    self.ensure_visible(i);
                    true
                }
                _ => self.press(p, area, host),
            },
            Mouse::Wheel(p, delta) => {
                if !area.contains(p) {
                    return false;
                }
                let before = self.scroll;
                self.scroll -= delta * ROW_H;
                self.clamp_scroll();
                self.hover = self.hit(p, area);
                self.scroll != before
            }
            Mouse::Up(_) => false,
        }
    }

    fn on_char(&mut self, ch: char) -> bool {
        self.mode != Mode::Idle && self.field.insert(ch)
    }

    fn on_key(&mut self, vk: u16) -> bool {
        if self.mode == Mode::Idle {
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
        self.mode != Mode::Idle
    }

    fn on_focus_lost(&mut self) -> bool {
        if self.mode == Mode::Idle {
            return false;
        }
        self.finish_edit(true);
        true
    }
}
