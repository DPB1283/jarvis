use super::{
    ToolDef, ToolOutput,
    ps::{ps, psq},
    s, safe,
};
use anyhow::{Context, Result, bail, ensure};
use jarvis_desktop::{
    input,
    monitors::{self, Monitor},
    window::{self, Rect},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex, time::Duration};

static ACTIVE: Mutex<Option<Monitor>> = Mutex::new(None);
static POINTER: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static LATEST: Mutex<Option<PathBuf>> = Mutex::new(None);
fn rect(m: Monitor) -> Rect {
    Rect {
        left: m.x,
        top: m.y,
        right: m.x + m.width,
        bottom: m.y + m.height,
    }
}
pub(crate) fn reset() {
    let fg = window::foreground().monitor;
    let all = monitors::list();
    *ACTIVE.lock().unwrap() = all
        .iter()
        .find(|m| Some(rect(**m)) == fg)
        .or_else(|| all.first())
        .copied();
}
fn active() -> Result<Monitor> {
    let all = monitors::list();
    let selected = *ACTIVE.lock().unwrap();
    all.iter()
        .find(|s| Some(**s) == selected)
        .or_else(|| all.first())
        .copied()
        .context("No monitor available")
}
fn select(index: u64) -> Result<Monitor> {
    let monitor = monitors::list()
        .into_iter()
        .find(|m| m.index as u64 == index)
        .context("Invalid screen number")?;
    *ACTIVE.lock().unwrap() = Some(monitor);
    Ok(monitor)
}
fn scale(w: i32, h: i32) -> f64 {
    (1568.0 / (w.max(h) as f64))
        .min((1_150_000.0 / (w as f64 * h as f64)).sqrt())
        .min(1.0)
}
pub(super) async fn snapshot(rect: Rect) -> Result<Value> {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    ensure!(w > 0 && h > 0, "Window is not visible");
    let scale = scale(w, h);
    let out_w = ((w as f64 * scale) as i32).max(1);
    let out_h = ((h as f64 * scale) as i32).max(1);
    let script = format!(
        r#"$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Display -Name Dpi -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr SetThreadDpiAwarenessContext(System.IntPtr context);'
[void][Display.Dpi]::SetThreadDpiAwarenessContext([IntPtr](-4))
$b=New-Object Drawing.Bitmap({w},{h})
$g=[Drawing.Graphics]::FromImage($b)
$g.CopyFromScreen({x},{y},0,0,$b.Size)
$o=New-Object Drawing.Bitmap($b,{out_w},{out_h})
$m=New-Object IO.MemoryStream
$o.Save($m,[Drawing.Imaging.ImageFormat]::Jpeg)
[Convert]::ToBase64String($m.ToArray())"#,
        x = rect.left,
        y = rect.top
    );
    let data = ps(&script).await?;
    ensure!(data.starts_with("/9j/"), "Screenshot capture failed");
    Ok(json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":data}}))
}

fn coords(m: Monitor, xy: &Value) -> Result<(i32, i32)> {
    let x = xy[0].as_i64().context("Missing X coordinate")?;
    let y = xy[1].as_i64().context("Missing Y coordinate")?;
    let scale = scale(m.width, m.height);
    let (w, h) = (
        (m.width as f64 * scale).floor() as i64,
        (m.height as f64 * scale).floor() as i64,
    );
    ensure!(
        (0..w).contains(&x) && (0..h).contains(&y),
        "Coordinate outside active screen"
    );
    Ok((
        m.x + (x * m.width as i64 / w) as i32,
        m.y + (y * m.height as i64 / h) as i32,
    ))
}
pub(crate) async fn action(name: &str, v: &Value, bus: &jarvis_core::Bus) -> Result<Value> {
    let m = active()?;
    let text = s(v, "text").unwrap_or("");
    let xy = &v["coordinate"];
    let move_cursor = |point: &Value| -> Result<()> {
        let (x, y) = coords(m, point)?;
        *POINTER.lock().unwrap() = Some((x, y));
        bus.emit(jarvis_core::Event::VirtualPointer { x, y });
        input::virtual_mouse(x, y, "move", "")?;
        Ok(())
    };
    match name {
        "screenshot" => return Ok(json!([snapshot(rect(m)).await?])),
        "zoom" => {
            let r = v["region"]
                .as_array()
                .filter(|r| r.len() == 4)
                .context("Expected four region coordinates")?;
            let (left, top) = coords(m, &json!([r[0], r[1]]))?;
            let (right, bottom) = coords(m, &json!([r[2], r[3]]))?;
            return Ok(json!([snapshot(Rect {
                left,
                top,
                right,
                bottom
            })
            .await?]));
        }
        "cursor_position" => {
            let (x, y) = POINTER.lock().unwrap().unwrap_or_else(monitors::cursor);
            if !m.contains(x, y) {
                return Ok(json!("Cursor is on another screen"));
            }
            let scale = scale(m.width, m.height);
            return Ok(json!(format!(
                "X={}, Y={}",
                ((x - m.x) as f64 * scale).round(),
                ((y - m.y) as f64 * scale).round()
            )));
        }
        "type" => input::type_text(text)?,
        "key" => {
            for _ in 0..v["repeat"].as_u64().unwrap_or(1).clamp(1, 100) {
                input::chord(text)?;
            }
        }
        "wait" | "hold_key" => {
            let _held = if name == "hold_key" {
                Some(input::held(text)?)
            } else {
                None
            };
            tokio::time::sleep(Duration::from_secs_f64(
                v["duration"].as_f64().unwrap_or(1.).clamp(0., 300.),
            ))
            .await;
        }
        "mouse_move" => move_cursor(xy)?,
        "left_click_drag" => {
            let (sx, sy) = coords(m, &v["start_coordinate"])?;
            let (ex, ey) = coords(m, xy)?;
            bus.emit(jarvis_core::Event::VirtualPointer { x: sx, y: sy });
            input::virtual_mouse(sx, sy, "left_down", text)?;
            input::virtual_mouse(ex, ey, "drag_move", text)?;
            input::virtual_mouse(ex, ey, "left_up", text)?;
            *POINTER.lock().unwrap() = Some((ex, ey));
            bus.emit(jarvis_core::Event::VirtualPointer { x: ex, y: ey });
        }
        _ => {
            let (x, y) = if !xy.is_null() {
                let position = coords(m, xy)?;
                *POINTER.lock().unwrap() = Some(position);
                position
            } else {
                POINTER.lock().unwrap().unwrap_or_else(monitors::cursor)
            };
            bus.emit(jarvis_core::Event::VirtualPointer { x, y });
            let send = |action: &str| input::virtual_mouse(x, y, action, text);
            match name {
                "left_mouse_down" => send("left_down")?,
                "left_mouse_up" => send("left_up")?,
                "left_click" | "middle_click" | "right_click" | "double_click" | "triple_click" => {
                    let button = match name {
                        "middle_click" => "middle",
                        "right_click" => "right",
                        _ => "left",
                    };
                    for _ in 0..match name {
                        "double_click" => 2,
                        "triple_click" => 3,
                        _ => 1,
                    } {
                        send(&format!("{button}_down"))?;
                        send(&format!("{button}_up"))?;
                    }
                }
                "scroll" => {
                    let direction = s(v, "scroll_direction").unwrap_or("down");
                    for _ in 0..v["scroll_amount"].as_i64().unwrap_or(3).clamp(1, 100) {
                        send(direction)?;
                    }
                }
                _ => bail!("Unsupported computer action {name}"),
            }
        }
    }
    Ok(json!("OK"))
}
/// Where the orb sits inside the HUD window. Keep in sync with `jarvis-app/src/windows.rs`.
fn orb_rect(hud: Rect, orb: &jarvis_core::settings::Orb) -> Rect {
    use jarvis_core::settings::OrbCorner::{BottomLeft, BottomRight, TopRight};
    const WIDTH: f64 = 520.0;
    const HEIGHT: f64 = 640.0;
    const PADDING: f64 = 14.0;
    const GLOW: f64 = 10.0; // include the soft glow and timer ring around the globe
    let scale = f64::from(hud.right - hud.left) / WIDTH;
    let size = orb.size.clamp(48.0, 180.0);
    let x = if matches!(orb.corner, TopRight | BottomRight) { WIDTH - PADDING - size } else { PADDING };
    let y = if matches!(orb.corner, BottomLeft | BottomRight) { HEIGHT - PADDING - size } else { PADDING };
    let edge = |dip: f64, origin: i32, limit: i32| (origin + (dip * scale).round() as i32).clamp(origin, limit);
    Rect {
        left: edge(x - GLOW, hud.left, hud.right),
        top: edge(y - GLOW, hud.top, hud.bottom),
        right: edge(x + size + GLOW, hud.left, hud.right),
        bottom: edge(y + size + GLOW, hud.top, hud.bottom),
    }
}

/// Crop `base` to a region given as fractions (0..1) of its width and height.
fn crop(base: Rect, region: &Value) -> Result<Rect> {
    let fraction = |key: &str| -> Result<f64> {
        let value = region[key].as_f64().with_context(|| format!("region.{key} is missing"))?;
        ensure!((0.0..=1.0).contains(&value), "region.{key} must be between 0 and 1");
        Ok(value)
    };
    let (left, top, right, bottom) = (fraction("left")?, fraction("top")?, fraction("right")?, fraction("bottom")?);
    ensure!(left < right && top < bottom, "region must have left < right and top < bottom");
    let (width, height) = (f64::from(base.right - base.left), f64::from(base.bottom - base.top));
    let out = Rect {
        left: base.left + (left * width).round() as i32,
        top: base.top + (top * height).round() as i32,
        right: base.left + (right * width).round() as i32,
        bottom: base.top + (bottom * height).round() as i32,
    };
    ensure!(out.right - out.left >= 8 && out.bottom - out.top >= 8, "region is too small");
    Ok(out)
}

/// What to capture: the chosen screen (default), all screens, the active window, a window found by title or
/// program name, Jarvis himself (the orb), optionally cropped to a fractional region of that.
fn area(v: &Value, orb: &jarvis_core::settings::Orb) -> Result<Rect> {
    if let Some(i) = v["screen"].as_u64() {
        select(i)?;
    }
    let everything = || {
        monitors::list()
            .into_iter()
            .map(rect)
            .reduce(|a, b| Rect {
                left: a.left.min(b.left),
                top: a.top.min(b.top),
                right: a.right.max(b.right),
                bottom: a.bottom.max(b.bottom),
            })
            .context("No monitors")
    };
    let named = s(v, "window").map(str::trim).filter(|w| !w.is_empty());
    let mut base = if let Some(query) = named {
        window::find(query)
            .map(|found| found.bounds)
            .with_context(|| format!("No visible window matching \"{query}\". Take a screenshot to see what is open."))?
    } else {
        match s(v, "area").unwrap_or("screen") {
            "screen" => rect(active()?),
            "active_window" => window::foreground().bounds.context("No foreground window")?,
            "all_screens" => everything()?,
            "jarvis" | "yourself" | "orb" => {
                ensure!(
                    orb.show_in_captures,
                    "I'm hidden from screenshots at the moment. Turn on \"Show Jarvis in screen shares, recordings and screenshots\" in Settings, Display."
                );
                let hud = window::find("Jarvis HUD").map(|f| f.bounds).context("I can't find my own window")?;
                orb_rect(hud, orb)
            }
            _ => bail!("Unknown screenshot area"),
        }
    };
    // Stay on the desktop (a window can overhang a screen edge, which would capture black).
    if let Ok(desktop) = everything() {
        base = Rect {
            left: base.left.max(desktop.left),
            top: base.top.max(desktop.top),
            right: base.right.min(desktop.right),
            bottom: base.bottom.min(desktop.bottom),
        };
        ensure!(base.right > base.left && base.bottom > base.top, "That area is off screen");
    }
    if v["region"].is_object() {
        base = crop(base, &v["region"])?;
    }
    Ok(base)
}
async fn save(v: &Value, memory: bool, orb: &jarvis_core::settings::Orb) -> Result<ToolOutput> {
    tokio::time::sleep(Duration::from_secs_f64(
        v["delay_seconds"].as_f64().unwrap_or(0.).clamp(0., 10.),
    ))
    .await;
    let r = area(v, orb)?;
    let mut dir = PathBuf::from(std::env::var_os("USERPROFILE").context("Missing user profile")?)
        .join("Pictures/Jarvis Screenshots");
    if memory {
        dir.push("Memories");
    }
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("Screenshot {}.png", uuid::Uuid::new_v4()));
    let path = file.to_string_lossy();
    ps(&format!(r#"$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Display -Name Dpi -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr SetThreadDpiAwarenessContext(System.IntPtr v);'
[void][Display.Dpi]::SetThreadDpiAwarenessContext([IntPtr](-4))
$b=New-Object Drawing.Bitmap({w},{h})
$g=[Drawing.Graphics]::FromImage($b)
$g.CopyFromScreen({x},{y},0,0,$b.Size)
$b.Save({path},[Drawing.Imaging.ImageFormat]::Png)"#,w=r.right-r.left,h=r.bottom-r.top,x=r.left,y=r.top,path=psq(&path))).await?;
    *LATEST.lock().unwrap() = Some(file.clone());
    if memory {
        let saved_view = ps(&format!(
            r#"Add-Type -AssemblyName System.Drawing
$i=[Drawing.Image]::FromFile({})
$b=New-Object Drawing.Bitmap($i,{},{})
$m=New-Object IO.MemoryStream
$b.Save($m,[Drawing.Imaging.ImageFormat]::Jpeg)
[Convert]::ToBase64String($m.ToArray())"#,
            psq(&path),
            ((r.right - r.left) as f64 * scale(r.right - r.left, r.bottom - r.top)).max(1.) as i32,
            ((r.bottom - r.top) as f64 * scale(r.right - r.left, r.bottom - r.top)).max(1.) as i32
        ))
        .await?;
        Ok(ToolOutput::Blocks(vec![
            json!({"type":"text","text":format!("Saved {path}. Use remember with image_path.")}),
            json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":saved_view}}),
        ]))
    } else {
        Ok(format!("Saved {path}").into())
    }
}
fn capture_schema() -> Value {
    json!({"type":"object","properties":{
        "area":{"type":"string","enum":["screen","all_screens","active_window","jarvis"],"description":"What to capture. jarvis is a photo of Jarvis himself (his orb)."},
        "window":{"type":"string","description":"Capture just one window, found by part of its title or program name, e.g. Discord, Chrome or Notepad. Overrides area."},
        "region":{"type":"object","description":"Crop to part of the captured area, as fractions from 0 to 1 of its width and height (left/top/right/bottom). Use after seeing a screenshot, e.g. the top-right quarter is left 0.5, top 0, right 1, bottom 0.5.","properties":{"left":{"type":"number"},"top":{"type":"number"},"right":{"type":"number"},"bottom":{"type":"number"}},"required":["left","top","right","bottom"]},
        "screen":{"type":"integer","minimum":1},"delay_seconds":{"type":"number","minimum":0,"maximum":10}}})
}
pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "take_screenshot",
            description: "Save a full-resolution screenshot to Pictures. Can capture the whole screen, one window (window), a cropped part of the screen or a window (region), or a photo of Jarvis himself (area jarvis).",
            schema: capture_schema,
            risk: safe,
            summary: |_| "save a screenshot".into(),
            run: |ctx, v| { let orb = ctx.settings.get().orb; Box::pin(async move { save(&v, false, &orb).await }) },
        },
        ToolDef {
            name: "remember_screen",
            description: "Save a screenshot for visual memory and return the image and its path. Accepts the same area, window and region options as take_screenshot.",
            schema: capture_schema,
            risk: safe,
            summary: |_| "remember the screen".into(),
            run: |ctx, v| { let orb = ctx.settings.get().orb; Box::pin(async move { save(&v, true, &orb).await }) },
        },
        ToolDef {
            name: "look_at_screens",
            description: "Inspect all monitors separately.",
            risk: safe,
            summary: |_| "look at the screens".into(),
            schema: || json!({"type":"object","properties":{}}),
            run: |_, _| {
                Box::pin(async {
                    let mut out = vec![];
                    let current = active()?;
                    for m in monitors::list() {
                        out.push(json!({"type":"text","text":format!("Screen {} ({}x{}), active: {}",m.index,m.width,m.height,m==current)}));
                        out.push(snapshot(rect(m)).await?);
                    }
                    Ok(ToolOutput::Blocks(out))
                })
            },
        },
        ToolDef {
            name: "use_screen",
            description: "Select a monitor for subsequent computer actions and inspect it.",
            risk: safe,
            summary: |_| "switch active screen".into(),
            schema: || json!({"type":"object","required":["screen"],"properties":{"screen":{"type":"integer","minimum":1}}}),
            run: |_, v| {
                Box::pin(async move {
                    let m = select(v["screen"].as_u64().context("Missing monitor")?)?;
                    Ok(ToolOutput::Blocks(vec![snapshot(rect(m)).await?]))
                })
            },
        },
        ToolDef {
            name: "use_screenshot",
            description: "Open, reveal, copy or paste a saved screenshot.",
            schema: || json!({"type":"object","required":["action"],"properties":{"action":{"type":"string","enum":["open","show_in_folder","copy","paste_here"]},"path":{"type":"string"}}}),
            risk: safe,
            summary: |_| "use a saved screenshot".into(),
            run: |_, v| {
                Box::pin(async move {
                    let path = s(&v, "path")
                        .map(PathBuf::from)
                        .or_else(|| LATEST.lock().unwrap().clone())
                        .context("No screenshot available")?;
                    ensure!(path.is_file(), "Screenshot does not exist");
                    let p = path.to_string_lossy();
                    match v["action"].as_str() {
                        Some("open") => {
                            std::process::Command::new("explorer.exe")
                                .arg(&path)
                                .spawn()?;
                        }
                        Some("show_in_folder") => {
                            std::process::Command::new("explorer.exe")
                                .arg(format!("/select,{p}"))
                                .spawn()?;
                        }
                        Some("copy") => {
                            ps(&format!("$ErrorActionPreference='Stop';Add-Type -AssemblyName System.Drawing,System.Windows.Forms;$i=[Drawing.Image]::FromFile({});[Windows.Forms.Clipboard]::SetImage($i)",psq(&p))).await?;
                        }
                        Some("paste_here") => {
                            super::clipboard::paste(window::foreground().hwnd, "", Some(&p)).await?
                        }
                        _ => bail!("Unknown screenshot action"),
                    }
                    Ok("Done.".into())
                })
            },
        },
    ]
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use jarvis_core::settings::{Orb, OrbCorner};

    fn r(l: i32, t: i32, rt: i32, b: i32) -> Rect {
        Rect { left: l, top: t, right: rt, bottom: b }
    }

    #[test]
    fn orb_is_cropped_out_of_its_hud_window() {
        let orb = Orb { size: 96.0, corner: OrbCorner::TopRight, ..Orb::default() };
        let crop = orb_rect(r(0, 0, 520, 640), &orb);
        assert_eq!((crop.left, crop.top, crop.right, crop.bottom), (400, 4, 516, 120));
        // A 150% display scales every edge with the window.
        let scaled = orb_rect(r(1000, 50, 1780, 1010), &orb);
        assert_eq!((scaled.left, scaled.top, scaled.right, scaled.bottom), (1600, 56, 1774, 230));
        let bottom_left = orb_rect(r(0, 0, 520, 640), &Orb { corner: OrbCorner::BottomLeft, ..orb });
        assert_eq!((bottom_left.left, bottom_left.top), (4, 520));
    }

    #[test]
    fn region_crops_by_fraction_and_rejects_nonsense() {
        let base = r(100, 200, 1100, 1200);
        let q = crop(base, &json!({"left":0.5,"top":0,"right":1,"bottom":0.5})).unwrap();
        assert_eq!((q.left, q.top, q.right, q.bottom), (600, 200, 1100, 700));
        assert!(crop(base, &json!({"left":0.6,"top":0,"right":0.5,"bottom":1})).is_err());
        assert!(crop(base, &json!({"left":0,"top":0,"right":1.5,"bottom":1})).is_err());
        assert!(crop(base, &json!({"left":0,"top":0,"right":0.001,"bottom":0.001})).is_err());
        assert!(crop(base, &json!({"left":0,"top":0,"right":1})).is_err());
    }
}
