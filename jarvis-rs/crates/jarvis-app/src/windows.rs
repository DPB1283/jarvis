use anyhow::{Context, Result};
use jarvis_core::settings::{Orb, OrbCorner, Point};
use serde::Serialize;
use tauri::window::Monitor;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub const HUD: &str = "hud";
pub const SETTINGS: &str = "settings";
const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 640.0;
const PADDING: f64 = 14.0;

pub fn open_settings(app: &AppHandle) -> Result<()> {
    open_settings_at(app, None)
}

/// Open Settings on one tab ("browser", "api-keys", ...): index.html#settings/<tab>.
/// Every open runs on the main thread, one after another: two requests at once (the first-run tour and the
/// installer's browser setup) would otherwise both find no window and open two.
pub fn open_settings_at(app: &AppHandle, tab: Option<&str>) -> Result<()> {
    let handle = app.clone();
    let tab = tab.map(str::to_owned);
    app.run_on_main_thread(move || {
        if let Err(error) = open_settings_now(&handle, tab.as_deref()) {
            tracing::warn!(%error, "Could not open the settings window");
        }
    })?;
    Ok(())
}

fn open_settings_now(app: &AppHandle, tab: Option<&str>) -> Result<()> {
    let tab = tab.filter(|t| t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let hash = tab.map(|t| format!("#settings/{t}")).unwrap_or_else(|| "#settings".into());
    let window = if let Some(window) = app.get_webview_window(SETTINGS) {
        if tab.is_some() {
            window.eval(format!("location.hash = {hash:?}"))?;
        }
        window
    } else {
        WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App(format!("index.html{hash}").into()))
            .title("Jarvis settings")
            .inner_size(800.0, 800.0)
            .min_inner_size(520.0, 480.0)
            .center()
            .build()?
    };
    window.unminimize()?;
    window.show()?;
    window.set_focus()?;
    Ok(())
}

/// Make a window ignore the mouse. On Linux the native window only exists once it has been shown, so a hidden
/// window is shown for an instant first.
fn click_through(window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let hidden = !window.is_visible()?;
        if hidden {
            window.show()?;
        }
        window.set_ignore_cursor_events(true)?;
        if hidden {
            window.hide()?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        window.set_ignore_cursor_events(true)?;
        Ok(())
    }
}

pub fn create_hud(app: &AppHandle, orb: &Orb) -> Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, HUD, WebviewUrl::App("index.html".into()))
        .title("Jarvis HUD")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(!orb.show_in_captures)
        .focused(false)
        .visible(false)
        .build()?;
    position_hud(&window, orb)?;
    click_through(&window)?;
    window.show()?;
    Ok(window)
}

fn offset(corner: OrbCorner, size: f64) -> Point {
    let right = matches!(corner, OrbCorner::TopRight | OrbCorner::BottomRight);
    let bottom = matches!(corner, OrbCorner::BottomLeft | OrbCorner::BottomRight);
    Point {
        x: if right {
            WIDTH - PADDING - size
        } else {
            PADDING
        },
        y: if bottom {
            HEIGHT - PADDING - size
        } else {
            PADDING
        },
    }
}

fn work(monitor: &Monitor) -> (Point, Point) {
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    (
        Point {
            x: f64::from(area.position.x) / scale,
            y: f64::from(area.position.y) / scale,
        },
        Point {
            x: f64::from(area.size.width) / scale,
            y: f64::from(area.size.height) / scale,
        },
    )
}

fn clamp(point: Point, origin: Point, extent: Point, size: f64) -> Point {
    Point {
        x: point
            .x
            .clamp(origin.x, origin.x + (extent.x - size).max(0.0)),
        y: point
            .y
            .clamp(origin.y, origin.y + (extent.y - size).max(0.0)),
    }
}

/// The HUD window's size while the orb shows a hologram (logical pixels), before clamping to the screen.
const VISUAL_WIDTH: f64 = 760.0;
const VISUAL_HEIGHT: f64 = 820.0;

/// Grow the HUD window while the orb shows a hologram, so it has room: "normal" is a large square around the orb,
/// "large" (the "expand" command) fills the screen's work area, "closed" puts the small HUD back over the orb.
pub fn visual_layout(window: &WebviewWindow, orb: &Orb, mode: &str) -> Result<()> {
    if mode == "closed" {
        window.set_size(tauri::LogicalSize::new(WIDTH, HEIGHT))?;
        return position_hud(window, orb);
    }
    let size = orb.size.clamp(48.0, 180.0);
    let Some((monitor, orb_at)) = orb_spot(window, orb)? else {
        return Ok(());
    };
    let (origin, extent) = work(&monitor);
    let (width, height, left, top) = if mode == "large" {
        (extent.x, extent.y, origin.x, origin.y)
    } else {
        let (width, height) = (VISUAL_WIDTH.min(extent.x), VISUAL_HEIGHT.min(extent.y));
        let corner = clamp(
            Point { x: orb_at.x + size / 2.0 - width / 2.0, y: orb_at.y + size / 2.0 - height / 2.0 },
            origin,
            Point { x: extent.x - width + 1.0, y: extent.y - height + 1.0 },
            1.0,
        );
        (width, height, corner.x, corner.y)
    };
    let scale = monitor.scale_factor();
    window.set_position(PhysicalPosition::new((left * scale).round() as i32, (top * scale).round() as i32))?;
    window.set_size(tauri::LogicalSize::new(width, height))?;
    Ok(())
}

