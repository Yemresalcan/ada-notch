//! Live CPU and memory usage gauges with a 60-second CPU sparkline.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

use crate::gfx::{Align, Color, DrawCtx, Point, Rect, TextStyle, Weight};
use crate::pages::Page;
use crate::theme;
use crate::worker::Waker;

const SPARK_CAP: usize = 60;
const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Clone, Copy)]
struct RamStats {
    used_gb: f32,
    total_gb: f32,
    percent: f32,
}

pub struct SystemPage {
    prev_times: Option<(u64, u64)>,
    cpu_pct: Option<f32>,
    history: [f32; SPARK_CAP],
    head: usize,
    len: usize,
    ram: Option<RamStats>,
}

impl SystemPage {
    pub fn new(_waker: Waker) -> Self {
        Self {
            prev_times: read_cpu_times(),
            cpu_pct: None,
            history: [0.0; SPARK_CAP],
            head: 0,
            len: 0,
            ram: read_ram(),
        }
    }

    fn push_sample(&mut self, value: f32) {
        self.history[self.head] = value;
        self.head = (self.head + 1) % SPARK_CAP;
        if self.len < SPARK_CAP {
            self.len += 1;
        }
    }

    fn sample_at(&self, index: usize) -> f32 {
        let start = if self.len < SPARK_CAP { 0 } else { self.head };
        self.history[(start + index) % SPARK_CAP]
    }
}

impl Page for SystemPage {
    fn title(&self) -> String {
        "System".to_string()
    }

    fn tick(&mut self) -> bool {
        if let Some((idle, total)) = read_cpu_times() {
            if let Some((prev_idle, prev_total)) = self.prev_times {
                let d_idle = idle.saturating_sub(prev_idle);
                let d_total = total.saturating_sub(prev_total);
                if d_total > 0 {
                    // Kernel time includes idle time on Win32.
                    let usage = (1.0 - d_idle as f64 / d_total as f64).clamp(0.0, 1.0) * 100.0;
                    let pct = usage as f32;
                    self.cpu_pct = Some(pct);
                    self.push_sample(pct);
                }
            }
            self.prev_times = Some((idle, total));
        }
        if let Some(ram) = read_ram() {
            self.ram = Some(ram);
        }
        true
    }

