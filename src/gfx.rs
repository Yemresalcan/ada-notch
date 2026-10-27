//! Direct3D 11 + Direct2D + DirectComposition rendering, and `DrawCtx`, the
//! small drawing API pages use. Pages never touch COM objects directly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    fn d2d(self, opacity: f32) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a * opacity }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> Point {
        Point { x: self.x + self.w / 2.0, y: self.y + self.h / 2.0 }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }

    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, w: (self.w - 2.0 * dx).max(0.0), h: (self.h - 2.0 * dy).max(0.0) }
    }

    /// Splits off a column of `width` from the left; returns (left, rest).
    pub fn split_left(&self, width: f32) -> (Rect, Rect) {
        let width = width.min(self.w);
        (Rect { w: width, ..*self }, Rect { x: self.x + width, w: self.w - width, ..*self })
    }

    /// Splits off a row of `height` from the top; returns (top, rest).
    pub fn split_top(&self, height: f32) -> (Rect, Rect) {
        let height = height.min(self.h);
        (Rect { h: height, ..*self }, Rect { y: self.y + height, h: self.h - height, ..*self })
    }

    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Ui,
    /// "Segoe Fluent Icons"; pass glyphs such as "\u{E8B7}" as the text.
    Icons,
    /// "Segoe UI Emoji", drawn in color (e.g. weather symbols).
    Emoji,
    /// "Segoe UI Emoji" glyph outlines filled with the style color (monochrome).
    EmojiMono,
    /// "Segoe Script" handwriting font.
    Script,
}

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub size: f32,
    pub weight: Weight,
    pub color: Color,
    pub align: Align,
    pub font: Font,
}

impl TextStyle {
    pub const fn new(size: f32, weight: Weight, color: Color) -> Self {
        Self { size, weight, color, align: Align::Left, font: Font::Ui }
    }

    pub const fn align(self, align: Align) -> Self {
        Self { align, ..self }
    }

    pub const fn icon(size: f32, color: Color) -> Self {
        Self { size, weight: Weight::Regular, color, align: Align::Center, font: Font::Icons }
    }

    /// Color emoji; the color only contributes opacity.
    pub const fn emoji(size: f32) -> Self {
        Self { size, weight: Weight::Regular, color: Color::rgba(1.0, 1.0, 1.0, 1.0), align: Align::Center, font: Font::Emoji }
    }

