//! Shelf page: a temporary shelf of file references (paths, never copies).
//! Drop files on the notch to add them, click a tile to open it, drag a tile
//! out to Explorer/Discord/a browser to copy it there, "x" removes a tile from
//! the shelf (the real file is never touched).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::{w, Interface, GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR};
use windows::Win32::Foundation::{
    BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, E_NOINTERFACE, E_POINTER, HWND, S_OK,
};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Vtbl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MODIFIERKEYS_FLAGS, MK_LBUTTON};
use windows::Win32::UI::Shell::{SHCreateItemFromParsingName, ShellExecuteW, BHID_DataObject, IShellItem};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::gfx::{Align, Color, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::{Host, Mouse, Page};
use crate::store;
use crate::theme;
use crate::worker::{self, Waker};

const STORE_FILE: &str = "shelf.txt";
const TILE_W: f32 = 64.0;
const TILE_H: f32 = 80.0;
const GAP: f32 = 8.0;
const PLUS_W: f32 = 48.0;
const CLOSE_R: f32 = 8.0;
const DRAG_THRESHOLD: f32 = 6.0;
const WHEEL_STEP: f32 = 56.0;
/// `tick` runs once per second; the existence check runs every this many ticks.
const CHECK_EVERY: u32 = 5;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Folder,
    Image,
    Document,
    Link,
    Generic,
}

impl Kind {
    fn of(path: &Path) -> Kind {
        if path.is_dir() {
            return Kind::Folder;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "svg" | "tif" | "tiff" | "heic" | "avif" => {
                Kind::Image
            }
            "url" | "lnk" | "webloc" => Kind::Link,
            "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" | "odt" | "xls" | "xlsx" | "csv" | "ppt" | "pptx" => {
                Kind::Document
            }
            _ => Kind::Generic,
        }
    }

    fn glyph(self) -> (&'static str, Color) {
        match self {
            Kind::Folder => ("\u{E8B7}", theme::WARN),
            Kind::Image => ("\u{EB9F}", theme::BLUE),
            Kind::Document => ("\u{E8A5}", theme::TEXT),
            Kind::Link => ("\u{E71B}", theme::ACCENT),
            Kind::Generic => ("\u{E7C3}", theme::TEXT_DIM),
        }
    }
}

struct Item {
    path: PathBuf,
    name: String,
    kind: Kind,
}

impl Item {
    fn new(path: PathBuf) -> Self {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => path.display().to_string(), // drive root such as "C:\"
        };
        let kind = Kind::of(&path);
        Self { path, name, kind }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    None,
    Tile(usize),
    Close(usize),
    Plus,
}

/// Mouse button is down on a tile; becomes a click on release or a drag on movement.
struct Press {
    index: usize,
    start: Point,
}

pub struct ShelfPage {
    waker: Waker,
    items: Vec<Item>,
    /// Horizontal scroll offset in DIPs.
    scroll: f32,
    mouse: Option<Point>,
    press: Option<Press>,
    ticks: u32,
    checking: bool,
    /// Worker -> UI: paths that no longer exist.
    missing_tx: Sender<Vec<PathBuf>>,
    missing_rx: Receiver<Vec<PathBuf>>,
}

