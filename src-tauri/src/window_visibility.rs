use tauri::{Emitter, WebviewWindow};

#[derive(Clone, serde::Serialize)]
pub struct WindowVisibility {
    visible: bool,
    focused: bool,
}

pub(crate) fn is_visible(window: &WebviewWindow) -> bool {
    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false)
}

fn snapshot(window: &WebviewWindow) -> WindowVisibility {
    WindowVisibility {
        visible: is_visible(window),
        focused: window.is_focused().unwrap_or(false),
    }
}

pub(crate) fn publish(window: &WebviewWindow) {
    let current = window.clone();
    let _ = window.run_on_main_thread(move || {
        let _ = current.emit("main-window-visibility", snapshot(&current));
    });
}

#[tauri::command]
pub fn get_main_window_visibility(window: WebviewWindow) -> WindowVisibility {
    snapshot(&window)
}

#[tauri::command]
pub fn hide_main_window(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|error| error.to_string())?;
    publish(&window);
    Ok(())
}
