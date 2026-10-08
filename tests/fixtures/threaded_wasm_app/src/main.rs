//! A WebAssembly app that uses only public crates: it draws nothing, but mounts a canvas so a
//! browser test can tell it started. `rayx app` includes this file in the entry crate it
//! generates, so the exports below are the ones the host page and the thread pool look for.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
export function mount_canvas() {
  const canvas = document.createElement('canvas');
  canvas.width = 320;
  canvas.height = 200;
  document.body.appendChild(canvas);
}
")]
extern "C" {
    fn mount_canvas();
}

/// Called by the host page after the module is initialized.
#[wasm_bindgen]
pub fn run() {
    mount_canvas();
}

/// The entry point a worker thread runs; `rayx app` refuses a build without it.
#[wasm_bindgen]
pub fn wasm_thread_entry_point(_ptr: u32) {}

fn main() {
    println!("{}", threaded_wasm_app::NAME);
}
