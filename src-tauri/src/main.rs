// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    webview::{PageLoadEvent, Webview, WebviewBuilder},
    LogicalPosition, LogicalRect, LogicalSize, Manager, Position, Rect, Runtime, Size, State,
    WebviewUrl, Window,
};

use std::sync::Mutex;

const CONTENT_WEBVIEW_LABEL: &str = "instance-content";
const DEFAULT_SIDEBAR_WIDTH: f64 = 280.0;

#[derive(Clone, Default)]
struct LoginCredentials {
    username: String,
    password: String,
}

#[derive(Default)]
struct AppState {
    login_credentials: Mutex<Option<LoginCredentials>>,
}

fn content_bounds(width: f64, height: f64, sidebar_width: f64) -> LogicalRect<f64, f64> {
    LogicalRect {
        position: LogicalPosition {
            x: sidebar_width,
            y: 0.0,
        },
        size: LogicalSize {
            width: (width - sidebar_width).max(0.0),
            height,
        },
    }
}

fn content_rect(width: f64, height: f64, sidebar_width: f64) -> Rect {
    let bounds = content_bounds(width, height, sidebar_width);

    Rect {
        position: Position::Logical(bounds.position),
        size: Size::Logical(bounds.size),
    }
}

fn content_webview<R: Runtime>(window: &Window<R>) -> Option<Webview<R>> {
    window.app_handle().get_webview(CONTENT_WEBVIEW_LABEL)
}

fn is_login_path(path: &str) -> bool {
    let normalized = path.trim_end_matches('/');
    normalized == "/login"
}

fn set_login_credentials(
    state: &State<'_, AppState>,
    username: Option<String>,
    password: Option<String>,
) -> Result<(), String> {
    let credentials = match (username, password) {
        (Some(username), Some(password)) if !username.is_empty() || !password.is_empty() => {
            Some(LoginCredentials { username, password })
        }
        (Some(username), None) if !username.is_empty() => Some(LoginCredentials {
            username,
            password: String::new(),
        }),
        (None, Some(password)) if !password.is_empty() => Some(LoginCredentials {
            username: String::new(),
            password,
        }),
        _ => None,
    };

    *state
        .login_credentials
        .lock()
        .map_err(|error| error.to_string())? = credentials;

    Ok(())
}

fn login_autofill_script(credentials: &LoginCredentials) -> Result<String, String> {
    let username =
        serde_json::to_string(&credentials.username).map_err(|error| error.to_string())?;
    let password =
        serde_json::to_string(&credentials.password).map_err(|error| error.to_string())?;

    Ok(format!(
        r#"
(() => {{
    const username = {username};
    const password = {password};
    const loginPath = window.location.pathname.replace(/\/+$/, "") || "/";
    if (loginPath !== "/login") return;

    const setNativeValue = (input, value) => {{
        if (!input || value === "") return false;
        const prototype = Object.getPrototypeOf(input);
        const descriptor = Object.getOwnPropertyDescriptor(prototype, "value");
        if (descriptor?.set) {{
            descriptor.set.call(input, value);
        }} else {{
            input.value = value;
        }}
        input.dispatchEvent(new Event("input", {{ bubbles: true }}));
        input.dispatchEvent(new Event("change", {{ bubbles: true }}));
        return true;
    }};

    const visibleInput = (input) => {{
        if (!(input instanceof HTMLInputElement)) return false;
        if (input.disabled || input.readOnly) return false;
        if (input.type === "hidden" || input.type === "submit" || input.type === "button") return false;
        return input.offsetParent !== null || input.getClientRects().length > 0;
    }};

    const textScore = (input) => {{
        const text = [
            input.autocomplete,
            input.name,
            input.id,
            input.placeholder,
            input.getAttribute("aria-label"),
        ].join(" ").toLowerCase();
        if (text.includes("user") || text.includes("login") || text.includes("email") || text.includes("mail")) return 3;
        if (input.type === "email") return 2;
        if (input.type === "text" || input.type === "") return 1;
        return 0;
    }};

    const fill = () => {{
        const inputs = Array.from(document.querySelectorAll("input")).filter(visibleInput);
        const passwordInput = inputs.find((input) => input.type === "password");
        const usernameInput = inputs
            .filter((input) => input !== passwordInput)
            .map((input) => [input, textScore(input)])
            .filter(([, score]) => score > 0)
            .sort((a, b) => b[1] - a[1])[0]?.[0];

        const didUsername = setNativeValue(usernameInput, username);
        const didPassword = setNativeValue(passwordInput, password);
        return didUsername || didPassword;
    }};

    let attempts = 0;
    const timer = window.setInterval(() => {{
        attempts += 1;
        if (fill() || attempts >= 20) {{
            window.clearInterval(timer);
        }}
    }}, 250);
    fill();
}})();
"#
    ))
}

fn autofill_login_if_needed<R: Runtime>(webview: &Webview<R>, state: &State<'_, AppState>) {
    let credentials = match state.login_credentials.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };

    if let Some(credentials) = credentials {
        if let Ok(script) = login_autofill_script(&credentials) {
            let _ = webview.eval(script);
        }
    }
}

#[tauri::command]
fn set_webview_bounds(
    window: Window,
    sidebar_width: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if let Some(webview) = content_webview(&window) {
        webview
            .set_bounds(content_rect(width, height, sidebar_width))
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[tauri::command]
async fn open_instance_url(
    window: Window,
    state: State<'_, AppState>,
    url: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<(), String> {
    let parsed_url = url
        .parse()
        .map_err(|_| "Could not open URL. Use an http:// or https:// URL.".to_string())?;

    set_login_credentials(&state, username, password)?;

    if let Some(webview) = content_webview(&window) {
        return webview
            .navigate(parsed_url)
            .map_err(|error| error.to_string());
    }

    let size = window.inner_size().map_err(|error| error.to_string())?;
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let logical_size = size.to_logical::<f64>(scale_factor);

    let bounds = content_bounds(
        logical_size.width,
        logical_size.height,
        DEFAULT_SIDEBAR_WIDTH,
    );

    let app_handle = window.app_handle().clone();
    window
        .add_child(
            WebviewBuilder::new(CONTENT_WEBVIEW_LABEL, WebviewUrl::External(parsed_url))
                .on_page_load(move |webview, payload| {
                    if payload.event() == PageLoadEvent::Finished
                        && is_login_path(payload.url().path())
                    {
                        let state = app_handle.state::<AppState>();
                        autofill_login_if_needed(&webview, &state);
                    }
                }),
            bounds.position,
            bounds.size,
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_instance_url,
            set_webview_bounds
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
