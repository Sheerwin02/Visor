//! Loading files from the asset bundle (assets/). On the web every file is fetched as raw bytes (textures are built in code,
//! working around bevy#26244); natively the files are read from disk (VISOR_ASSETS, default ../assets).
use bevy::prelude::*;
use std::sync::{Arc, Mutex};

/// Finished requests: (file name, bytes or error text).
#[derive(Resource, Clone, Default)]
pub struct Inbox(pub Arc<Mutex<Vec<(String, Result<Vec<u8>, String>)>>>);

impl Inbox {
    pub fn request(&self, name: &str) {
        let inbox = self.0.clone();
        let name = name.to_string();
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let r = fetch_bytes(&format!("assets/{name}")).await;
            inbox.lock().unwrap().push((name, r));
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            let root = std::env::var("VISOR_ASSETS").unwrap_or_else(|_| "../assets".to_string());
            let r = std::fs::read(format!("{root}/{name}")).map_err(|e| e.to_string());
            inbox.lock().unwrap().push((name, r));
        }
    }

    pub fn take(&self) -> Vec<(String, Result<Vec<u8>, String>)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let win = web_sys::window().ok_or("no window")?;
    let resp = JsFuture::from(win.fetch_with_str(url)).await.map_err(|e| format!("{e:?}"))?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "not a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {} for {url}", resp.status()));
    }
    let buf = JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?).await.map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}
