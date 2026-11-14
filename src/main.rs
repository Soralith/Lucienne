//! Kaimono desktop shell: a Tauri window (system WebKitGTK, no Chromium)
//! pointed at a local axum server that renders the converted Django templates.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod filters;
mod server;

/// WebKitGTK's Wayland compositing path paints the scrolled content area blank
/// and leaves the cursor frozen after scrolling on some drivers (seen on
/// Hyprland with hybrid Intel/NVIDIA graphics). The X11/XWayland path renders
/// the same page correctly. Pick X11 when both backends are available so the
/// app works out of the box; an explicit GDK_BACKEND still wins.
#[cfg(target_os = "linux")]
fn prefer_x11_on_wayland() {
    if std::env::var_os("GDK_BACKEND").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_some()
    {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    prefer_x11_on_wayland();

    let address = server::start().expect("local server failed to start");
    let url = format!("http://{address}/dashboard/");

    // Headless mode serves the UI without opening a window (smoke tests, curl).
    if std::env::var_os("LUCIENNE_HEADLESS").is_some() {
        println!("{url}");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    tauri::Builder::default()
        .setup(move |app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse().expect("valid url")),
            )
            .title("Kaimono")
            .inner_size(1440.0, 900.0)
            .min_inner_size(1024.0, 680.0)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Kaimono");
}