    /// Handwriting ("Segoe Script").
    pub const fn script(size: f32, color: Color) -> Self {
        Self { size, weight: Weight::Regular, color, align: Align::Left, font: Font::Script }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FormatKey {
    size_centi: u32,
    weight: Weight,
    align: Align,
    font: Font,
}

/// Device-independent resources shared by every frame.
struct Resources {
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    brush: ID2D1SolidColorBrush,
    round_stroke: ID2D1StrokeStyle,
    dash_stroke: ID2D1StrokeStyle,
    formats: RefCell<HashMap<FormatKey, IDWriteTextFormat>>,
    /// Decoded images keyed by the address of their embedded bytes.
    images: RefCell<HashMap<usize, ID2D1Bitmap>>,
}

impl Resources {
    fn format(&self, style: &TextStyle) -> Result<IDWriteTextFormat> {
        let key = FormatKey {
            size_centi: (style.size * 100.0).round() as u32,
            weight: style.weight,
            align: style.align,
            font: style.font,
        };
        if let Some(f) = self.formats.borrow().get(&key) {
            return Ok(f.clone());
        }
        let family = match style.font {
            Font::Ui => w!("Segoe UI Variable Text"),
            Font::Icons => w!("Segoe Fluent Icons"),
            Font::Emoji | Font::EmojiMono => w!("Segoe UI Emoji"),
            Font::Script => w!("Segoe Script"),
        };
        let weight = match style.weight {
            Weight::Regular => DWRITE_FONT_WEIGHT_NORMAL,
            Weight::Medium => DWRITE_FONT_WEIGHT_MEDIUM,
            Weight::Semibold => DWRITE_FONT_WEIGHT_SEMI_BOLD,
            Weight::Bold => DWRITE_FONT_WEIGHT_BOLD,
        };
        unsafe {
            let f = self.dwrite.CreateTextFormat(
                family,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                w!("en-us"),
            )?;
            f.SetTextAlignment(match style.align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = self.dwrite.CreateEllipsisTrimmingSign(&f)?;
            let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            f.SetTrimming(&trimming, &sign)?;
            self.formats.borrow_mut().insert(key, f.clone());
            Ok(f)
        }
    }
}

pub struct Gfx {
    dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    res: Resources,
}

impl Gfx {
    pub fn new(hwnd: HWND, px_w: u32, px_h: u32, scale: f32) -> Result<Self> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
            let device = device.ok_or_else(|| Error::from(E_FAIL))?;
            let dxgi: IDXGIDevice = device.cast()?;

            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dc = factory.CreateDevice(&dxgi)?.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

            let dxgi_factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: px_w,
                Height: px_h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = dxgi_factory.CreateSwapChainForComposition(&device, &desc, None)?;

            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0 * scale,
                dpiY: 96.0 * scale,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bitmap = dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            dc.SetTarget(&bitmap);

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let brush = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None)?;
            let mut stroke = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            // Use the base ID2D1Factory overload so the result is a plain ID2D1StrokeStyle.
            let base: &ID2D1Factory = &factory;
            let round_stroke = base.CreateStrokeStyle(&stroke, None)?;
            stroke.dashStyle = D2D1_DASH_STYLE_CUSTOM;
            stroke.dashCap = D2D1_CAP_STYLE_FLAT;
            let dash_stroke = base.CreateStrokeStyle(&stroke, Some(&[4.0, 3.0]))?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            Ok(Self {
                dc,
                swap,
                _dcomp: dcomp,
                _target: target,
                _visual: visual,
                res: Resources {
                    factory,
                    dwrite,
                    brush,
                    round_stroke,
                    dash_stroke,
                    formats: RefCell::new(HashMap::new()),
                    images: RefCell::new(HashMap::new()),
                },
            })
        }
    }

    /// Clears to transparent, runs `paint`, and presents (vsync-paced).
    pub fn frame(&self, paint: impl FnOnce(&mut DrawCtx)) -> Result<()> {
        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            let mut ctx = DrawCtx { dc: &self.dc, res: &self.res, opacity: 1.0 };
            paint(&mut ctx);
            self.dc.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    pub fn measure_text(&self, text: &str, style: &TextStyle) -> f32 {
        DrawCtx { dc: &self.dc, res: &self.res, opacity: 1.0 }.measure_text(text, style)
    }
}

/// Drawing API handed to pages. Coordinates are DIPs in window space.
pub struct DrawCtx<'a> {
    dc: &'a ID2D1DeviceContext,
    res: &'a Resources,
    /// Multiplied into every color's alpha (used for page transitions).
    pub opacity: f32,
}

