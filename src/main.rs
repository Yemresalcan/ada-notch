#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod droptarget;
mod gfx;
mod http;
mod mascot;
mod pages;
mod shell;
mod store;
mod theme;
mod worker;

use windows::core::{w, Result};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

fn main() -> Result<()> {
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\Ada.SingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        // OLE (STA) on the UI thread: needed for drag-and-drop with the shelf.
        OleInitialize(None)?;
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    shell::run()
}
