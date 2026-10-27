# Contributing to Ada

Thanks for your interest in contributing to Ada! This document covers how the codebase is structured, the rules that keep Ada tiny and responsive, and how to submit changes.

## Getting Started

Install [Rust](https://rustup.rs/) on Windows 10 (1903+) or Windows 11, then run:

```powershell
cargo run
```

Before opening a pull request, always format your code and verify that it compiles cleanly without warnings:

```powershell
cargo fmt
cargo build
```

## Adding a New Page

Each panel view in the expanded notch is a module under `src/pages/` implementing the `Page` trait from `src/pages/mod.rs`.

### 1. Create `src/pages/<name>.rs`

Give the page struct a `pub fn new(waker: Waker) -> Self` constructor and implement `Page`:

```rust
use crate::gfx::{Align, DrawCtx, Rect, TextStyle, Weight};
use crate::pages::Page;
use crate::theme;
use crate::worker::Waker;

pub struct ExamplePage {
    // Page state lives here.
}

impl ExamplePage {
    pub fn new(_waker: Waker) -> Self {
        Self {}
    }
}

impl Page for ExamplePage {
    fn title(&self) -> String {
        "Example".to_string()
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        ctx.fill_rect(area, 10.0, theme::CARD);
        ctx.text(
            "Hello from Ada",
            area,
            TextStyle::new(13.0, Weight::Medium, theme::TEXT).align(Align::Center),
        );
    }
}
```

Optional `Page` hooks you can override as needed:

- `poll(&mut self) -> bool` — Drain results from background workers; return `true` to request a redraw.
- `tick(&mut self) -> bool` — Called once per second while Ada is running.
- `on_mouse`, `on_char`, `on_key`, `editing`, `on_focus_lost` — Pointer and keyboard input inside `area`.
- `accepts_files`, `on_drop_files` — Handle files dragged onto the notch.
- `activity`, `draw_activity`, `activity_animating` — Show a live widget in the collapsed notch (such as media bars or a running timer).

### 2. Register the page in `src/pages/mod.rs`

1. Declare `mod <name>;` at the top of `src/pages/mod.rs`.
2. Add an `(id, label)` entry to `pages::FEATURES`:
   ```rust
   pub const FEATURES: &[(&str, &str)] = &[
       // ...
       ("example", "Example"),
   ];
   ```
3. Instantiate it in `pages::build`:
   ```rust
   "example" => Box::new(example::ExamplePage::new(waker)),
   ```

Every entry in `FEATURES` automatically gets an on/off toggle in the **Settings** page. Disabled pages are never constructed, so their background workers never start.

## Coding Rules

- **Never block the UI thread.** Network requests, slow file I/O, and blocking WinRT calls must run on a worker thread spawned with `worker::spawn`. Send results back over an `std::sync::mpsc` channel owned by the page, call `waker.wake()`, and drain the receiver in `Page::poll`:
  ```rust
  let (tx, rx) = mpsc::channel();
  worker::spawn("ada-example", move || {
      let result = fetch_something();
      let _ = tx.send(result);
      waker.wake();
  });
  ```
- **Draw only through `DrawCtx`.** Page modules must not touch Direct2D, Direct3D, or DirectComposition COM interfaces directly. Keep all rendering inside the `area: Rect` passed to `Page::draw` using `gfx::DrawCtx`.
- **Use colors and geometry from `theme.rs`.** Pick colors from `crate::theme` (`TEXT`, `TEXT_DIM`, `TEXT_FAINT`, `CARD`, `CARD_HOVER`, `BORDER`, `ACCENT`, `WARN`, `DANGER`, `BLUE`) so pages stay visually consistent.
- **Persist data via `store.rs`.** Use `crate::store` for plain-text settings/lists under `%APPDATA%\Ada` or caches under `%LOCALAPPDATA%\Ada`.
- **English UI strings.** All user-facing titles, labels, and status messages in the UI should be written in concise English.
- **Zero warnings & formatted code.** Run `cargo fmt` and ensure `cargo build` emits no warnings. Avoid adding external crates unless truly necessary — keeping the binary ~700 KB and dependency-light is a core goal.
- **Small, focused PRs.** Keep each pull request scoped to one feature or fix rather than bundling unrelated changes.

## Issue & Pull Request Etiquette

- **Search first:** Check open and closed issues/PRs before filing a duplicate.
- **Bug reports:** Include your Windows version (e.g., Windows 11 23H2), monitor DPI scaling if relevant, and clear steps to reproduce.
- **Feature ideas & new pages:** For non-trivial features or new pages, open an issue first so we can agree on scope and design before you spend time implementing it.
- **Be respectful:** Keep reviews and discussions constructive and welcoming.
- **License:** By submitting a contribution, you agree that it is licensed under the project's [Apache License 2.0](LICENSE).

## Cutting a Release

For maintainers publishing a new version:

1. Bump `version` in `Cargo.toml` and merge/push to `main`.
2. Tag the commit as `vX.Y.Z` and push the tag:
   ```powershell
   git tag v0.2.0
   git push origin v0.2.0
   ```
3. GitHub Actions automatically builds `ada.exe` and the `Ada-Setup-X.Y.Z.exe` Inno Setup installer and publishes the GitHub Release.
