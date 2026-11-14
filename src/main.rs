//! Kaimono desktop shell: a Tauri window (system WebKitGTK, no Chromium)
//! pointed at a local axum server that renders the converted Django templates.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod filters;
mod server;

fn main() {
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