pub fn position_hud(window: &WebviewWindow, orb: &Orb) -> Result<()> {
    let Some((monitor, point)) = orb_spot(window, orb)? else {
        tracing::warn!("No monitor reported; not positioning the orb");
        return Ok(());
    };
    let size = orb.size.clamp(48.0, 180.0);
    let offset = offset(orb.corner, size);
    let scale = monitor.scale_factor();
    window.set_position(PhysicalPosition::new(
        ((point.x - offset.x) * scale).round() as i32,
        ((point.y - offset.y) * scale).round() as i32,
    ))?;
    Ok(())
}

/// The monitor the orb is on and the orb's top-left corner there (logical pixels).
fn orb_spot(window: &WebviewWindow, orb: &Orb) -> Result<Option<(Monitor, Point)>> {
    let size = orb.size.clamp(48.0, 180.0);
    let monitors = window.available_monitors()?;
    let monitor = orb
        .position
        .and_then(|point| {
            monitors
                .iter()
                .find(|monitor| {
                    let (origin, extent) = work(monitor);
                    let center = Point {
                        x: point.x + size / 2.0,
                        y: point.y + size / 2.0,
                    };
                    center.x >= origin.x
                        && center.x <= origin.x + extent.x
                        && center.y >= origin.y
                        && center.y <= origin.y + extent.y
                })
                .cloned()
        })
        .or(window.primary_monitor()?)
        .or(monitors.first().cloned());
    // Some X11 setups report no monitors at all. Leave the orb wherever the system put it instead of failing to start.
    let Some(monitor) = monitor else {
        return Ok(None);
    };
    let (origin, extent) = work(&monitor);
    let offset = offset(orb.corner, size);
    let saved = orb.position.unwrap_or(Point {
        x: origin.x
            + if offset.x > PADDING {
                extent.x - size - PADDING
            } else {
                PADDING
            },
        y: origin.y
            + if offset.y > PADDING {
                extent.y - size - PADDING
            } else {
                PADDING
            },
    });
    let point = clamp(saved, origin, extent, size);
    Ok(Some((monitor, point)))
}

pub fn dropped_orb(window: &WebviewWindow, orb: &Orb) -> Result<(Point, OrbCorner)> {
    let scale = window.scale_factor()?;
    let window_position = window.outer_position()?;
    let offset = offset(orb.corner, orb.size);
    let position = Point {
        x: f64::from(window_position.x) / scale + offset.x,
        y: f64::from(window_position.y) / scale + offset.y,
    };
    let center = Point {
        x: position.x + orb.size / 2.0,
        y: position.y + orb.size / 2.0,
    };
    let monitor = window
        .monitor_from_point(center.x * scale, center.y * scale)?
        .or(window.current_monitor()?)
        .context("HUD has no monitor.")?;
    let (origin, extent) = work(&monitor);
    let corner = match (
        center.x > origin.x + extent.x / 2.0,
        center.y > origin.y + extent.y / 2.0,
    ) {
        (false, false) => OrbCorner::TopLeft,
        (true, false) => OrbCorner::TopRight,
        (false, true) => OrbCorner::BottomLeft,
        (true, true) => OrbCorner::BottomRight,
    };
    Ok((clamp(position, origin, extent, orb.size), corner))
}

pub fn orb_center(window: &WebviewWindow, orb: &Orb) -> Result<PhysicalPosition<i32>> {
    let position = window.outer_position()?;
    let scale = window.scale_factor()?;
    let offset = offset(orb.corner, orb.size);
    Ok(PhysicalPosition::new(
        position.x + ((offset.x + orb.size / 2.0) * scale).round() as i32,
        position.y + ((offset.y + orb.size / 2.0) * scale).round() as i32,
    ))
}

pub struct Drag {
    cursor: PhysicalPosition<f64>,
    window: PhysicalPosition<i32>,
}

impl Drag {
    pub fn start(window: &WebviewWindow) -> Result<Self> {
        Ok(Self {
            cursor: window.cursor_position()?,
            window: window.outer_position()?,
        })
    }

    pub fn update(&self, window: &WebviewWindow, point: PhysicalPosition<f64>) -> Result<()> {
        window.set_position(PhysicalPosition::new(
            self.window.x + (point.x - self.cursor.x).round() as i32,
            self.window.y + (point.y - self.cursor.y).round() as i32,
        ))?;
        Ok(())
    }
}

#[derive(Clone, Serialize)]
struct Cursor {
    x: f64,
    y: f64,
}

/// WebView2 stops delivering pointer movement while its window is click-through.
/// Forward the cursor position while it crosses the HUD so the renderer can hit-test its controls.
pub fn forward_cursor(
    window: &WebviewWindow,
    cursor: PhysicalPosition<f64>,
    was_inside: &mut bool,
) -> Result<()> {
    let origin = window.inner_position()?;
    let extent = window.inner_size()?;
    let scale = window.scale_factor()?;
    let x = cursor.x - f64::from(origin.x);
    let y = cursor.y - f64::from(origin.y);
    let inside =
        x >= 0.0 && y >= 0.0 && x < f64::from(extent.width) && y < f64::from(extent.height);
    if inside {
        window.emit(
            "jarvis:cursor",
            Some(Cursor {
                x: x / scale,
                y: y / scale,
            }),
        )?;
    } else if *was_inside {
        window.emit("jarvis:cursor", Option::<Cursor>::None)?;
    }
    *was_inside = inside;
    Ok(())
}

pub fn create_pointer(app: &AppHandle, show_in_captures: bool) -> Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, "pointer", WebviewUrl::App("index.html#pointer".into()))
        .title("Jarvis pointer")
        .inner_size(110.0, 45.0)
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(!show_in_captures)
        .focused(false)
        .visible(false)
        .build()?;
    click_through(&window)?;
    Ok(window)
}
