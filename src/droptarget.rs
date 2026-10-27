//! OLE drop target for the notch window.
//!
//! `WM_DROPFILES` only reports the final drop; an `IDropTarget` registered
//! with `RegisterDragDrop(hwnd, &target)` also reports the drag entering and
//! leaving, so the shell can expand the notch ("Release to add") while the
//! user is still dragging.
//!
//! All methods run on the UI thread (STA), so the handler may hold `Rc` /
//! non-`Send` state. A drop is reported through [`DropHandler::drop`] only;
//! the handler is *not* additionally sent `drag_leave` for an accepted drop
//! (it is for a drop whose data could not be read).
//!
//! The crate's `windows` feature list has no `Win32_System_Com_StructuredStorage`,
//! `Win32_System_Memory` or `Win32_System_DataExchange`, so `IDataObject::GetData`,
//! `GlobalLock`, `ReleaseStgMedium` and `RegisterClipboardFormatW` are reached
//! through the tiny FFI layer at the bottom of this file instead.

use std::cell::Cell;
use std::ffi::{c_void, OsString};
use std::io::{self, Write};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr;

use windows::core::{implement, Interface, Result};
use windows::Win32::Foundation::POINTL;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
    DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// What was dropped on the notch.
pub enum DropData {
    Files(Vec<PathBuf>),
    /// A web link (e.g. dragged from a browser's address bar).
    Url { url: String, title: Option<String> },
}

/// Receives drag-and-drop events on the UI thread.
pub trait DropHandler {
    /// An acceptable payload is being dragged over the window.
    fn drag_enter(&self);
    /// The drag left the window (or was cancelled) without dropping.
    fn drag_leave(&self);
    /// The payload was dropped.
    fn drop(&self, data: DropData);
}

/// Creates the drop target; pass it to `RegisterDragDrop(hwnd, &target)`.
pub fn create(handler: Box<dyn DropHandler>) -> IDropTarget {
    DropTarget {
        handler,
        url_format: unsafe { RegisterClipboardFormatW(windows::core::w!("UniformResourceLocatorW").as_ptr()) },
        active: Cell::new(false),
        effect: Cell::new(DROPEFFECT_NONE),
    }
    .into()
}

#[implement(IDropTarget)]
struct DropTarget {
    handler: Box<dyn DropHandler>,
    /// Registered id of the "UniformResourceLocatorW" clipboard format (0 if registration failed).
    url_format: u32,
    /// True between an accepted `DragEnter` and the matching leave/drop.
    active: Cell<bool>,
    /// The effect to report for the current drag (COPY/LINK or NONE).
    effect: Cell<DROPEFFECT>,
}

impl DropTarget {
    /// Picks the single effect to report given the effects the source allows.
    fn pick_effect(&self, allowed: DROPEFFECT, prefer_link: bool) -> DROPEFFECT {
        let (first, second) = if prefer_link {
            (DROPEFFECT_LINK, DROPEFFECT_COPY)
        } else {
            (DROPEFFECT_COPY, DROPEFFECT_LINK)
        };
        if allowed.0 & first.0 != 0 {
            first
        } else if allowed.0 & second.0 != 0 {
            second
        } else {
            DROPEFFECT_NONE
        }
    }

    fn set_effect(&self, out: *mut DROPEFFECT, effect: DROPEFFECT) {
        self.effect.set(effect);
        if !out.is_null() {
            unsafe { *out = effect };
        }
    }

