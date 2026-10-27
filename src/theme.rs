//! Visual constants: geometry, springs and colors.

use crate::gfx::{Color, Rect};

pub const COLLAPSED_W: f32 = 150.0;
pub const COLLAPSED_H: f32 = 30.0;
pub const EXPANDED_W: f32 = 420.0;
pub const EXPANDED_H: f32 = 140.0;
/// "Minimal notch" setting: a small, empty pill.
pub const MINIMAL_W: f32 = 104.0;
pub const MINIMAL_H: f32 = 24.0;
/// Size while showing a peek (notification, timer finished, ...).
pub const PEEK_W: f32 = 360.0;
pub const PEEK_H: f32 = 64.0;
/// Bottom corner radius = BASE + GROWTH * expansion (clamped to half the size).
pub const BOTTOM_RADIUS_BASE: f32 = 14.0;
pub const BOTTOM_RADIUS_GROWTH: f32 = 18.0;
/// Concave top "ear" radius = BASE + GROWTH * expansion.
pub const EAR_RADIUS_BASE: f32 = 10.0;
pub const EAR_RADIUS_GROWTH: f32 = 12.0;
/// Size springs (stiffness 230, damping 19 — a visible bounce).
pub const SIZE_STIFFNESS: f32 = 230.0;
pub const SIZE_DAMPING: f32 = 19.0;
/// Horizontal slide distance when switching pages.
pub const PAGE_SLIDE: f32 = 90.0;

/// Window size: room for the ears and spring overshoot.
pub const WINDOW_W: f32 = 640.0;
pub const WINDOW_H: f32 = 190.0;

pub const HEADER_H: f32 = 30.0;
/// Page content area, relative to the expanded panel's top-left corner.
pub const BODY: Rect = Rect { x: 38.0, y: 32.0, w: EXPANDED_W - 76.0, h: 88.0 };
pub const ARROW_RADIUS: f32 = 9.0;
pub const ARROW_INSET: f32 = 18.0;
pub const DOTS_Y: f32 = 130.0;
pub const DOT_SIZE: f32 = 5.0;
pub const DOT_ACTIVE_W: f32 = 14.0;
pub const DOT_GAP: f32 = 5.0;

pub const BG: Color = Color::rgba(0.0, 0.0, 0.0, 1.0);
pub const TEXT: Color = Color::rgba(1.0, 1.0, 1.0, 1.0);
pub const TEXT_DIM: Color = Color::rgba(1.0, 1.0, 1.0, 0.62);
pub const TEXT_FAINT: Color = Color::rgba(1.0, 1.0, 1.0, 0.38);
pub const CARD: Color = Color::rgba(1.0, 1.0, 1.0, 0.08);
pub const CARD_HOVER: Color = Color::rgba(1.0, 1.0, 1.0, 0.14);
pub const BORDER: Color = Color::rgba(1.0, 1.0, 1.0, 0.32);
pub const ACCENT: Color = Color::rgba(0.20, 0.86, 0.36, 1.0);
pub const WARN: Color = Color::rgba(1.0, 0.72, 0.16, 1.0);
pub const DANGER: Color = Color::rgba(1.0, 0.30, 0.28, 1.0);
pub const BLUE: Color = Color::rgba(0.30, 0.62, 1.0, 1.0);
