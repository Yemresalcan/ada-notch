//! Settings page: toggle optional features, the mascot and minimal notch mode,
//! persisted in `settings.txt`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::gfx::{DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::{self, Host, Mouse, Page};
use crate::store;
use crate::theme;
use crate::worker::{self, ShellCmd, Waker};

const FILE: &str = "settings.txt";
const COLS: usize = 2;
const COL_GAP: f32 = 12.0;
const ROW_H: f32 = 22.0;
const ROW_GAP: f32 = 4.0;
const ROW_RADIUS: f32 = 6.0;
const PAD_X: f32 = 6.0;
const LABEL_GAP: f32 = 6.0;
const SWITCH_W: f32 = 26.0;
const SWITCH_H: f32 = 14.0;
const SWITCH_R: f32 = 7.0;
const KNOB_R: f32 = 5.0;

/// Preserves scroll position when `ReloadSettings` recreates `SettingsPage`.
static LAST_SCROLL: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
enum Kind {
    Feature(&'static str),
    Flag { key: &'static str, default_on: bool },
}

struct Row {
    label: &'static str,
    kind: Kind,
}

fn is_row_on(map: &BTreeMap<String, String>, kind: Kind) -> bool {
    match kind {
        Kind::Feature(id) => !map
            .get(pages::DISABLED_KEY)
            .is_some_and(|v| v.split(',').any(|x| x.trim() == id)),
        Kind::Flag { key, default_on: true } => map.get(key).map_or(true, |v| v != "0"),
        Kind::Flag { key, default_on: false } => map.get(key).is_some_and(|v| v == "1"),
    }
}

pub struct SettingsPage {
    rows: Vec<Row>,
    map: BTreeMap<String, String>,
    /// Unscrolled row rectangles from layout; scroll offset is applied on draw and hit-test.
    rects: Vec<Rect>,
    last_area: Option<Rect>,
    view_h: f32,
    scroll: f32,
    hover: Option<usize>,
    inside: bool,
}

impl SettingsPage {
    pub fn new(_waker: Waker) -> Self {
        let mut rows = Vec::with_capacity(pages::FEATURES.len() + 2);
        for &(id, label) in pages::FEATURES {
            rows.push(Row { label, kind: Kind::Feature(id) });
        }
        rows.push(Row { label: "Mascot", kind: Kind::Flag { key: "mascot", default_on: true } });
        rows.push(Row { label: "Minimal notch", kind: Kind::Flag { key: "minimal", default_on: false } });

        let saved = f32::from_bits(LAST_SCROLL.load(Ordering::Relaxed));
        let scroll = if saved.is_finite() { saved.max(0.0) } else { 0.0 };

        Self {
            rows,
            map: store::read_kv(FILE),
            rects: Vec::new(),
            last_area: None,
            view_h: theme::BODY.h,
            scroll,
            hover: None,
            inside: false,
        }
    }

    fn max_scroll(&self) -> f32 {
        let grid_rows = (self.rows.len() + COLS - 1) / COLS;
        let content_h = if grid_rows == 0 {
            0.0
        } else {
            grid_rows as f32 * ROW_H + (grid_rows - 1) as f32 * ROW_GAP
        };
        (content_h - self.view_h).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        LAST_SCROLL.store(self.scroll.to_bits(), Ordering::Relaxed);
    }

    fn layout_rects(&mut self, area: Rect) {
        let col_w = ((area.w - COL_GAP * (COLS as f32 - 1.0)) / COLS as f32).max(0.0);
        self.rects.clear();
        self.rects.reserve(self.rows.len());
        for i in 0..self.rows.len() {
            let col = i % COLS;
            let row = i / COLS;
            let x = area.x + col as f32 * (col_w + COL_GAP);
            let y = area.y + row as f32 * (ROW_H + ROW_GAP);
            self.rects.push(Rect::new(x, y, col_w, ROW_H));
        }
        self.last_area = Some(area);
    }

    fn hit(&self, p: Point, area: Rect) -> Option<usize> {
        if !area.contains(p) {
            return None;
        }
        self.rects.iter().position(|r| r.offset(0.0, -self.scroll).contains(p))
    }

    fn toggle(&mut self, index: usize) {
        let Some(row) = self.rows.get(index) else { return };
        let next_on = !is_row_on(&self.map, row.kind);
        let kind = row.kind;

        let mut map = store::read_kv(FILE);
        match kind {
            Kind::Feature(id) => {
                let mut disabled: Vec<String> = map
                    .get(pages::DISABLED_KEY)
                    .map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                if next_on {
                    disabled.retain(|x| x != id);
                } else if !disabled.iter().any(|x| x == id) {
                    disabled.push(id.to_string());
                }
                if disabled.is_empty() {
                    map.remove(pages::DISABLED_KEY);
                } else {
                    map.insert(pages::DISABLED_KEY.to_string(), disabled.join(","));
                }
            }
            Kind::Flag { key, .. } => {
                map.insert(key.to_string(), if next_on { "1" } else { "0" }.to_string());
            }
        }
        let _ = store::write_kv(FILE, &map);
        self.map = map;
        worker::send(ShellCmd::ReloadSettings);
    }
}

impl Page for SettingsPage {
    fn title(&self) -> String {
        "Settings".to_string()
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        self.view_h = area.h;
        self.clamp_scroll();
        self.layout_rects(area);

        ctx.push_clip(area);
        for (i, (row, &base)) in self.rows.iter().zip(self.rects.iter()).enumerate() {
            let r = base.offset(0.0, -self.scroll);
            if r.bottom() <= area.y || r.y >= area.bottom() {
                continue;
            }
            if self.hover == Some(i) {
                ctx.fill_rect(r, ROW_RADIUS, theme::CARD);
            }
            let on = is_row_on(&self.map, row.kind);
            let track = Rect::new(
                r.right() - PAD_X - SWITCH_W,
                r.y + (ROW_H - SWITCH_H) / 2.0,
                SWITCH_W,
                SWITCH_H,
            );
            ctx.fill_rect(track, SWITCH_R, if on { theme::ACCENT } else { theme::CARD_HOVER });
            let knob_x = if on { track.right() - SWITCH_R } else { track.x + SWITCH_R };
            ctx.fill_circle(Point { x: knob_x, y: track.center().y }, KNOB_R, theme::TEXT);

            let label_x = r.x + PAD_X;
            let label_w = (track.x - LABEL_GAP - label_x).max(0.0);
            let label_rect = Rect::new(label_x, r.y, label_w, ROW_H);
            ctx.text(row.label, label_rect, TextStyle::new(12.0, Weight::Regular, theme::TEXT));
        }
        ctx.pop_clip();
    }

    fn on_mouse(&mut self, ev: Mouse, area: Rect, _host: &mut Host) -> bool {
        self.view_h = area.h;
        if self.last_area != Some(area) {
            self.clamp_scroll();
            self.layout_rects(area);
        }
        match ev {
            Mouse::Move(p) => {
                let mut changed = false;
                if !self.inside {
                    self.inside = true;
                    let fresh = store::read_kv(FILE);
                    if fresh != self.map {
                        self.map = fresh;
                        changed = true;
                    }
                }
                let hit = self.hit(p, area);
                if hit != self.hover {
                    self.hover = hit;
                    changed = true;
                }
                changed
            }
            Mouse::Leave => {
                self.inside = false;
                self.hover.take().is_some()
            }
            // A double click arrives as Down + DoubleClick; toggling on both would undo it.
            Mouse::DoubleClick(_) => false,
            Mouse::Down(p) => {
                self.hover = self.hit(p, area);
                if let Some(i) = self.hover {
                    self.toggle(i);
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
                self.scroll -= delta * (ROW_H + ROW_GAP);
                self.clamp_scroll();
                self.hover = self.hit(p, area);
                self.scroll != before
            }
            Mouse::Up(_) => false,
        }
    }
}
