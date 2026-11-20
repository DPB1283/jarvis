#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod windows;

use anyhow::{Result, bail};
use jarvis_brain::ipc::{InvokeError, handle_invoke};
use jarvis_brain::{BrainHandle, Ctx};
use jarvis_core::events::{ToastLevel, WakeSource};
use jarvis_core::settings::GameModeSetting;
use jarvis_core::{Bus, Event, Settings, SettingsStore};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use windows::{Drag, HUD, SETTINGS};

#[derive(Clone)]
struct AppState {
    bus: Bus,
    settings: SettingsStore,
    drag: Arc<Mutex<Option<Drag>>>,
    game: Arc<Mutex<Option<Event>>>,
    timers: Arc<Mutex<Option<Event>>>,
    ctx: Ctx,
    brain: Arc<Mutex<Option<BrainHandle>>>,
    voice: Arc<Mutex<Option<Arc<jarvis_voice::VoiceHandle>>>>,
}

fn warn(bus: &Bus, text: String) {
    bus.emit(Event::Toast {
        level: ToastLevel::Warn,
        text,
    });
}

fn talk(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Some(window) = app.get_webview_window(HUD) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    state.bus.emit(Event::Wake {
        source: WakeSource::Hotkey,
    });
}

fn open_settings(app: &AppHandle) {
    if let Err(error) = windows::open_settings(app) {
        tracing::warn!(%error, "Could not open the settings window");
    }
}

fn hotkeys(app: &AppHandle, state: &AppState, settings: &Settings) {
    let manager = app.global_shortcut();
    if let Err(error) = manager.unregister_all() {
        tracing::warn!(%error, "Could not release previous hotkeys");
    }
    for (key, action) in [
        (&settings.hotkeys.talk, talk as fn(&AppHandle)),
        (&settings.hotkeys.settings, open_settings as fn(&AppHandle)),
    ] {
        if key.trim().is_empty() {
            continue;
        }
        let key = key
            .replace("CommandOrControl", "Control")
            .replace("CmdOrCtrl", "Control");
        let installed = key
            .parse::<Shortcut>()
            .map_err(anyhow::Error::from)
            .and_then(|shortcut| {
                Ok(manager.on_shortcut(shortcut, move |app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            action(&app);
                        });
                    }
                })?)
            });
        if installed.is_err() {
            warn(&state.bus, format!("Could not register the {key} hotkey."));
        }
    }
}

fn tray(app: &AppHandle) -> Result<()> {
    let talk_item = MenuItem::with_id(app, "talk", "Talk to Jarvis", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Jarvis", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&talk_item, &settings, &quit])?;
    let mut icon = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Jarvis")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let app = app.clone();
            let id = event.id().as_ref().to_owned();
            tauri::async_runtime::spawn(async move {
                match id.as_str() {
                    "talk" => talk(&app),
                    "settings" => open_settings(&app),
                    "quit" => app.exit(0),
                    _ => {}
                }
            });
        })
        .on_tray_icon_event(|icon, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                talk(icon.app_handle());
            }
        });
    if let Some(image) = app.default_window_icon() {
        icon = icon.icon(image.clone());
    }
    icon.build(app)?;
    Ok(())
}