impl DrawCtx<'_> {
    fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.res.brush.SetColor(&color.d2d(self.opacity)) };
        &self.res.brush
    }

    pub fn fill_rect(&mut self, r: Rect, radius: f32, color: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillRoundedRectangle(&rr, self.brush(color)) };
    }

    pub fn stroke_rect(&mut self, r: Rect, radius: f32, color: Color, width: f32, dashed: bool) {
        let rr = D2D1_ROUNDED_RECT { rect: r.inset(width / 2.0, width / 2.0).d2d(), radiusX: radius, radiusY: radius };
        let style = if dashed { &self.res.dash_stroke } else { &self.res.round_stroke };
        unsafe { self.dc.DrawRoundedRectangle(&rr, self.brush(color), width, style) };
    }

    pub fn fill_circle(&mut self, center: Point, radius: f32, color: Color) {
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: center.x, y: center.y }, radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillEllipse(&e, self.brush(color)) };
    }

    pub fn stroke_circle(&mut self, center: Point, radius: f32, color: Color, width: f32) {
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: center.x, y: center.y }, radiusX: radius, radiusY: radius };
        unsafe { self.dc.DrawEllipse(&e, self.brush(color), width, &self.res.round_stroke) };
    }

    pub fn line(&mut self, a: Point, b: Point, color: Color, width: f32) {
        unsafe {
            self.dc.DrawLine(
                D2D_POINT_2F { x: a.x, y: a.y },
                D2D_POINT_2F { x: b.x, y: b.y },
                self.brush(color),
                width,
                &self.res.round_stroke,
            )
        };
    }

    /// Clockwise arc starting at 12 o'clock covering `fraction` (0..=1) of a circle.
    pub fn arc(&mut self, center: Point, radius: f32, fraction: f32, color: Color, width: f32) {
        let fraction = fraction.clamp(0.0, 1.0);
        if fraction <= 0.0 {
            return;
        }
        if fraction >= 0.999 {
            self.stroke_circle(center, radius, color, width);
            return;
        }
        let angle = fraction * std::f32::consts::TAU;
        let start = D2D_POINT_2F { x: center.x, y: center.y - radius };
        let end = D2D_POINT_2F { x: center.x + radius * angle.sin(), y: center.y - radius * angle.cos() };
        let geometry = (|| -> Result<ID2D1PathGeometry> {
            unsafe {
                let geometry = self.res.factory.CreatePathGeometry()?;
                let sink = geometry.Open()?;
                sink.BeginFigure(start, D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: end,
                    size: D2D_SIZE_F { width: radius, height: radius },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: if fraction > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.Close()?;
                Ok(geometry.cast()?)
            }
        })();
        if let Ok(geometry) = geometry {
            unsafe { self.dc.DrawGeometry(&geometry, self.brush(color), width, &self.res.round_stroke) };
        }
    }

    pub fn fill_ellipse(&mut self, center: Point, rx: f32, ry: f32, color: Color) {
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: center.x, y: center.y }, radiusX: rx, radiusY: ry };
        unsafe { self.dc.FillEllipse(&e, self.brush(color)) };
    }

    fn gradient_stops(&self, stops: &[(f32, Color)]) -> Result<ID2D1GradientStopCollection> {
        let stops: Vec<D2D1_GRADIENT_STOP> = stops
            .iter()
            .map(|&(position, color)| D2D1_GRADIENT_STOP { position, color: color.d2d(self.opacity) })
            .collect();
        // Base ID2D1RenderTarget overload (the device-context one takes color spaces).
        let target: &ID2D1RenderTarget = self.dc;
        unsafe { target.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) }
    }

    /// Ellipse filled with a radial gradient from `inner` (center) to `outer` (edge).
    pub fn glow_ellipse(&mut self, center: Point, rx: f32, ry: f32, inner: Color, outer: Color) {
        let Ok(stops) = self.gradient_stops(&[(0.0, inner), (1.0, outer)]) else { return };
        let c = D2D_POINT_2F { x: center.x, y: center.y };
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
            center: c,
            gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
            radiusX: rx,
            radiusY: ry,
        };
        unsafe {
            if let Ok(brush) = self.dc.CreateRadialGradientBrush(&props, None, &stops) {
                let e = D2D1_ELLIPSE { point: c, radiusX: rx, radiusY: ry };
                self.dc.FillEllipse(&e, &brush);
            }
        }
    }

    /// Rounded rectangle filled with a linear gradient running from `from` to
    /// `to` (window DIPs) through the given `(offset 0..1, color)` stops.
    pub fn fill_rect_gradient(&mut self, r: Rect, radius: f32, from: Point, to: Point, stops: &[(f32, Color)]) {
        let Ok(stops) = self.gradient_stops(stops) else { return };
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F { x: from.x, y: from.y },
            endPoint: D2D_POINT_2F { x: to.x, y: to.y },
        };
        unsafe {
            if let Ok(brush) = self.dc.CreateLinearGradientBrush(&props, None, &stops) {
                let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
                self.dc.FillRoundedRectangle(&rr, &brush);
            }
        }
    }

    /// Stroked circular arc from `start_deg` sweeping `sweep_deg` clockwise
    /// (0° = 3 o'clock, angles grow clockwise as on screen); round caps.
    pub fn stroke_arc(&mut self, center: Point, radius: f32, start_deg: f32, sweep_deg: f32, color: Color, width: f32) {
        if sweep_deg.abs() < 0.01 || radius <= 0.0 {
            return;
        }
        let sweep = sweep_deg.clamp(-359.9, 359.9);
        let at = |deg: f32| {
            let a = deg.to_radians();
            D2D_POINT_2F { x: center.x + radius * a.cos(), y: center.y + radius * a.sin() }
        };
        let geometry = (|| -> Result<ID2D1PathGeometry> {
            unsafe {
                let geometry = self.res.factory.CreatePathGeometry()?;
                let sink = geometry.Open()?;
                sink.BeginFigure(at(start_deg), D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: at(start_deg + sweep),
                    size: D2D_SIZE_F { width: radius, height: radius },
                    rotationAngle: 0.0,
                    sweepDirection: if sweep > 0.0 { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                    arcSize: if sweep.abs() > 180.0 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                sink.Close()?;
                Ok(geometry.cast()?)
            }
        })();
        if let Ok(geometry) = geometry {
            unsafe { self.dc.DrawGeometry(&geometry, self.brush(color), width, &self.res.round_stroke) };
        }
    }

    /// Scales everything drawn afterwards by `scale` around `origin` and then
    /// shifts it by `offset`, until `reset_transform`.
    pub fn set_transform(&mut self, scale: f32, origin: Point, offset: Point) {
        let m = windows::Foundation::Numerics::Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: origin.x * (1.0 - scale) + offset.x,
            M32: origin.y * (1.0 - scale) + offset.y,
        };
        unsafe { self.dc.SetTransform(&m) };
    }

    /// Like `set_transform`, plus a clockwise rotation of `degrees` around `origin`.
    pub fn set_transform_rotated(&mut self, scale: f32, degrees: f32, origin: Point, offset: Point) {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let (a, b) = (scale * cos, scale * sin);
        let m = windows::Foundation::Numerics::Matrix3x2 {
            M11: a,
            M12: b,
            M21: -b,
            M22: a,
            M31: origin.x + offset.x - origin.x * a + origin.y * b,
            M32: origin.y + offset.y - origin.x * b - origin.y * a,
        };
        unsafe { self.dc.SetTransform(&m) };
    }

    /// Draws an embedded image (PNG/JPEG bytes, e.g. from `include_bytes!`)
    /// scaled into `dest`. Decoded once and cached for the process lifetime.
    pub fn image(&mut self, bytes: &'static [u8], dest: Rect, opacity: f32) {
        let key = bytes.as_ptr() as usize;
        let cached = self.res.images.borrow().get(&key).cloned();
        let bitmap = match cached {
            Some(b) => b,
            None => match decode_image(self.dc, bytes) {
                Ok(b) => {
                    self.res.images.borrow_mut().insert(key, b.clone());
                    b
                }
                Err(_) => return,
            },
        };
        unsafe {
            self.dc.DrawBitmap(
                &bitmap,
                Some(&dest.d2d()),
                (opacity * self.opacity).clamp(0.0, 1.0),
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            )
        };
    }

    /// Draws runtime image bytes (e.g. album art) identified by `key`; bytes
    /// are decoded on first use per key. Use a new key when the image changes.
    /// At most 16 such images are cached.
    pub fn image_dynamic(&mut self, key: u64, bytes: &[u8], dest: Rect, opacity: f32) {
        // Dynamic keys live in the upper half of the key space so they never
        // collide with the addresses used by `image`.
        let key = (key | (1 << 63)) as usize;
        let cached = self.res.images.borrow().get(&key).cloned();
        let bitmap = match cached {
            Some(b) => b,
            None => {
                let Ok(b) = decode_image(self.dc, bytes) else { return };
                let mut images = self.res.images.borrow_mut();
                let dynamic = images.keys().filter(|k| **k >= 1 << 63).count();
                if dynamic >= 16 {
                    images.retain(|k, _| *k < 1 << 63);
                }
                images.insert(key, b.clone());
                b
            }
        };
        unsafe {
            self.dc.DrawBitmap(
                &bitmap,
                Some(&dest.d2d()),
                (opacity * self.opacity).clamp(0.0, 1.0),
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            )
        };
    }

    pub fn reset_transform(&mut self) {
        let m = windows::Foundation::Numerics::Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };
        unsafe { self.dc.SetTransform(&m) };
    }

    /// The notch silhouette: flush with the top edge, concave
    /// "ears" of radius `ear` flaring out at both top corners, straight sides
    /// and rounded bottom corners of radius `bottom`. `body` excludes the ears.
    pub fn fill_notch(&mut self, body: Rect, bottom: f32, ear: f32, color: Color) {
        let bottom = bottom.min(body.w / 2.0).min(body.h / 2.0).max(0.0);
        let ear = ear.min(body.h - bottom).max(0.0);
        let (l, r, b) = (body.x, body.right(), body.bottom());
        let pt = |x: f32, y: f32| D2D_POINT_2F { x, y };
        let arc = |to: D2D_POINT_2F, radius: f32, dir: D2D1_SWEEP_DIRECTION| D2D1_ARC_SEGMENT {
            point: to,
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: dir,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        let geometry = (|| -> Result<ID2D1PathGeometry> {
            unsafe {
                let geometry = self.res.factory.CreatePathGeometry()?;
                let sink = geometry.Open()?;
                sink.BeginFigure(pt(l - ear, 0.0), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddArc(&arc(pt(l, ear), ear, D2D1_SWEEP_DIRECTION_CLOCKWISE));
                sink.AddLine(pt(l, b - bottom));
                sink.AddArc(&arc(pt(l + bottom, b), bottom, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
                sink.AddLine(pt(r - bottom, b));
                sink.AddArc(&arc(pt(r, b - bottom), bottom, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
                sink.AddLine(pt(r, ear));
                sink.AddArc(&arc(pt(r + ear, 0.0), ear, D2D1_SWEEP_DIRECTION_CLOCKWISE));
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                Ok(geometry.cast()?)
            }
        })();
        if let Ok(geometry) = geometry {
            unsafe { self.dc.FillGeometry(&geometry, self.brush(color), None) };
        }
    }

    /// Single line of text, vertically centered in `r`, trimmed with an ellipsis.
    pub fn text(&mut self, text: &str, r: Rect, style: TextStyle) {
        let Ok(format) = self.res.format(&style) else { return };
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.dc.DrawText(
                &wide,
                &format,
                &r.d2d(),
                self.brush(style.color),
                if style.font == Font::EmojiMono { D2D1_DRAW_TEXT_OPTIONS_NONE } else { D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT },
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    /// Width in DIPs of `text` rendered with `style`.
    pub fn measure_text(&self, text: &str, style: &TextStyle) -> f32 {
        let Ok(format) = self.res.format(style) else { return 0.0 };
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.res.dwrite.CreateTextLayout(&wide, &format, 10_000.0, 1_000.0) else { return 0.0 };
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_ok() {
                metrics.widthIncludingTrailingWhitespace
            } else {
                0.0
            }
        }
    }

    /// Restricts drawing to `r` until the matching `pop_clip`.
    pub fn push_clip(&mut self, r: Rect) {
        unsafe { self.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    pub fn pop_clip(&mut self) {
        unsafe { self.dc.PopAxisAlignedClip() };
    }
}

/// Decodes image bytes with WIC into a premultiplied BGRA Direct2D bitmap.
fn decode_image(dc: &ID2D1DeviceContext, bytes: &[u8]) -> Result<ID2D1Bitmap> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
        let target: &ID2D1RenderTarget = dc;
        target.CreateBitmapFromWicBitmap(&converter, None)
    }
}