impl ShelfPage {
    pub fn new(waker: Waker) -> Self {
        let (missing_tx, missing_rx) = channel();
        let mut items: Vec<Item> = Vec::new();
        for line in store::read_lines(STORE_FILE) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let path = PathBuf::from(line);
            if !items.iter().any(|i| same_path(&i.path, &path)) {
                items.push(Item::new(path));
            }
        }
        let mut page = Self {
            waker,
            items,
            scroll: 0.0,
            mouse: None,
            press: None,
            ticks: 0,
            checking: false,
            missing_tx,
            missing_rx,
        };
        // Existence is checked off the UI thread (a dead network path can block).
        page.start_check();
        page
    }

    fn save(&self) {
        let lines: Vec<String> = self.items.iter().map(|i| i.path.to_string_lossy().into_owned()).collect();
        let _ = store::write_lines(STORE_FILE, &lines);
    }

    fn start_check(&mut self) {
        if self.checking || self.items.is_empty() {
            return;
        }
        self.checking = true;
        let paths: Vec<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        let tx = self.missing_tx.clone();
        let waker = self.waker;
        worker::spawn("shelf-check", move || {
            let missing: Vec<PathBuf> = paths.into_iter().filter(|p| !p.exists()).collect();
            let _ = tx.send(missing); // always reply so `checking` is cleared
            waker.wake();
        });
    }

    fn content_width(&self) -> f32 {
        self.items.len() as f32 * (TILE_W + GAP) + PLUS_W
    }

    fn max_scroll(&self, area: Rect) -> f32 {
        (self.content_width() - area.w).max(0.0)
    }

    fn clamp_scroll(&mut self, area: Rect) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(area));
    }

    fn tile_rect(&self, index: usize, area: Rect) -> Rect {
        let y = area.y + (area.h - TILE_H) / 2.0;
        Rect::new(area.x + index as f32 * (TILE_W + GAP) - self.scroll, y, TILE_W, TILE_H)
    }

    fn plus_rect(&self, area: Rect) -> Rect {
        let y = area.y + (area.h - TILE_H) / 2.0;
        Rect::new(area.x + self.items.len() as f32 * (TILE_W + GAP) - self.scroll, y, PLUS_W, TILE_H)
    }

    fn close_center(tile: Rect) -> Point {
        Point { x: tile.right() - CLOSE_R - 3.0, y: tile.y + CLOSE_R + 3.0 }
    }

    fn hit(&self, p: Point, area: Rect) -> Hit {
        if !area.contains(p) {
            return Hit::None;
        }
        for i in 0..self.items.len() {
            let tile = self.tile_rect(i, area);
            if tile.contains(p) {
                let c = Self::close_center(tile);
                let (dx, dy) = (p.x - c.x, p.y - c.y);
                return if dx * dx + dy * dy <= (CLOSE_R + 1.0) * (CLOSE_R + 1.0) { Hit::Close(i) } else { Hit::Tile(i) };
            }
        }
        if self.plus_rect(area).contains(p) {
            Hit::Plus
        } else {
            Hit::None
        }
    }

    fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.save();
        }
    }

    fn open(&self, index: usize) {
        let Some(item) = self.items.get(index) else { return };
        let wide: Vec<u16> = item.path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        unsafe {
            ShellExecuteW(HWND::default(), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }

    fn draw_empty(&self, ctx: &mut DrawCtx, area: Rect) {
        ctx.stroke_rect(area, 12.0, theme::BORDER, 1.0, true);
        ctx.text("\u{E896}", Rect::new(area.x, area.y + 10.0, area.w, 30.0), TextStyle::icon(22.0, theme::TEXT_DIM));
        ctx.text(
            "Drop files here",
            Rect::new(area.x, area.y + 42.0, area.w, 18.0),
            TextStyle::new(13.0, Weight::Semibold, theme::TEXT).align(Align::Center),
        );
        ctx.text(
            "Drag out anytime \u{00B7} click to open",
            Rect::new(area.x, area.y + 60.0, area.w, 16.0),
            TextStyle::new(11.0, Weight::Regular, theme::TEXT_DIM).align(Align::Center),
        );
    }
}

impl Page for ShelfPage {
    fn title(&self) -> String {
        "Shelf".to_string()
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(missing) = self.missing_rx.try_recv() {
            self.checking = false;
            if missing.is_empty() {
                continue;
            }
            let before = self.items.len();
            self.items.retain(|i| !missing.iter().any(|m| same_path(m, &i.path)));
            if self.items.len() != before {
                self.press = None;
                self.save();
                changed = true;
            }
        }
        changed
    }

    fn tick(&mut self) -> bool {
        self.ticks += 1;
        if self.ticks % CHECK_EVERY == 0 {
            self.start_check();
        }
        false
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        if self.items.is_empty() {
            self.scroll = 0.0;
            self.draw_empty(ctx, area);
            return;
        }
        self.clamp_scroll(area);
        let hover = self.mouse.map_or(Hit::None, |p| self.hit(p, area));
        let hovered_tile = match hover {
            Hit::Tile(i) | Hit::Close(i) => Some(i),
            _ => None,
        };

        ctx.push_clip(area);
        for (i, item) in self.items.iter().enumerate() {
            let tile = self.tile_rect(i, area);
            if tile.right() < area.x || tile.x > area.right() {
                continue;
            }
            let hovered = hovered_tile == Some(i);
            ctx.fill_rect(tile, 10.0, if hovered { theme::CARD_HOVER } else { theme::CARD });
            let (glyph, color) = item.kind.glyph();
            ctx.text(glyph, Rect::new(tile.x, tile.y + 12.0, tile.w, 36.0), TextStyle::icon(28.0, color));
            ctx.text(
                &item.name,
                Rect::new(tile.x + 4.0, tile.y + 54.0, tile.w - 8.0, 18.0),
                TextStyle::new(11.0, Weight::Regular, theme::TEXT).align(Align::Center),
            );
            if hovered {
                let c = Self::close_center(tile);
                let on_close = hover == Hit::Close(i);
                let bg = if on_close { theme::DANGER } else { Color::rgba(0.0, 0.0, 0.0, 0.55) };
                ctx.fill_circle(c, CLOSE_R, bg);
                ctx.text(
                    "\u{E711}",
                    Rect::new(c.x - CLOSE_R, c.y - CLOSE_R, CLOSE_R * 2.0, CLOSE_R * 2.0),
                    TextStyle::icon(9.0, theme::TEXT),
                );
            }
        }

        let plus = self.plus_rect(area);
        if plus.right() >= area.x && plus.x <= area.right() {
            if hover == Hit::Plus {
                ctx.fill_rect(plus, 10.0, theme::CARD);
            }
            ctx.stroke_rect(plus, 10.0, theme::BORDER, 1.0, true);
            ctx.text("\u{E710}", Rect::new(plus.x, plus.y + 16.0, plus.w, 24.0), TextStyle::icon(16.0, theme::TEXT_DIM));
            ctx.text(
                "Drop more",
                Rect::new(plus.x, plus.y + 42.0, plus.w, 16.0),
                TextStyle::new(9.0, Weight::Regular, theme::TEXT_FAINT).align(Align::Center),
            );
        }
        ctx.pop_clip();
    }

    fn on_mouse(&mut self, ev: Mouse, area: Rect, _host: &mut Host) -> bool {
        match ev {
            Mouse::Move(p) => {
                let before = self.mouse.map_or(Hit::None, |m| self.hit(m, area));
                self.mouse = Some(p);
                let after = self.hit(p, area);
                if let Some(press) = &self.press {
                    let (dx, dy) = (p.x - press.start.x, p.y - press.start.y);
                    if dx * dx + dy * dy > DRAG_THRESHOLD * DRAG_THRESHOLD {
                        let index = press.index;
                        self.press = None;
                        if let Some(item) = self.items.get(index) {
                            drag_out(&item.path);
                        }
                        // The modal drag loop swallowed the button release; the
                        // pointer position is stale afterwards.
                        self.mouse = None;
                        return true;
                    }
                }
                before != after
            }
            Mouse::Down(p) => {
                self.mouse = Some(p);
                match self.hit(p, area) {
                    Hit::Close(i) => {
                        self.press = None;
                        self.remove(i);
                        true
                    }
                    Hit::Tile(i) => {
                        self.press = Some(Press { index: i, start: p });
                        false
                    }
                    _ => {
                        self.press = None;
                        false
                    }
                }
            }
            // A double click replaces the second Down; ignore it so the file
            // is not opened twice.
            Mouse::DoubleClick(_) => {
                self.press = None;
                false
            }
            Mouse::Up(p) => {
                self.mouse = Some(p);
                if let Some(press) = self.press.take() {
                    if self.hit(p, area) == Hit::Tile(press.index) {
                        self.open(press.index);
                    }
                }
                false
            }
            Mouse::Wheel(_, delta) => {
                let max = self.max_scroll(area);
                if max <= 0.0 {
                    return false;
                }
                let old = self.scroll;
                self.scroll = (self.scroll - delta * WHEEL_STEP).clamp(0.0, max);
                self.scroll != old
            }
            Mouse::Leave => {
                self.press = None;
                self.mouse.take().is_some()
            }
        }
    }

    fn accepts_files(&self) -> bool {
        true
    }

    fn on_drop_files(&mut self, paths: &[PathBuf]) -> bool {
        let mut added = false;
        for path in paths {
            if !path.exists() || self.items.iter().any(|i| same_path(&i.path, path)) {
                continue;
            }
            self.items.push(Item::new(path.clone()));
            added = true;
        }
        if added {
            self.save();
            // Show the newest tile (clamped to the real maximum on next draw).
            self.scroll = f32::MAX;
        }
        added
    }
}

/// Windows paths are case-insensitive.
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

/// Minimal OLE drop source (left button released = drop, Esc = cancel). The
/// `#[implement]` macro needs a direct `windows-core` dependency, so the COM
/// object is written by hand: a vtable pointer followed by a refcount.
#[repr(C)]
struct DropSource {
    vtbl: *const IDropSource_Vtbl,
    refs: AtomicU32,
}

static DROP_SOURCE_VTBL: IDropSource_Vtbl = IDropSource_Vtbl {
    base__: IUnknown_Vtbl { QueryInterface: ds_query_interface, AddRef: ds_add_ref, Release: ds_release },
    QueryContinueDrag: ds_query_continue_drag,
    GiveFeedback: ds_give_feedback,
};

impl DropSource {
    /// Returns an interface owning the initial reference.
    fn create() -> IDropSource {
        let raw = Box::into_raw(Box::new(DropSource { vtbl: &DROP_SOURCE_VTBL, refs: AtomicU32::new(1) }));
        // SAFETY: `raw` starts with a valid vtable pointer and carries one reference.
        unsafe { IDropSource::from_raw(raw as *mut c_void) }
    }
}

unsafe extern "system" fn ds_query_interface(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let iid = &*iid;
    if *iid == IUnknown::IID || *iid == IDropSource::IID {
        ds_add_ref(this);
        *out = this;
        S_OK
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn ds_add_ref(this: *mut c_void) -> u32 {
    (*(this as *const DropSource)).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn ds_release(this: *mut c_void) -> u32 {
    let left = (*(this as *const DropSource)).refs.fetch_sub(1, Ordering::Release) - 1;
    if left == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this as *mut DropSource));
    }
    left
}

unsafe extern "system" fn ds_query_continue_drag(_this: *mut c_void, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
    if escape.as_bool() {
        DRAGDROP_S_CANCEL
    } else if (keys.0 & MK_LBUTTON.0) == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn ds_give_feedback(_this: *mut c_void, _effect: DROPEFFECT) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

/// Starts a (blocking, modal) OLE drag of `path`; the shelf keeps the item.
fn drag_out(path: &Path) {
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None) else { return };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return };
        let source = DropSource::create();
        let mut effect = DROPEFFECT_NONE;
        let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_LINK, &mut effect);
    }
}