fn forward_events(app: AppHandle, state: AppState) {
    let mut receiver = state.bus.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut previous = state.settings.get();
        while let Some(event) = Bus::recv(&mut receiver).await {
            match &event {
                Event::VirtualPointer { x, y } => {
                    if let Some(pointer) = app.get_webview_window("pointer") {
                        let _ = pointer.set_position(tauri::PhysicalPosition::new(*x, *y));
                        let _ = pointer.show();
                        static POINTER_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                        let revision = POINTER_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            if POINTER_REVISION.load(std::sync::atomic::Ordering::Relaxed) == revision {
                                let _ = pointer.hide();
                            }
                        });
                    }
                }
                Event::SettingsChanged { settings } => {
                    if settings.hotkeys != previous.hotkeys {
                        hotkeys(&app, &state, settings);
                    }
                    if settings.orb.show_in_captures != previous.orb.show_in_captures {
                        for label in [HUD, "pointer"] {
                            if let Some(window) = app.get_webview_window(label) {
                                let _ = window.set_content_protected(!settings.orb.show_in_captures);
                            }
                        }
                    }
                    if settings.orb != previous.orb {
                        if let Some(window) = app.get_webview_window(HUD) {
                            if let Err(error) = windows::position_hud(&window, &settings.orb) {
                                tracing::warn!(%error, "Could not reposition the orb");
                            }
                        }
                    }
                    if settings.launch_at_startup != previous.launch_at_startup {
                        let manager = app.autolaunch();
                        let result = if settings.launch_at_startup {
                            manager.enable()
                        } else {
                            manager.disable()
                        };
                        if let Err(error) = result {
                            tracing::warn!(%error, "Could not update startup behavior");
                        }
                    }
                    previous = settings.as_ref().clone();
                }
                Event::GameMode { .. } => *state.game.lock().unwrap() = Some(event.clone()),
                Event::Timers { .. } => *state.timers.lock().unwrap() = Some(event.clone()),
                _ => {}
            }
            if event.for_renderer() {
                let _ = app.emit("jarvis:event", event);
            }
        }
    });
}

fn track_cursor(window: WebviewWindow, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(34));
        let mut previous = None;
        let mut inside = false;
        loop {
            interval.tick().await;
            let (x, y) = jarvis_desktop::monitors::cursor();
            let cursor = tauri::PhysicalPosition::new(f64::from(x), f64::from(y));
            if previous == Some(cursor) {
                continue;
            }
            previous = Some(cursor);
            let drag = state.drag.lock().unwrap();
            if let Some(drag) = drag.as_ref() {
                let _ = drag.update(&window, cursor);
            } else {
                let _ = windows::forward_cursor(&window, cursor, &mut inside);
            }
        }
    });
}

fn track_game(window: WebviewWindow, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
        let mut candidate = None::<u32>;
        let mut previous = (false, None, false);
        loop {
            interval.tick().await;
            let settings = state.settings.get();
            let next = match settings.game_mode.mode {
                GameModeSetting::Off => (false, None, false),
                GameModeSetting::On => (true, Some("manual".to_owned()), true),
                GameModeSetting::Auto => {
                    let foreground = jarvis_desktop::window::foreground();
                    if foreground.fullscreen
                        && foreground.pid != std::process::id()
                        && foreground.pid != 0
                    {
                        let active = candidate == Some(foreground.pid);
                        candidate = Some(foreground.pid);
                        let hides = active
                            && windows::orb_center(&window, &settings.orb)
                                .ok()
                                .is_some_and(|point| {
                                    foreground
                                        .monitor
                                        .is_some_and(|monitor| monitor.contains(point.x, point.y))
                                });
                        (active, active.then_some(foreground.exe), hides)
                    } else {
                        candidate = None;
                        (false, None, false)
                    }
                }
            };
            if next != previous {
                previous = next.clone();
                let (active, app, hide_hud) = next;
                state.bus.emit(Event::GameMode {
                    active,
                    app,
                    hide_hud,
                });
            }
        }
    });
}

#[tauri::command]
fn jarvis_send(
    state: State<'_, AppState>,
    window: WebviewWindow,
    event: Event,
) -> Result<(), String> {
    if window.label() != HUD || !event.renderer_may_send() {
        return Err("This renderer event is not allowed.".into());
    }
    state.bus.emit(event);
    Ok(())
}

#[tauri::command]
async fn jarvis_invoke(
    state: State<'_, AppState>,
    window: WebviewWindow,
    channel: String,
    args: Vec<Value>,
) -> Result<Value, String> {
    if window.label() == "pointer" && channel == "settings:get" {
        return Ok(serde_json::to_value(state.settings.get()).map_err(|e| e.to_string())?);
    }
    if !matches!(window.label(), HUD | SETTINGS) {
        return Err("Unknown Jarvis window.".into());
    }
    invoke(&state, &window, &channel, &args)
        .await
        .map_err(|error| error.to_string())
}

