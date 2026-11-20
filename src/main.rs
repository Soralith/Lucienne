//! Kaimono local UI server. It renders the converted Django templates and
//! prints the URL it is listening on; the Electron shell (or any browser)
//! opens that URL. No windowing or webview code lives here.

mod filters;
mod library_cache;
mod server;
mod steam;

fn main() {
    let address = server::start().expect("local server failed to start");

    // The Electron shell reads this line from stdout to find the UI.
    println!("http://{address}/dashboard/");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
