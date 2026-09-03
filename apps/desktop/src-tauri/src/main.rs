#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity_probe;
mod game_backend;
mod input_probe;
mod window_probe;

use activity_probe::ActivityProbe;
use game_backend::{EconomySnapshot, GameEngine, PurchaseRequest, SaleRequest, TradeQuote};
use input_probe::{InputController, InputHealth, PermissionState, RuntimeProbe};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    AppHandle, Manager, RunEvent, State, WindowEvent,
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};

const MENU_SHOW: &str = "show-main";
const MENU_HIDE: &str = "hide-main";
const MENU_QUIT: &str = "quit";

struct GameAppState {
    runtime: Arc<RuntimeProbe>,
    activity: Arc<ActivityProbe>,
    input: InputController,
    game: GameEngine,
    shutting_down: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GameSnapshot {
    permission: PermissionState,
    input_permission_required: bool,
    input_health: InputHealth,
    total_effective_inputs: u64,
    economy: EconomySnapshot,
}

#[tauri::command]
fn game_snapshot(state: State<'_, GameAppState>) -> Result<GameSnapshot, String> {
    let productive = state.activity.sample();
    Ok(GameSnapshot {
        permission: state.runtime.permission(),
        input_permission_required: cfg!(target_os = "macos"),
        input_health: state.runtime.health(),
        total_effective_inputs: state.runtime.total_effective_inputs(),
        economy: state
            .game
            .snapshot(state.runtime.total_effective_inputs(), productive)?,
    })
}

#[tauri::command]
fn quote_purchase(
    request: PurchaseRequest,
    state: State<'_, GameAppState>,
) -> Result<TradeQuote, String> {
    state.game.quote_purchase(&request)
}

#[tauri::command]
fn commit_purchase(
    request: PurchaseRequest,
    state: State<'_, GameAppState>,
) -> Result<EconomySnapshot, String> {
    state.game.purchase(&request)
}

#[tauri::command]
fn quote_sale(request: SaleRequest, state: State<'_, GameAppState>) -> Result<TradeQuote, String> {
    state.game.quote_sale(&request)
}

#[tauri::command]
fn commit_sale(
    request: SaleRequest,
    state: State<'_, GameAppState>,
) -> Result<EconomySnapshot, String> {
    state.game.sale(&request)
}

#[tauri::command]
fn request_input_permission(
    app: AppHandle,
    state: State<'_, GameAppState>,
) -> Result<bool, String> {
    window_probe::set_main_window_always_on_top(&app, false)?;
    if input_probe::preflight_permission() || input_probe::request_permission() {
        enable_input(&app, &state)?;
        window_probe::set_main_window_always_on_top(&app, true)?;
        Ok(true)
    } else {
        state.runtime.set_permission(PermissionState::Denied);
        state.runtime.set_health(InputHealth::Stopped);
        Ok(false)
    }
}

#[tauri::command]
fn refresh_input_permission(
    app: AppHandle,
    state: State<'_, GameAppState>,
) -> Result<bool, String> {
    let allowed = if input_probe::preflight_permission() {
        enable_input(&app, &state)?;
        true
    } else {
        state.runtime.set_permission(PermissionState::Denied);
        state.runtime.set_health(InputHealth::Stopped);
        false
    };
    window_probe::set_main_window_always_on_top(&app, true)?;
    Ok(allowed)
}

#[tauri::command]
#[cfg(target_os = "macos")]
fn open_input_monitoring_settings() -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
        .status()
        .map_err(|error| format!("failed to open Input Monitoring settings: {error}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "failed to open Input Monitoring settings".to_owned())
}

#[tauri::command]
#[cfg(not(target_os = "macos"))]
fn open_input_monitoring_settings() -> Result<(), String> {
    Err("Input Monitoring settings are only available on macOS".to_owned())
}

fn enable_input(app: &AppHandle, state: &GameAppState) -> Result<(), String> {
    if state.runtime.permission() != PermissionState::Allowed
        || state.runtime.health() == InputHealth::Stopped
    {
        state.input.stop(&state.runtime);
        state.runtime.set_permission(PermissionState::Allowed);
        start_input(app, state)?;
    }
    Ok(())
}

fn start_input(_app: &AppHandle, state: &GameAppState) -> Result<(), String> {
    state
        .input
        .start(Arc::clone(&state.runtime), Arc::new(|| {}))
}

fn prepare_exit(app: &AppHandle) {
    let state = app.state::<GameAppState>();
    if state
        .shutting_down
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        state.input.stop(&state.runtime);
    }
}

fn install_application_menu(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id(MENU_SHOW, "显示牧场").build(app)?;
    let hide = MenuItemBuilder::with_id(MENU_HIDE, "隐藏牧场（继续运行）")
        .accelerator("CmdOrCtrl+W")
        .build(app)?;
    let quit = MenuItemBuilder::with_id(MENU_QUIT, "退出敲敲牧场")
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;
    let submenu = SubmenuBuilder::new(app, "敲敲牧场")
        .item(&show)
        .item(&hide)
        .separator()
        .item(&quit)
        .build()?;
    let menu = MenuBuilder::new(app).item(&submenu).build()?;
    app.set_menu(menu)?;
    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = window_probe::show_main_window(app);
        }))
        .invoke_handler(tauri::generate_handler![
            game_snapshot,
            request_input_permission,
            refresh_input_permission,
            open_input_monitoring_settings,
            quote_purchase,
            commit_purchase,
            quote_sale,
            commit_sale
        ])
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => {
                let _ = window_probe::show_main_window(app);
            }
            MENU_HIDE => {
                let _ = window_probe::hide_main_window(app);
            }
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .setup(|app| {
            let runtime = Arc::new(RuntimeProbe::new());
            let activity = ActivityProbe::new();
            activity.install().map_err(std::io::Error::other)?;
            let save_path = app
                .path()
                .app_data_dir()
                .map_err(std::io::Error::other)?
                .join("game-state.json");
            app.manage(GameAppState {
                runtime: Arc::clone(&runtime),
                activity,
                input: InputController::new(),
                game: GameEngine::load(save_path).map_err(std::io::Error::other)?,
                shutting_down: AtomicBool::new(false),
            });
            install_application_menu(app)?;
            window_probe::configure_desktop_window(app.handle()).map_err(std::io::Error::other)?;
            window_probe::install_dock_menu(app.handle().clone()).map_err(std::io::Error::other)?;
            if input_probe::preflight_permission() {
                let state = app.state::<GameAppState>();
                enable_input(app.handle(), &state).map_err(std::io::Error::other)?;
            } else {
                runtime.set_permission(PermissionState::Denied);
                runtime.set_health(InputHealth::Stopped);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Click Clack Farm Demo");

    app.run(|app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == window_probe::MAIN_WINDOW_LABEL => {
            api.prevent_close();
            let _ = window_probe::hide_main_window(app);
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            let _ = window_probe::show_main_window(app);
        }
        RunEvent::ExitRequested { .. } => prepare_exit(app),
        _ => {}
    });
}