async fn invoke(
    state: &AppState,
    window: &WebviewWindow,
    channel: &str,
    args: &[Value],
) -> Result<Value> {
    let arg = args.first().unwrap_or(&Value::Null);
    match channel {
        "window:ready" => {
            for slot in [&state.game, &state.timers] {
                if let Some(event) = slot.lock().unwrap().as_ref() {
                    let _ = window.emit("jarvis:event", event.clone());
                }
            }
        }
        "window:open-settings" => windows::open_settings(window.app_handle())?,
        "window:set-ignore-mouse" if window.label() == HUD => {
            if state.drag.lock().unwrap().is_none() {
                window.set_ignore_cursor_events(
                    arg.as_bool()
                        .ok_or_else(|| anyhow::anyhow!("Expected a boolean."))?,
                )?;
            }
        }
        "window:drag" if window.label() == HUD => match arg.as_str() {
            Some("start") => *state.drag.lock().unwrap() = Some(Drag::start(window)?),
            Some("end") => {
                if state.drag.lock().unwrap().take().is_some() {
                    let (position, corner) =
                        windows::dropped_orb(window, &state.settings.get().orb)?;
                    state
                        .settings
                        .update(&json!({"orb":{"position":position,"corner":corner}}))?;
                }
            }
            _ => bail!("Unknown drag action."),
        },
        "settings:update" if window.label() == SETTINGS => {
            let mut patch = arg.clone();
            if patch.pointer("/orb/corner").is_some_and(|corner| {
                serde_json::to_value(state.settings.get().orb.corner)
                    .is_ok_and(|saved| saved != *corner)
            }) {
                patch["orb"]["position"] = Value::Null;
            }
            return Ok(handle_invoke(&state.ctx, channel, vec![patch]).await?);
        }
        "settings:update" => bail!("Open Settings to change preferences."),
        "audio:inputs" => {
            return Ok(json!(
                tauri::async_runtime::spawn_blocking(jarvis_voice::list_inputs).await??
            ));
        }
        "audio:test-input" => {
            let name = args.first().and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            return Ok(json!(jarvis_voice::test_input(name, args.get(1).and_then(Value::as_f64).unwrap_or(100.0)).await?));
        }
        "audio:outputs" => {
            return Ok(json!(
                tauri::async_runtime::spawn_blocking(jarvis_voice::list_outputs).await??
            ));
        }
        "voices:list" => return Ok(json!(jarvis_voice::list_voices().await?)),
        "voice:preview" => {
            let voice = state
                .voice
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| anyhow::anyhow!("The voice service is still starting."))?;
            voice
                .preview_voice(arg.as_str().unwrap_or_default().to_owned())
                .await?;
        }
        _ => {
            return match handle_invoke(&state.ctx, channel, args.to_vec()).await {
                Ok(result) => Ok(result),
                Err(InvokeError::Unhandled) => bail!("Unknown Jarvis command."),
                Err(error) => Err(error.into()),
            };
        }
    }
    Ok(Value::Null)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    jarvis_core::paths::load_env();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| talk(app)))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![jarvis_invoke, jarvis_send])
        .setup(|app| {
            let bus = Bus::new();
            let ctx = Ctx::new(bus.clone())?;
            let settings = ctx.settings.clone();
            let state = AppState {
                bus,
                settings,
                drag: Default::default(),
                game: Default::default(),
                timers: Default::default(),
                ctx,
                brain: Default::default(),
                voice: Default::default(),
            };
            app.manage(state.clone());
            let hud = windows::create_hud(app.handle(), &state.settings.get().orb)?;
            windows::create_pointer(app.handle(), state.settings.get().orb.show_in_captures)?;
            tray(app.handle())?;
            hotkeys(app.handle(), &state, &state.settings.get());
            forward_events(app.handle().clone(), state.clone());
            track_cursor(hud.clone(), state.clone());
            track_game(hud, state);
            let voice_state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let voice = jarvis_voice::start_voice(
                    voice_state.bus.clone(),
                    voice_state.settings.clone(),
                );
                *voice_state.voice.lock().unwrap() = Some(Arc::new(voice));
                *voice_state.brain.lock().unwrap() =
                    Some(jarvis_brain::start(voice_state.ctx.clone()));
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Jarvis could not start")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    state.voice.lock().unwrap().take();
                    if let Some(brain) = state.brain.lock().unwrap().take() {
                        brain.shutdown();
                    }
                }
            }
        });
}



