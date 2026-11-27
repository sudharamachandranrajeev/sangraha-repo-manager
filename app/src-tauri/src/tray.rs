//! The system tray, when the owner turns it on in settings: Sangraha keeps running when its
//! window closes, reads GitHub every few hours, shows the digest's summary as the tray tooltip,
//! and sends a desktop notification when something new needs attention.

use std::time::Duration;

use sangraha_core::Config;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

const ID: &str = "sangraha";
/// How often the tray reads GitHub (about 1.5 MB each time).
const EVERY: Duration = Duration::from_secs(6 * 3600);

/// Puts the tray icon up, or takes it down.
pub fn set(app: &AppHandle, on: bool) {
    if !on {
        let _ = app.remove_tray_by_id(ID);
        return;
    }
    if app.tray_by_id(ID).is_some() {
        return;
    }
    if let Err(e) = build(app) {
        eprintln!("Sangraha: no tray icon: {e}");
    }
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Sangraha", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &quit])?;
    let mut tray = TrayIconBuilder::with_id(ID)
        .tooltip("Sangraha")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show(app),
            "check" => {
                let app = app.clone();
                std::thread::spawn(move || check_now(&app, true));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Makes the digest (reading GitHub first with `refresh`), puts its summary on the tray, and
/// with `refresh` notifies about new items and counts them as seen.
fn check_now(app: &AppHandle, refresh: bool) {
    let tip = match sangraha_core::digest(&Config::load(), refresh, refresh) {
        Ok(d) => {
            if refresh && d.new > 0 {
                let _ = sangraha_core::digest::notify(&d);
            }
            let _ = app.emit("checked", ());
            format!("Sangraha: {}", d.summary)
        }
        Err(e) => format!("Sangraha: {e}"),
    };
    if let Some(t) = app.tray_by_id(ID) {
        let _ = t.set_tooltip(Some(tip));
    }
}

/// The background check: the tooltip shortly after start (from what was last read), then a
/// fresh read every [`EVERY`] while the tray is on.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        if Config::load().run_in_tray {
            check_now(&app, false);
        }
        loop {
            std::thread::sleep(EVERY);
            if Config::load().run_in_tray {
                check_now(&app, true);
            }
        }
    });
}
