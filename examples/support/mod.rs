//! Platform helpers shared by the native and WebGPU demos.

// Each example uses a different subset.
#![allow(dead_code)]

use bevy::prelude::*;

/// Browser examples attach to the page's existing canvas instead of creating another one.
pub fn window(title: &str) -> Window {
    status(title);
    Window {
        title: title.into(),
        #[cfg(target_arch = "wasm32")]
        canvas: Some("#bevy".into()),
        #[cfg(target_arch = "wasm32")]
        fit_canvas_to_parent: true,
        ..default()
    }
}

/// The command line; in a browser, the page's query string, where `?debug-view=lighting&gizmos`
/// stands for `--debug-view lighting --gizmos`.
pub fn args() -> std::vec::IntoIter<String> {
    #[cfg(not(target_arch = "wasm32"))]
    let args: Vec<String> = std::env::args().collect();
    #[cfg(target_arch = "wasm32")]
    let args = query_args();
    args.into_iter()
}

#[cfg(target_arch = "wasm32")]
fn query_args() -> Vec<String> {
    let search = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let mut args = Vec::new();
    for pair in search.trim_start_matches('?').split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name.is_empty() {
            continue;
        }
        args.push(format!("--{name}"));
        if !value.is_empty() {
            args.push(value.to_owned());
        }
    }
    args
}

pub fn argument(name: &str) -> Option<String> {
    let mut args = args().skip_while(|arg| arg != name);
    args.next()?;
    args.next()
}

pub fn flag(name: &str) -> bool {
    args().any(|arg| arg == name)
}

pub fn status(message: &str) {
    page_text("demo-status", message);
}

pub fn page_text(id: &str, message: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    {
        element.set_text_content(Some(message));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, message);
}
