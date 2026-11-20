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
fn area(v: &Value) -> Result<Rect> {
    if let Some(i) = v["screen"].as_u64() {
        select(i)?;
    }
    match s(v, "area").unwrap_or("screen") {
        "screen" => Ok(rect(active()?)),
        "active_window" => window::foreground().bounds.context("No foreground window"),
        "all_screens" => monitors::list()
            .into_iter()
            .map(rect)
            .reduce(|a, b| Rect {
                left: a.left.min(b.left),
                top: a.top.min(b.top),
                right: a.right.max(b.right),
                bottom: a.bottom.max(b.bottom),
            })
            .context("No monitors"),
        _ => bail!("Unknown screenshot area"),
    }
}
async fn save(v: &Value, memory: bool) -> Result<ToolOutput> {
    tokio::time::sleep(Duration::from_secs_f64(
        v["delay_seconds"].as_f64().unwrap_or(0.).clamp(0., 10.),
    ))
    .await;
    let r = area(v)?;
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
    json!({"type":"object","properties":{"area":{"type":"string","enum":["screen","all_screens","active_window"]},"screen":{"type":"integer","minimum":1},"delay_seconds":{"type":"number","minimum":0,"maximum":10}}})
}
pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "take_screenshot",
            description: "Save a full-resolution screenshot to Pictures.",
            schema: capture_schema,
            risk: safe,
            summary: |_| "save a screenshot".into(),
            run: |_, v| Box::pin(async move { save(&v, false).await }),
        },
        ToolDef {
            name: "remember_screen",
            description: "Save a screenshot for visual memory and return the image and its path.",
            schema: capture_schema,
            risk: safe,
            summary: |_| "remember the screen".into(),
            run: |_, v| Box::pin(async move { save(&v, true).await }),
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
