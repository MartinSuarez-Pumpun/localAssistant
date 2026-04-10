mod chat;
mod config_panel;
mod file_browser;
mod plugins;
mod shell;

use leptos::prelude::*;
use shell::Shell;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! { <Shell/> }
}