    fn payload_kind(&self, data: &IDataObject) -> Kind {
        unsafe {
            if has_format(data, CF_HDROP.0) {
                Kind::Files
            } else if self.url_format != 0 && has_format(data, self.url_format as u16) {
                Kind::Url
            } else if has_format(data, CF_UNICODETEXT.0) && read_text(data, CF_UNICODETEXT.0).is_some_and(|t| as_web_url(&t).is_some()) {
                Kind::UrlText
            } else {
                Kind::None
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    None,
    Files,
    Url,
    /// Plain text that holds an http(s) URL.
    UrlText,
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Option<&IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        let kind = pdataobj.map_or(Kind::None, |d| self.payload_kind(d));
        let allowed = if pdweffect.is_null() { DROPEFFECT_NONE } else { unsafe { *pdweffect } };
        let effect = if kind == Kind::None { DROPEFFECT_NONE } else { self.pick_effect(allowed, kind != Kind::Files) };
        self.set_effect(pdweffect, effect);
        let accepted = effect != DROPEFFECT_NONE;
        if accepted && !self.active.replace(true) {
            self.handler.drag_enter();
        }
        Ok(())
    }

    fn DragOver(&self, _grfkeystate: MODIFIERKEYS_FLAGS, _pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        // Re-evaluate against the currently allowed effects (modifier keys change them).
        let allowed = if pdweffect.is_null() { DROPEFFECT_NONE } else { unsafe { *pdweffect } };
        let effect = if self.active.get() {
            let prefer_link = self.effect.get() == DROPEFFECT_LINK;
            self.pick_effect(allowed, prefer_link)
        } else {
            DROPEFFECT_NONE
        };
        self.set_effect(pdweffect, effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        if self.active.replace(false) {
            self.handler.drag_leave();
        }
        self.effect.set(DROPEFFECT_NONE);
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Option<&IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        let was_active = self.active.replace(false);
        let data = pdataobj.and_then(|d| self.extract(d));
        match data {
            Some(data) => {
                let effect = if self.effect.get() == DROPEFFECT_NONE { DROPEFFECT_COPY } else { self.effect.get() };
                self.set_effect(pdweffect, effect);
                DropHandler::drop(&*self.handler, data);
            }
            None => {
                self.set_effect(pdweffect, DROPEFFECT_NONE);
                if was_active {
                    self.handler.drag_leave();
                }
            }
        }
        self.effect.set(DROPEFFECT_NONE);
        Ok(())
    }
}

impl DropTarget {
    fn extract(&self, data: &IDataObject) -> Option<DropData> {
        unsafe {
            if has_format(data, CF_HDROP.0) {
                if let Some(files) = read_files(data) {
                    if !files.is_empty() {
                        return Some(DropData::Files(files));
                    }
                }
            }
            if self.url_format != 0 && has_format(data, self.url_format as u16) {
                if let Some(text) = read_text(data, self.url_format as u16) {
                    let url = text.lines().next().unwrap_or("").trim().to_string();
                    if !url.is_empty() {
                        return Some(DropData::Url { url, title: None });
                    }
                }
            }
            if has_format(data, CF_UNICODETEXT.0) {
                if let Some(url) = read_text(data, CF_UNICODETEXT.0).as_deref().and_then(as_web_url) {
                    return Some(DropData::Url { url: url.to_string(), title: None });
                }
            }
        }
        None
    }
}

/// First line of `text` if it is an http(s) URL.
fn as_web_url(text: &str) -> Option<&str> {
    let line = text.lines().next()?.trim();
    let lower = line.get(..8).unwrap_or(line).to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")).then_some(line)
}

// ---- data object helpers ----

fn format_etc(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

unsafe fn has_format(data: &IDataObject, cf: u16) -> bool {
    data.QueryGetData(&format_etc(cf)).is_ok()
}

/// An owned `STGMEDIUM` that is released on drop.
struct Medium(StgMedium);

impl Medium {
    fn global(&self) -> Option<*mut c_void> {
        (self.0.tymed == TYMED_HGLOBAL.0 as u32 && !self.0.data.is_null()).then_some(self.0.data)
    }
}

impl Drop for Medium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

/// `IDataObject::GetData` through the raw vtable (slot 3: after QueryInterface/AddRef/Release).
unsafe fn get_data(data: &IDataObject, cf: u16) -> Option<Medium> {
    type GetDataFn = unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut StgMedium) -> windows::core::HRESULT;
    let this = data.as_raw();
    let vtable = *(this as *const *const usize);
    let get: GetDataFn = std::mem::transmute(*vtable.add(3));
    let format = format_etc(cf);
    let mut medium = StgMedium { tymed: 0, data: ptr::null_mut(), release: ptr::null_mut() };
    if get(this, &format, &mut medium).is_ok() {
        Some(Medium(medium))
    } else {
        None
    }
}

unsafe fn read_files(data: &IDataObject) -> Option<Vec<PathBuf>> {
    let medium = get_data(data, CF_HDROP.0)?;
    let hdrop = HDROP(medium.global()?);
    let count = DragQueryFileW(hdrop, u32::MAX, None);
    let mut files = Vec::new();
    for i in 0..count {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        if len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len + 1];
        let written = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
        files.push(PathBuf::from(OsString::from_wide(&buf[..written.min(len)])));
    }
    Some(files)
}

/// Reads a NUL-terminated UTF-16 string from an HGLOBAL medium.
unsafe fn read_text(data: &IDataObject, cf: u16) -> Option<String> {
    let medium = get_data(data, cf)?;
    let handle = medium.global()?;
    let size = GlobalSize(handle);
    let ptr = GlobalLock(handle) as *const u16;
    if ptr.is_null() {
        return None;
    }
    let max = size / 2;
    let mut len = 0;
    while len < max && *ptr.add(len) != 0 {
        len += 1;
    }
    let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
    GlobalUnlock(handle);
    Some(text)
}

// ---- minimal FFI for the APIs the enabled `windows` features do not expose ----

/// Layout-compatible `STGMEDIUM` (x64: tymed, union pointer, IUnknown*).
#[repr(C)]
struct StgMedium {
    tymed: u32,
    data: *mut c_void,
    release: *mut c_void,
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalLock(h: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(h: *mut c_void) -> i32;
    fn GlobalSize(h: *mut c_void) -> usize;
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
}

#[link(name = "ole32")]
extern "system" {
    fn ReleaseStgMedium(medium: *mut StgMedium);
}

// ---- .url shortcut helper ----

/// Writes a Windows `.url` internet shortcut into `dir`, named after `title`
/// or the URL's host (made filesystem-safe and unique). Returns its path.
pub fn save_url_shortcut(dir: &Path, url: &str, title: Option<&str>) -> io::Result<PathBuf> {
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    let url = url.trim();
    if url.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty URL"));
    }
    std::fs::create_dir_all(dir)?;

    let base = title
        .map(sanitize_file_name)
        .filter(|n| !n.is_empty())
        .or_else(|| Some(sanitize_file_name(&host_of(url))).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| "Link".to_string());

    let content = format!("[InternetShortcut]\r\nURL={url}\r\n");
    for n in 1..1000 {
        let name = if n == 1 { format!("{base}.url") } else { format!("{base} ({n}).url") };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many shortcuts with the same name"))
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let cleaned: String = cleaned.trim().chars().take(80).collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&cleaned)) {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_hosts() {
        assert_eq!(host_of("https://www.example.com:8080/a?b#c"), "example.com");
        assert_eq!(sanitize_file_name("a/b:c*?. "), "a_b_c__");
        assert_eq!(sanitize_file_name("con"), "_con");
        assert_eq!(as_web_url("HTTPS://x.y\nmore"), Some("HTTPS://x.y"));
        assert_eq!(as_web_url("ftp://x"), None);
    }

    #[test]
    fn shortcut_is_unique() {
        let dir = std::env::temp_dir().join(format!("ada-url-test-{}", std::process::id()));
        let a = save_url_shortcut(&dir, "https://example.com/x", None).unwrap();
        let b = save_url_shortcut(&dir, "https://example.com/y", Some("Example")).unwrap();
        let c = save_url_shortcut(&dir, "https://example.com/z", None).unwrap();
        assert_eq!(a.file_name().unwrap(), "example.com.url");
        assert_eq!(b.file_name().unwrap(), "Example.url");
        assert_eq!(c.file_name().unwrap(), "example.com (2).url");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "[InternetShortcut]\r\nURL=https://example.com/x\r\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