    fn draw(&mut self, ctx: &mut DrawCtx, area: Rect) {
        if self.cpu_pct.is_none() && self.ram.is_none() {
            ctx.stroke_rect(area.inset(0.0, 2.0), 10.0, theme::BORDER, 1.0, true);
            ctx.text(
                "No system data yet",
                area,
                TextStyle::new(12.5, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
            );
            return;
        }

        let (left, right_col) = area.split_left((area.w * 0.48).round());
        let ring_y = area.y + 35.0;
        let label_y = area.y + 67.0;
        let cpu_col = cpu_color(self.cpu_pct.unwrap_or(0.0));

        draw_gauge(
            ctx,
            Point { x: left.x + left.w * 0.25, y: ring_y },
            label_y,
            self.cpu_pct,
            "CPU",
            cpu_col,
        );
        draw_gauge(
            ctx,
            Point { x: left.x + left.w * 0.75, y: ring_y },
            label_y,
            self.ram.map(|r| r.percent),
            "RAM",
            theme::BLUE,
        );

        let card = Rect::new(
            right_col.x + 6.0,
            area.y + 2.0,
            (right_col.w - 6.0).max(0.0),
            (area.h - 4.0).max(0.0),
        );
        ctx.fill_rect(card, 10.0, theme::CARD);

        let inner = card.inset(10.0, 8.0);
        ctx.text(
            "CPU",
            Rect::new(inner.x, inner.y, inner.w, 13.0),
            TextStyle::new(10.5, Weight::Medium, theme::TEXT_FAINT),
        );
        ctx.text(
            "60s",
            Rect::new(inner.x, inner.y, inner.w, 13.0),
            TextStyle::new(10.5, Weight::Regular, theme::TEXT_FAINT).align(Align::Right),
        );

        let chart = Rect::new(inner.x, inner.y + 17.0, inner.w, (inner.h - 37.0).max(0.0));
        let guide = theme::TEXT_FAINT.with_alpha(0.16);
        ctx.line(
            Point { x: chart.x, y: chart.y + chart.h * 0.5 },
            Point { x: chart.right(), y: chart.y + chart.h * 0.5 },
            guide,
            1.0,
        );
        ctx.line(
            Point { x: chart.x, y: chart.bottom() },
            Point { x: chart.right(), y: chart.bottom() },
            guide,
            1.0,
        );

        if self.len == 0 {
            ctx.text(
                "Sampling…",
                chart,
                TextStyle::new(10.5, Weight::Regular, theme::TEXT_FAINT).align(Align::Center),
            );
        } else if self.len == 1 {
            let y = chart.bottom() - (self.sample_at(0) / 100.0).clamp(0.0, 1.0) * chart.h;
            ctx.line(Point { x: chart.x, y }, Point { x: chart.right(), y }, cpu_col, 1.5);
        } else {
            // Fixed 60 s time axis; the newest sample sits at the right edge.
            let step = chart.w / (SPARK_CAP - 1) as f32;
            let point_for = |i: usize| {
                let s = self.sample_at(i);
                Point {
                    x: chart.right() - (self.len - 1 - i) as f32 * step,
                    y: chart.bottom() - (s / 100.0).clamp(0.0, 1.0) * chart.h,
                }
            };
            let mut prev = point_for(0);
            for i in 1..self.len {
                let cur = point_for(i);
                ctx.line(prev, cur, cpu_col, 1.5);
                prev = cur;
            }
        }

        let ram_line = match self.ram {
            Some(r) => format!("RAM {:.1} / {:.1} GB", r.used_gb, r.total_gb),
            None => "RAM —".to_string(),
        };
        ctx.text(
            &ram_line,
            Rect::new(inner.x, inner.bottom() - 15.0, inner.w, 15.0),
            TextStyle::new(11.5, Weight::Medium, theme::TEXT_DIM),
        );
    }
}

fn cpu_color(pct: f32) -> Color {
    if pct > 90.0 {
        theme::DANGER
    } else if pct > 70.0 {
        theme::WARN
    } else {
        theme::ACCENT
    }
}

fn draw_gauge(ctx: &mut DrawCtx, center: Point, label_y: f32, pct: Option<f32>, label: &str, color: Color) {
    let radius = 26.0;
    let stroke = 5.0;
    ctx.stroke_circle(center, radius, color.with_alpha(0.18), stroke);
    if let Some(p) = pct {
        ctx.arc(center, radius, p / 100.0, color, stroke);
    }
    let (value_text, value_color) = match pct {
        Some(p) => (format!("{:.0}%", p.clamp(0.0, 100.0)), theme::TEXT),
        None => ("—".to_string(), theme::TEXT_DIM),
    };
    ctx.text(
        &value_text,
        Rect::new(center.x - 24.0, center.y - 12.0, 48.0, 24.0),
        TextStyle::new(12.5, Weight::Bold, value_color).align(Align::Center),
    );
    ctx.text(
        label,
        Rect::new(center.x - 32.0, label_y, 64.0, 18.0),
        TextStyle::new(11.0, Weight::Medium, theme::TEXT_DIM).align(Align::Center),
    );
}

fn filetime_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

fn read_cpu_times() -> Option<(u64, u64)> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let idle_u = filetime_u64(idle);
    let total_u = filetime_u64(kernel).wrapping_add(filetime_u64(user));
    Some((idle_u, total_u))
}

fn read_ram() -> Option<RamStats> {
    let mut mem = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut mem) }.ok()?;
    if mem.ullTotalPhys == 0 {
        return None;
    }
    let used = mem.ullTotalPhys.saturating_sub(mem.ullAvailPhys);
    Some(RamStats {
        used_gb: (used as f64 / BYTES_PER_GIB) as f32,
        total_gb: (mem.ullTotalPhys as f64 / BYTES_PER_GIB) as f32,
        percent: mem.dwMemoryLoad.min(100) as f32,
    })
}
