//! Slint voice-lab harness. Single-engine mode (pick STT/TTS engine, Record/Test)
//! plus two Compare modes: record once → every STT engine transcribes → mark which
//! are correct; and synth one line through every TTS engine → latency + replay.
//! Native rendering (no WebView), so it runs on old Android.

slint::include_modules!();

mod llm;
#[cfg(target_os = "android")]
#[allow(dead_code)]
mod native_stt;
mod opus;
#[cfg(target_os = "ios")]
mod ios_audio;
#[cfg(target_os = "ios")]
mod ios_location;
#[cfg(target_os = "ios")]
mod ios_url;
#[cfg(target_os = "ios")]
mod ios_vpio;
#[cfg(target_os = "ios")]
mod ios_map;
// Talk (realtime WebRTC) runs on Android + iOS. Lab (sherpa STT/TTS) is Android-only.
#[cfg(any(target_os = "android", target_os = "ios"))]
mod realtime;
mod soniox;
#[cfg(target_os = "android")]
mod stt;
mod tts; // kept cross-platform: Talk needs tts::Resampler + tts::files_dir()
use slint::{Model, ModelRc, VecModel};
use std::rc::Rc;
#[cfg(target_os = "android")]
use stt::{Stt, SttResult, STT_MOONSHINE_EN};
#[cfg(target_os = "android")]
use tts::{
    OpusResult, RoundTripResult, Tts, TtsCompareResult, ENGINE_PIPER_ID, ENGINE_SONIOX_ID,
    SAMPLE_EN, SAMPLE_ID,
};

// ==== VOID_AI view surface: drive the VS global from Nova's UI-control messages ====

fn icon_for(id: &str) -> &'static str {
    match id {
        "chat" => "💬", "images" => "🖼", "web" => "🌐",
        "map" => "📍", "products" => "🛍", "docs" => "📄", _ => "•",
    }
}
fn label_for(id: &str) -> &'static str {
    match id {
        "chat" => "Chat", "images" => "Images", "web" => "Web",
        "map" => "Map", "products" => "Shopping", "docs" => "Documents", _ => "View",
    }
}
fn tab_for(id: &str) -> ViewTab {
    ViewTab { id: id.into(), label: label_for(id).into(), icon: icon_for(id).into() }
}
/// Per-result placeholder tint (real thumbnails come later).
fn tint(i: usize) -> slint::Color {
    const HUES: [(u8, u8, u8); 6] =
        [(232, 50, 28), (255, 111, 66), (60, 66, 92), (120, 96, 112), (86, 120, 150), (96, 112, 96)];
    let (r, g, b) = HUES[i % 6];
    slint::Color::from_rgb_u8(r, g, b)
}

/// Ensure `id` is in VS.open-views (a Rust-owned VecModel — set up in run_app).
fn ensure_open(ui: &MainWindow, id: &str) {
    let model = ui.global::<VS>().get_open_views();
    if let Some(vm) = model.as_any().downcast_ref::<VecModel<ViewTab>>() {
        if (0..vm.row_count()).all(|i| vm.row_data(i).map(|t| t.id != id).unwrap_or(true)) {
            vm.push(tab_for(id));
        }
    }
}
fn close_view(ui: &MainWindow, id: &str) {
    if id == "chat" {
        return;
    }
    let vs = ui.global::<VS>();
    let model = vs.get_open_views();
    if let Some(vm) = model.as_any().downcast_ref::<VecModel<ViewTab>>() {
        if let Some(idx) =
            (0..vm.row_count()).find(|&i| vm.row_data(i).map(|t| t.id == id).unwrap_or(false))
        {
            vm.remove(idx);
        }
    }
    if vs.get_current_view() == id {
        vs.set_current_view("chat".into());
    }
}

/// Parse a `blocks` array into typed Block rows. `kinds` are the accepted per-block
/// keys in priority order; the kind label is the key. "rule" is a bool marker.
fn json_blocks(v: &serde_json::Value, key: &str, kinds: &[&str]) -> Vec<Block> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|b| {
                    for k in kinds {
                        if *k == "rule" {
                            if b.get("rule").and_then(|x| x.as_bool()).unwrap_or(false) {
                                return Some(Block { kind: "rule".into(), text: "———".into() });
                            }
                        } else if let Some(t) = b.get(*k).and_then(|x| x.as_str()) {
                            return Some(Block { kind: (*k).into(), text: t.into() });
                        }
                    }
                    None
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Which model slot an async-fetched image belongs to.
#[derive(Clone, Copy)]
enum ImgSlot {
    Images(usize),
    Products(usize),
    Map,
}

/// Fetch + decode an image URL into a Slint-ready RGBA buffer (blocking; run off the UI thread).
fn fetch_pixels(url: &str) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("VOID_AI/0.1")
        .build()
        .ok()?;
    let bytes = client.get(url).send().ok()?.error_for_status().ok()?.bytes().ok()?;
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    buf.make_mut_bytes().copy_from_slice(rgba.as_raw());
    Some(buf)
}

/// Kick off a background image fetch; when it lands, drop it into its model slot on the UI
/// thread. Views show the placeholder tint until each image streams in (progressive load).
fn spawn_image_fetch(ui: &MainWindow, url: String, slot: ImgSlot) {
    if url.is_empty() {
        return;
    }
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let Some(buf) = fetch_pixels(&url) else { return };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let img = slint::Image::from_rgba8(buf);
            let vs = ui.global::<VS>();
            match slot {
                ImgSlot::Map => vs.set_map_image(img),
                ImgSlot::Images(i) => {
                    if let Some(vm) = vs.get_images().as_any().downcast_ref::<VecModel<ImageItem>>() {
                        if let Some(mut row) = vm.row_data(i) {
                            row.pic = img;
                            vm.set_row_data(i, row);
                        }
                    }
                }
                ImgSlot::Products(i) => {
                    if let Some(vm) = vs.get_products().as_any().downcast_ref::<VecModel<Product>>() {
                        if let Some(mut row) = vm.row_data(i) {
                            row.pic = img;
                            vm.set_row_data(i, row);
                        }
                    }
                }
            }
        });
    });
}

/// Apply one UI-control message (the `data` object of a server-message). UI thread.
// Browser tabs: each search_web result becomes a tab. Kept in Rust (a nested list of blocks
// per tab doesn't fit a flat Slint model); VS.web-* mirrors the ACTIVE tab. UI-thread only.
struct WebTab {
    url: String,
    title: String,
    blocks: Vec<Block>,
}
thread_local! {
    static WEB_TABS: std::cell::RefCell<Vec<WebTab>> = const { std::cell::RefCell::new(Vec::new()) };
    static WEB_ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Push VS.web-* (count, titles, active index, active blocks/url) from the tab list.
fn sync_web(ui: &MainWindow) {
    let vs = ui.global::<VS>();
    WEB_TABS.with(|tabs| {
        let tabs = tabs.borrow();
        vs.set_web_tabs(tabs.len() as i32);
        let titles: Vec<slint::SharedString> = tabs.iter().map(|t| t.title.as_str().into()).collect();
        vs.set_web_tab_titles(ModelRc::from(Rc::new(VecModel::from(titles))));
        let active = if tabs.is_empty() { 0 } else { WEB_ACTIVE.with(|a| a.get()).min(tabs.len() - 1) };
        WEB_ACTIVE.with(|a| a.set(active));
        vs.set_web_active_tab(active as i32);
        if let Some(t) = tabs.get(active) {
            vs.set_web_blocks(ModelRc::from(Rc::new(VecModel::from(t.blocks.clone()))));
            vs.set_web_url(t.url.as_str().into());
            vs.set_web_title(t.title.as_str().into());
        } else {
            vs.set_web_blocks(ModelRc::from(Rc::new(VecModel::<Block>::default())));
            vs.set_web_url("".into());
            vs.set_web_title("".into());
        }
    });
}

fn apply_ui_control(ui: &MainWindow, data: &str) {
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return,
    };
    let op = v.get("op").and_then(|o| o.as_str()).unwrap_or("");
    let vs = ui.global::<VS>();
    let sget = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let bget = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
    // Content ops open + switch to their view, and honor an optional `collapse`.
    let switch = |ui: &MainWindow, view: &str, collapse_default: bool| {
        ensure_open(ui, view);
        ui.global::<VS>().set_current_view(view.into());
        if bget("collapse", collapse_default) {
            ui.global::<VS>().set_collapsed(true);
        }
    };
    match op {
        "open_view" => {
            let view = sget("view");
            ensure_open(ui, &view);
            vs.set_current_view(view.into());
        }
        "focus_view" => vs.set_current_view(sget("view").into()),
        "close_view" => close_view(ui, &sget("view")),
        "collapse" => vs.set_collapsed(bget("on", true)),
        "fullscreen" => vs.set_fullscreen(bget("on", true)),
        "images" => {
            let items: Vec<ImageItem> = v
                .get("items")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .enumerate()
                        .map(|(i, it)| ImageItem {
                            cap: it.get("cap").and_then(|x| x.as_str()).unwrap_or("").into(),
                            url: it.get("full").and_then(|x| x.as_str()).unwrap_or("").into(),
                            tint: tint(i),
                            pic: Default::default(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let urls: Vec<String> = items.iter().map(|it| it.url.to_string()).collect();
            vs.set_images(ModelRc::from(Rc::new(VecModel::from(items))));
            vs.set_images_query(sget("query").into());
            switch(ui, "images", true);
            for (i, u) in urls.into_iter().enumerate() {
                spawn_image_fetch(ui, u, ImgSlot::Images(i));
            }
        }
        "map" => {
            let pins: Vec<MapPin> = v
                .get("pins")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .enumerate()
                        .map(|(i, p)| {
                            let rating = p
                                .get("rating")
                                .and_then(|x| x.as_f64())
                                .map(|r| format!("{r:.1}★"))
                                .unwrap_or_default();
                            // real normalized position from the tool if present, else a fan-out grid
                            let x = p.get("x").and_then(|v| v.as_f64()).map(|v| v as f32)
                                .unwrap_or(0.26 + 0.22 * ((i % 3) as f32) + 0.05 * ((i / 3) as f32))
                                .clamp(0.06, 0.94);
                            let y = p.get("y").and_then(|v| v.as_f64()).map(|v| v as f32)
                                .unwrap_or(0.28 + 0.24 * ((i / 3) as f32) + 0.06 * ((i % 3) as f32))
                                .clamp(0.06, 0.94);
                            MapPin {
                                name: p.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
                                note: p.get("note").and_then(|x| x.as_str()).unwrap_or("").into(),
                                dist: p.get("dist").and_then(|x| x.as_str()).unwrap_or("").into(),
                                rating: rating.into(),
                                x,
                                y,
                                lat: p.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                                lng: p.get("lng").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            vs.set_pins(ModelRc::from(Rc::new(VecModel::from(pins))));
            vs.set_map_query(sget("query").into());
            // directions carry a `route`: show the ETA + an "Open in Maps" deep link. A plain
            // places search has no route, so clear both (hides the nav bar).
            if let Some(r) = v.get("route").filter(|r| !r.is_null()) {
                let mins = r.get("eta_min").and_then(|x| x.as_i64()).unwrap_or(0);
                let miles = r.get("miles").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let mode = r.get("mode").and_then(|x| x.as_str()).unwrap_or("driving");
                let dlat = r.get("dest_lat").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let dlng = r.get("dest_lng").and_then(|x| x.as_f64()).unwrap_or(0.0);
                vs.set_map_eta(format!("{mins} min · {miles} mi").into());
                vs.set_map_nav_url(
                    format!("https://www.google.com/maps/dir/?api=1&destination={dlat},{dlng}&travelmode={mode}")
                        .into(),
                );
            } else {
                vs.set_map_eta("".into());
                vs.set_map_nav_url("".into());
            }
            switch(ui, "map", true);
            spawn_image_fetch(ui, sget("img"), ImgSlot::Map);
            // iOS: drive the live Mapbox webview (ios_map) with the same pins — markers + a
            // destination pin for directions; the page fits/flies to them. (The static tile
            // above still serves other platforms + shows until the live map paints.)
            #[cfg(target_os = "ios")]
            {
                let markers: Vec<serde_json::Value> = v
                    .get("pins")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| {
                                let lat = p.get("lat").and_then(|x| x.as_f64())?;
                                let lng = p.get("lng").and_then(|x| x.as_f64())?;
                                if lat == 0.0 && lng == 0.0 {
                                    return None;
                                }
                                Some(serde_json::json!({
                                    "lng": lng, "lat": lat,
                                    "label": p.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                                }))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut data = serde_json::json!({ "markers": markers });
                // Directions: hand the page dest + origin (from the op, else the last GPS fix) +
                // mode so it draws the route polyline via the Mapbox Directions API — or a
                // precomputed `geometry` (GeoJSON LineString) if the bot ever sends one.
                if let Some(r) = v.get("route").filter(|r| !r.is_null()) {
                    let dlat = r.get("dest_lat").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    let dlng = r.get("dest_lng").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    if dlat != 0.0 || dlng != 0.0 {
                        let mode = r.get("mode").and_then(|x| x.as_str()).unwrap_or("driving");
                        let mut route = serde_json::json!({ "dest": [dlng, dlat], "mode": mode });
                        let origin = match (
                            r.get("orig_lat").and_then(|x| x.as_f64()),
                            r.get("orig_lng").and_then(|x| x.as_f64()),
                        ) {
                            (Some(a), Some(o)) if a != 0.0 || o != 0.0 => Some((a, o)),
                            _ => ios_location::last_location(),
                        };
                        if let Some((olat, olng)) = origin {
                            route["origin"] = serde_json::json!([olng, olat]);
                        }
                        if let Some(g) = r.get("geometry").filter(|g| !g.is_null()) {
                            route["geometry"] = g.clone();
                        }
                        data["route"] = route;
                    }
                }
                ios_map::set_data(&data.to_string());
            }
        }
        "products" => {
            let items: Vec<Product> = v
                .get("items")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .enumerate()
                        .map(|(i, p)| Product {
                            name: p.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
                            price: p.get("price").and_then(|x| x.as_str()).unwrap_or("").into(),
                            store: p.get("store").and_then(|x| x.as_str()).unwrap_or("").into(),
                            rating: p.get("rating").and_then(|x| x.as_str()).unwrap_or("—").into(),
                            ships: p.get("ships").and_then(|x| x.as_str()).unwrap_or("").into(),
                            link: p.get("link").and_then(|x| x.as_str()).unwrap_or("").into(),
                            tint: tint(i),
                            pic: Default::default(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let thumbs: Vec<String> = v
                .get("items")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|p| p.get("thumb").and_then(|x| x.as_str()).unwrap_or("").to_string()).collect())
                .unwrap_or_default();
            vs.set_products(ModelRc::from(Rc::new(VecModel::from(items))));
            vs.set_products_query(sget("query").into());
            switch(ui, "products", true);
            for (i, u) in thumbs.into_iter().enumerate() {
                spawn_image_fetch(ui, u, ImgSlot::Products(i));
            }
        }
        "web" => {
            // Each result opens a new tab; VS.web-* mirrors the (now active) one.
            let blocks = json_blocks(&v, "blocks", &["h", "p"]);
            let url = sget("url");
            let title = {
                let t = sget("title");
                if t.is_empty() { url.clone() } else { t }
            };
            WEB_TABS.with(|tabs| {
                let mut tabs = tabs.borrow_mut();
                tabs.push(WebTab { url, title, blocks });
                WEB_ACTIVE.with(|a| a.set(tabs.len() - 1));
            });
            sync_web(ui);
            switch(ui, "web", false);
        }
        "doc" => {
            vs.set_doc_blocks(ModelRc::from(Rc::new(VecModel::from(json_blocks(
                &v,
                "blocks",
                &["h2", "p", "rule"],
            )))));
            vs.set_doc_name(sget("name").into());
            vs.set_doc_page(sget("page").into());
            switch(ui, "docs", false);
        }
        // plan-then-handoff: open the maps deep link in the phone's nav app
        "navigate" => {
            let url = sget("url");
            if !url.is_empty() {
                #[cfg(target_os = "android")]
                if let Err(e) = crate::native_stt::launch_url(&url) {
                    eprintln!("[navigate] launch failed: {e}");
                }
                #[cfg(not(target_os = "android"))]
                eprintln!("[navigate] would open nav app: {url}");
            }
        }
        _ => {}
    }
}

/// Parse the rendered transcript ("🗣  You\n…\n\n🤖  Nova\n…") into styled chat turns.
fn set_chat_turns(ui: &MainWindow, rendered: &str) {
    let turns: Vec<ChatTurn> = rendered
        .split("\n\n")
        .filter(|c| !c.trim().is_empty())
        .map(|chunk| {
            let mut it = chunk.splitn(2, '\n');
            let head = it.next().unwrap_or("");
            let body = it.next().unwrap_or("").trim().to_string();
            let is_user = head.contains("You");
            ChatTurn {
                who: if is_user { "YOU" } else { "NOVA" }.into(),
                text: body.into(),
                is_user,
            }
        })
        .collect();
    ui.global::<VS>()
        .set_chat_turns(ModelRc::from(Rc::new(VecModel::from(turns))));
}

pub fn run_app() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let ui_weak = ui.as_weak();

    // Empty list-models for the two compare views (results stream into these).
    ui.set_stt_rows(ModelRc::from(Rc::new(VecModel::<SttRow>::default())));
    ui.set_tts_rows(ModelRc::from(Rc::new(VecModel::<TtsRow>::default())));

    // ---- VOID_AI view surface: Rust-owned open-views model + add/close wiring ----
    // Replace the Slint-literal model with a VecModel we can mutate (chat is permanent).
    ui.global::<VS>()
        .set_open_views(ModelRc::from(Rc::new(VecModel::from(vec![tab_for("chat")]))));
    {
        let aw = ui_weak.clone();
        ui.global::<VS>().on_add_view(move |id: slint::SharedString| {
            if let Some(ui) = aw.upgrade() {
                ensure_open(&ui, id.as_str());
                ui.global::<VS>().set_current_view(id);
            }
        });
        let cw = ui_weak.clone();
        ui.global::<VS>().on_close_view(move |id: slint::SharedString| {
            if let Some(ui) = cw.upgrade() {
                close_view(&ui, id.as_str());
            }
        });
    }

    // ---- browser tabs: switch / close / open-in-phone-browser ----
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_switch_web_tab(move |i: i32| {
            if let Some(ui) = w.upgrade() {
                WEB_ACTIVE.with(|a| a.set(i.max(0) as usize));
                sync_web(&ui);
            }
        });
    }
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_close_web_tab(move |i: i32| {
            if let Some(ui) = w.upgrade() {
                let i = i.max(0) as usize;
                WEB_TABS.with(|tabs| {
                    let mut tabs = tabs.borrow_mut();
                    if i < tabs.len() {
                        tabs.remove(i);
                    }
                    let active = WEB_ACTIVE.with(|a| a.get());
                    if active >= i && active > 0 {
                        WEB_ACTIVE.with(|a| a.set(active - 1));
                    }
                });
                sync_web(&ui);
            }
        });
    }
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_open_web_external(move || {
            if let Some(ui) = w.upgrade() {
                let url = ui.global::<VS>().get_web_url().to_string();
                if !url.is_empty() {
                    #[cfg(target_os = "android")]
                    if let Err(e) = crate::native_stt::launch_url(&url) {
                        eprintln!("[web] open failed: {e}");
                    }
                    #[cfg(target_os = "ios")]
                    crate::ios_url::launch_url(&url);
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    eprintln!("[web] would open: {url}");
                }
            }
        });
    }

    // Desktop-only visual check: VOIDAI_DEMO=1 seeds sample content so the view surface
    // is populated without a live bot. Harmless on device (env unset).
    // Seed sample content for a visual check: desktop via VOIDAI_DEMO=1, or any Android
    // debug build (env vars don't reach the app there). TODO: gate off before Phase 3.
    if std::env::var("VOIDAI_DEMO").is_ok()
        || cfg!(all(any(target_os = "android", target_os = "ios"), debug_assertions))
    {
        set_chat_turns(&ui, "🗣  You\nShow me some cool brutalist buildings in Seattle.\n\n🤖  Nova\nUgh, fine. Seattle's got a few concrete beasts — the kind of raw, unpainted slabs that look like they were poured by someone with a serious grudge against joy. Freeway Park, the old Public Safety Building before they mercifully demolished it, a couple of parking garages that somehow got called architecture. Try not to nod off on me while I dig them up, this is thrilling work.\n\n🗣  You\nWhich one is the best?\n\n🤖  Nova\nFreeway Park, obviously. It's a whole park stacked on top of a highway, which is either genius or a cry for help.");
        apply_ui_control(&ui, r#"{"op":"images","query":"brutalist buildings · Seattle","items":[{"cap":"freeway_park.jpg","full":"https://picsum.photos/seed/voidbru1/500/500"},{"cap":"rainier_square.jpg","full":"https://picsum.photos/seed/voidbru2/500/500"},{"cap":"kingdome_1976.jpg","full":"https://picsum.photos/seed/voidbru3/500/500"},{"cap":"seattle_muni.jpg","full":"https://picsum.photos/seed/voidbru4/500/500"}]}"#);
        apply_ui_control(&ui, r#"{"op":"products","query":"concrete planter","items":[{"name":"Raw Concrete Planter, 6\"","price":"$38","store":"WEST ELM","rating":"4.6","ships":"Free ship","link":"https://www.google.com/search?tbm=shop&q=concrete+planter","thumb":"https://picsum.photos/seed/voidp1/300/300"},{"name":"Béton Cylinder Pot","price":"$44","store":"AMAZON","rating":"4.4","ships":"Prime","link":"https://www.google.com/search?tbm=shop&q=concrete+planter","thumb":"https://picsum.photos/seed/voidp2/300/300"},{"name":"Faceted Concrete Planter","price":"$29","store":"WAYFAIR","rating":"4.2","ships":"2-day","link":"https://www.google.com/search?tbm=shop&q=concrete+planter","thumb":"https://picsum.photos/seed/voidp3/300/300"},{"name":"Modern Cement Pot Set","price":"$52","store":"TARGET","rating":"4.7","ships":"Free ship","link":"https://www.google.com/search?tbm=shop&q=concrete+planter","thumb":"https://picsum.photos/seed/voidp4/300/300"},{"name":"Terrazzo Mini Planter","price":"$34","store":"CB2","rating":"4.5","ships":"In store","link":"https://www.google.com/search?tbm=shop&q=concrete+planter","thumb":"https://picsum.photos/seed/voidp5/300/300"},{"name":"Ribbed Concrete Bowl","price":"$41","store":"ETSY","rating":"4.9","ships":"Handmade","link":"https://www.google.com/search?tbm=shop&q=concrete+planter","thumb":"https://picsum.photos/seed/voidp6/300/300"}]}"#);
        apply_ui_control(&ui, r#"{"op":"web","url":"https://en.wikipedia.org/wiki/Brutalist_architecture","title":"Brutalism","blocks":[{"h":"Brutalist architecture"},{"p":"Brutalist architecture is a style that emerged in the 1950s, growing out of the early-20th-century modernist movement. Brutalist buildings are marked by minimalist construction that puts the bare structure and materials on show rather than any decorative design."},{"p":"The style favours exposed, unpainted concrete or brick, blunt geometric forms, and a mostly monochrome palette; steel, timber and glass often appear alongside the concrete."},{"h":"Origin of the term"},{"p":"The name comes from the French beton brut — \"raw concrete\" — a phrase Le Corbusier used for the board-marked surfaces of his post-war work. British critics Alison and Peter Smithson, and later Reyner Banham, popularised the label through the 1950s."},{"p":"Brutalism became a favourite for institutional buildings — universities, libraries, courthouses and city halls — through the 1960s and 70s, prized for its honesty, low cost and monumental presence."},{"p":"By the 1980s it had fallen from favour, its weather-stained concrete tied in the public mind to urban decay. Appreciation has revived since the 2010s, and several landmarks now carry heritage protection."}]}"#);
        apply_ui_control(&ui, r#"{"op":"map","query":"coffee near Capitol Hill, Seattle","img":"https://staticmap.openstreetmap.de/staticmap.php?center=47.6205,-122.3212&zoom=13&size=600x520&maptype=mapnik","pins":[{"name":"Victrola Coffee Roasters","note":"Roastery · Wi-Fi","dist":"0.3 mi","rating":4.6,"lat":47.6229,"lng":-122.3212},{"name":"Analog Coffee","note":"Minimalist","dist":"0.5 mi","rating":4.5,"lat":47.6150,"lng":-122.3211},{"name":"Espresso Vivace","note":"Latte-art OG","dist":"0.7 mi","rating":4.7,"lat":47.6180,"lng":-122.3215}]}"#);
        apply_ui_control(&ui, r#"{"op":"doc","name":"Brutalism_A_Reader.pdf","page":"1 / 12","blocks":[{"h2":"Brutalism: A Reader"},{"p":"The term derives from the French beton brut — raw concrete — the phrase Le Corbusier used for the board-marked surfaces of his post-war buildings."},{"rule":true},{"p":"Reyner Banham's 1955 essay \"The New Brutalism\" gave the movement its critical vocabulary, framing it as an ethic as much as an aesthetic — an insistence on showing a building's structure and services honestly."},{"p":"Through the 1960s the approach spread from Britain across Europe, North America and beyond, shaping campuses, housing estates and civic centres on almost every continent."},{"p":"By the late 1970s the mood had soured; raw concrete weathered badly in wet climates and the style became shorthand for austere, unloved public architecture. Today conservationists argue its best examples deserve protection as bold expressions of their era."}]}"#);
        apply_ui_control(&ui, r#"{"op":"open_view","view":"chat"}"#);
        apply_ui_control(&ui, r#"{"op":"collapse","on":false}"#);
    }

    // ==== LAB (engine bench) — Android-only. The sherpa STT/TTS engines aren't ported
    // to iOS yet (Phase 2); on iOS the Lab tab renders but its callbacks stay unwired. ====
    #[cfg(target_os = "android")]
    {
    // ---- TTS controller (built first so STT's round-trip handler can call it) ----
    let tts = {
        let w = ui_weak.clone();
        let report = move |msg: String| {
            let _ = w.upgrade_in_event_loop(move |ui| ui.set_status(msg.into()));
        };
        let cw = ui_weak.clone();
        let on_compare = move |r: TtsCompareResult| {
            let meta = format!("1st {:.2}s · total {:.2}s · {:.1}s audio", r.first, r.total, r.dur);
            let engine = r.engine.clone();
            let _ = cw.upgrade_in_event_loop(move |ui| {
                if let Some(vm) = ui.get_tts_rows().as_any().downcast_ref::<VecModel<TtsRow>>() {
                    vm.push(TtsRow { engine: engine.into(), meta: meta.into() });
                }
            });
        };
        // round-trip: TTS reports LLM + first-audio latency and the LLM reply;
        // combine with the STT time we stashed on the UI for the full loop cost.
        let rw = ui_weak.clone();
        let on_roundtrip = move |r: RoundTripResult| {
            let _ = rw.upgrade_in_event_loop(move |ui| {
                let stt = ui.get_rt_stt_secs();
                ui.set_rt_reply(r.reply.into());
                let line = if r.streamed {
                    // Streaming: tts_first = POST → first sentence's audio (LLM+TTS
                    // pipelined), so time-to-first-audio = STT + tts_first.
                    let total = stt + r.tts_first;
                    let wire = if r.audio_over_wire {
                        format!("{:.1} KB Opus", r.wire_bytes as f32 / 1024.0)
                    } else {
                        format!("{} B text", r.wire_bytes)
                    };
                    format!(
                        "STT {stt:.2}s + stream {:.2}s  =  {total:.2}s to 1st audio\nLLM 1st sentence {:.2}s · {wire} over the wire",
                        r.tts_first, r.llm
                    )
                } else {
                    let total = stt + r.llm + r.tts_first;
                    format!(
                        "STT {stt:.2}s + LLM {:.2}s + TTS 1st {:.2}s  =  {total:.2}s to first reply",
                        r.llm, r.tts_first
                    )
                };
                ui.set_rt_result(line.into());
            });
        };
        // Opus streaming test result → format into the Opus-mode card.
        let ow = ui_weak.clone();
        let on_opus = move |r: OpusResult| {
            let line = if !r.err.is_empty() {
                format!("error: {}", r.err)
            } else if r.ok {
                format!(
                    "SMOOTH ✓\n1st audio {:.2}s · {:.1}s played · {:.0} kbps · no rebuffer",
                    r.first_audio, r.audio_secs, r.recv_kbps
                )
            } else {
                format!(
                    "STALLED\n1st audio {:.2}s · rebuffered {:.1}s · {} late packets · {:.0} kbps",
                    r.first_audio, r.late_ms / 1000.0, r.late_packets, r.recv_kbps
                )
            };
            let _ = ow.upgrade_in_event_loop(move |ui| {
                ui.set_opus_running(false);
                ui.set_opus_result(line.into());
            });
        };
        // Reply text streamed sentence-by-sentence → show it as it arrives,
        // rather than only when the whole round-trip finishes.
        let rpw = ui_weak.clone();
        let on_reply = move |reply: String| {
            let _ = rpw.upgrade_in_event_loop(move |ui| ui.set_rt_reply(reply.into()));
        };
        Rc::new(Tts::new(
            Box::new(report),
            Box::new(on_compare),
            Box::new(on_roundtrip),
            Box::new(on_opus),
            Box::new(on_reply),
        ))
    };

    // ---- STT controller ----
    let stt = {
        let sw = ui_weak.clone();
        let status = move |msg: String| {
            let _ = sw.upgrade_in_event_loop(move |ui| ui.set_status(msg.into()));
        };
        // single mode → transcript text; round-trip mode → kick off TTS reply
        let tw = ui_weak.clone();
        let on_text = move |r: SttResult| {
            let rtf = r.decode_secs / r.audio_secs.max(0.001);
            let line = format!(
                "{}\n({} · {:.1}s audio · {:.1}s decode · RTF {:.2})",
                if r.text.is_empty() { "(nothing recognized)" } else { &r.text },
                r.engine, r.audio_secs, r.decode_secs, rtf
            );
            let heard = if r.text.is_empty() { "(nothing recognized)".to_string() } else { r.text.clone() };
            let decode = r.decode_secs;
            let err = r.err.clone();
            let _ = tw.upgrade_in_event_loop(move |ui| {
                // Native STT self-completes (no Stop tap), so make sure the record
                // buttons reset here regardless of mode or outcome.
                ui.set_recording(false);
                ui.set_rt_recording(false);
                if !err.is_empty() {
                    let msg = format!("{} error: {err}", "STT");
                    if ui.get_mode() == 3 {
                        ui.set_rt_result(msg.into());
                    } else {
                        ui.set_transcript(msg.into());
                    }
                    ui.set_status("".into());
                    return;
                }
                if ui.get_mode() == 3 {
                    ui.set_rt_heard(heard.clone().into());
                    ui.set_rt_reply("".into());
                    ui.set_rt_stt_secs(decode);
                    ui.set_rt_result(format!("STT {decode:.2}s · asking the LLM…").into());
                    let eng = ui.get_engine();
                    // delivery 0 = on-device TTS; 1 = server Opus via fly (cap just
                    // signals the server path — the real network is the latency).
                    let cap = if ui.get_rt_delivery_idx() == 1 { 1 } else { 0 };
                    let bitrate = ui.get_opus_bitrate();
                    ui.invoke_rt_speak(heard.into(), eng, cap, bitrate);
                } else {
                    ui.set_transcript(line.into());
                }
            });
        };
        // compare mode → append a row per engine
        let cw = ui_weak.clone();
        let on_compare = move |r: SttResult| {
            let rtf = r.decode_secs / r.audio_secs.max(0.001);
            let meta = format!("{:.1}s decode · RTF {:.2}", r.decode_secs, rtf);
            let engine = r.engine.to_string();
            let text = r.text.clone();
            let _ = cw.upgrade_in_event_loop(move |ui| {
                if let Some(vm) = ui.get_stt_rows().as_any().downcast_ref::<VecModel<SttRow>>() {
                    vm.push(SttRow {
                        engine: engine.into(),
                        text: text.into(),
                        meta: meta.into(),
                        correct: false,
                    });
                }
            });
        };
        Rc::new(Stt::new(Box::new(status), Box::new(on_text), Box::new(on_compare)))
    };

    // ---- single-mode STT record ----
    ui.on_record_toggle({
        let w = ui_weak.clone();
        let stt = stt.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let now = !ui.get_recording();
                ui.set_recording(now);
                let eng = ui.get_stt_engine() as u8;
                if now { stt.start(eng); } else { stt.stop(eng); }
            }
        }
    });

    // ---- single-mode TTS test ----
    ui.on_test({
        let w = ui_weak.clone();
        let tts = tts.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let engine = ui.get_engine() as u8;
                let text = if engine == ENGINE_PIPER_ID || engine == ENGINE_SONIOX_ID {
                    SAMPLE_ID
                } else {
                    SAMPLE_EN
                };
                ui.set_status("synthesizing…".into());
                tts.speak(text.to_string(), ui.get_sid(), ui.get_steps(), ui.get_threads(), engine);
            }
        }
    });

    // ---- STT compare: record → run all engines ----
    ui.on_compare_record({
        let w = ui_weak.clone();
        let stt = stt.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let now = !ui.get_comparing();
                ui.set_comparing(now);
                if now {
                    if let Some(vm) = ui.get_stt_rows().as_any().downcast_ref::<VecModel<SttRow>>() {
                        vm.set_vec(vec![]);
                    }
                    ui.set_status("● listening… (tap Stop)".into());
                    stt.start(STT_MOONSHINE_EN); // compare captures the cpal buffer for all engines
                } else {
                    ui.set_status("running all engines…".into());
                    stt.compare_stop();
                }
            }
        }
    });

    // ---- STT compare: mark a row correct/incorrect ----
    ui.on_mark_correct({
        let w = ui_weak.clone();
        move |i| {
            if let Some(ui) = w.upgrade() {
                if let Some(vm) = ui.get_stt_rows().as_any().downcast_ref::<VecModel<SttRow>>() {
                    if let Some(mut row) = vm.row_data(i as usize) {
                        row.correct = !row.correct;
                        vm.set_row_data(i as usize, row);
                    }
                }
            }
        }
    });

    // ---- TTS compare: synth the same line through all engines ----
    ui.on_compare_synth({
        let w = ui_weak.clone();
        let tts = tts.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                if let Some(vm) = ui.get_tts_rows().as_any().downcast_ref::<VecModel<TtsRow>>() {
                    vm.set_vec(vec![]);
                }
                ui.set_status("synthesizing all…".into());
                tts.compare_all(SAMPLE_ID.to_string());
            }
        }
    });

    // ---- TTS compare: replay a clip ----
    ui.on_play_clip({
        let tts = tts.clone();
        move |i| tts.play_stored(i as usize)
    });

    // ---- Round-trip: record → transcribe (selected STT) → speak (selected TTS) ----
    ui.on_roundtrip_toggle({
        let w = ui_weak.clone();
        let stt = stt.clone();
        let tts = tts.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let now = !ui.get_rt_recording();
                ui.set_rt_recording(now);
                if now {
                    ui.set_rt_heard("".into());
                    ui.set_rt_result("".into());
                    ui.set_status("● listening… (tap Stop)".into());
                    // Prewarm the on-device voice model NOW so its cold load
                    // overlaps with STT (only for on-device delivery — server
                    // Opus synthesizes on fly, not here).
                    if ui.get_rt_delivery_idx() == 0 {
                        tts.warmup(ui.get_engine() as u8);
                    }
                    stt.start(ui.get_stt_engine() as u8);
                } else {
                    ui.set_status("transcribing…".into());
                    stt.stop(ui.get_stt_engine() as u8);
                }
            }
        }
    });

    // Fired on the UI thread once the transcript is ready → drive the TTS reply
    // (cap 0 = on-device TTS; >0 = server Opus stream at that simulated 3G cap).
    ui.on_rt_speak({
        let tts = tts.clone();
        move |text, engine, cap, bitrate| {
            tts.roundtrip_speak(text.to_string(), engine as u8, cap as u32, bitrate as u32)
        }
    });

    // ---- Opus streaming test: stream Opus from the server at a simulated 3G cap ----
    ui.on_opus_stream({
        let tts = tts.clone();
        let w = ui_weak.clone();
        move |cap_kbps, bitrate| {
            if let Some(ui) = w.upgrade() {
                if ui.get_opus_running() {
                    return; // a stream is already in flight — ignore re-taps
                }
                ui.set_opus_running(true);
                ui.set_opus_result("connecting…".into());
            }
            // Host: /sdcard/kaira/opus_server.txt (LAN IP or emulator loopback).
            let host = std::fs::read_to_string(format!("{}/opus_server.txt", tts::files_dir()))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "10.0.2.2:8770".to_string());
            tts.opus_stream(host, cap_kbps as u32, bitrate as u32);
        }
    });
    } // ==== end Android-only Lab section ====

    // ---- Realtime agent (Pipecat WebRTC) — Talk tab (Android + iOS) ----
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let realtime = {
        let w = ui_weak.clone();
        let status: realtime::StatusCb = std::sync::Arc::new(move |msg: String| {
            let _ = w.upgrade_in_event_loop(move |ui| ui.set_rt_status(msg.into()));
        });
        let wl = ui_weak.clone();
        let level: realtime::LevelCb = std::sync::Arc::new(move |peak: f32| {
            let _ = wl.upgrade_in_event_loop(move |ui| ui.set_rt_mic_level(peak.clamp(0.0, 1.0)));
        });
        // Nova's (inbound) live level → the visualizer, so it can tell who's speaking.
        let wal = ui_weak.clone();
        let agent_level: realtime::LevelCb = std::sync::Arc::new(move |peak: f32| {
            let _ = wal.upgrade_in_event_loop(move |ui| ui.set_rt_agent_level(peak.clamp(0.0, 1.0)));
        });
        let wt = ui_weak.clone();
        let transcript: realtime::TranscriptCb = std::sync::Arc::new(move |text: String| {
            let _ = wt.upgrade_in_event_loop(move |ui| {
                set_chat_turns(&ui, &text);
                ui.set_rt_transcript(text.into());
            });
        });
        // Nova's UI-control messages (see UI_CONTRACT.md) → drive the Slint view surface.
        let wu = ui_weak.clone();
        let ui_control: realtime::UiControlCb = std::sync::Arc::new(move |data: String| {
            let wu = wu.clone();
            let _ = wu.upgrade_in_event_loop(move |ui| apply_ui_control(&ui, &data));
        });
        // Truthful connection state (not tap-optimistic): `rt-live` follows the real peer
        // state, and a terminal failure resets `rt-connected` so a re-tap retries.
        let wcs = ui_weak.clone();
        let conn_state: realtime::StateCb = std::sync::Arc::new(move |s: realtime::ConnState| {
            use realtime::ConnState::*;
            let _ = wcs.upgrade_in_event_loop(move |ui| match s {
                Connecting | Reconnecting => {
                    ui.set_rt_live(false);
                    ui.set_rt_failed(false);
                }
                Live => {
                    ui.set_rt_live(true);
                    ui.set_rt_failed(false);
                }
                Ended => {
                    ui.set_rt_live(false);
                    ui.set_rt_failed(false);
                }
                Failed => {
                    // Gave up reconnecting: reset intent so the button/tap becomes a retry,
                    // and keep the failure visible.
                    ui.set_rt_live(false);
                    ui.set_rt_connected(false);
                    ui.set_rt_failed(true);
                }
            });
        });
        Rc::new(realtime::Realtime::new(
            status, conn_state, level, agent_level, transcript, ui_control,
        ))
    };
    ui.on_realtime_toggle({
        let w = ui_weak.clone();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let now = !ui.get_rt_connected();
                ui.set_rt_connected(now);
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    if now {
                        // iOS: cpal can't capture until the AVAudioSession is active.
                        #[cfg(target_os = "ios")]
                        ios_audio::activate();
                        // iOS: start GPS (main thread) so we can report the user's location
                        // to the bot on connect — same purpose as Android's last_location().
                        #[cfg(target_os = "ios")]
                        ios_location::start();
                        // Default bot URL per platform (overridable via realtime_url.txt
                        // in the app's files dir). iOS → the DigitalOcean Droplet: a real
                        // public IP with open UDP, so aiortc advertises a directly-reachable
                        // host candidate — NO TURN, works on any network incl. cellular.
                        // Android emulator → host loopback for fast local dev.
                        #[cfg(target_os = "ios")]
                        const DEFAULT_BOT: &str = "http://143.198.134.89:8080/api/offer";
                        #[cfg(not(target_os = "ios"))]
                        const DEFAULT_BOT: &str = "http://10.0.2.2:7860/api/offer";
                        let url = std::fs::read_to_string(format!(
                            "{}/realtime_url.txt",
                            tts::files_dir()
                        ))
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| DEFAULT_BOT.to_string());
                        ui.set_rt_failed(false);
                        ui.set_rt_status("connecting…".into());
                        realtime.connect(url);
                    } else {
                        realtime.disconnect();
                    }
                }
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                ui.set_rt_status("Realtime needs Android or iOS.".into());
            }
        }
    });

    // Mic mute → tell the realtime client to send silence instead of the mic.
    ui.on_mic_mute({
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move |muted| {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            realtime.set_muted(muted);
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let _ = muted;
        }
    });

    // Tap the waveform → cut Nova off (instant local silence + tell the bot to stop).
    ui.on_interrupt_nova({
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            realtime.interrupt();
        }
    });

    // Persist a setting (rail position, theme, …) to the app's files dir; restored on boot.
    ui.on_save_setting(move |key, val| {
        let path = format!("{}/setting_{}.txt", tts::files_dir(), key);
        if let Err(e) = std::fs::write(&path, val.as_str()) {
            eprintln!("[settings] save '{key}' failed: {e}");
        }
    });

    // Open a URL (product store page, map location, …) in the phone's default handler.
    ui.on_open_url(move |url| {
        let url = url.to_string();
        if !url.is_empty() {
            #[cfg(target_os = "android")]
            if let Err(e) = crate::native_stt::launch_url(&url) {
                eprintln!("[open-url] launch failed: {e}");
            }
            #[cfg(target_os = "ios")]
            crate::ios_url::launch_url(&url);
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            eprintln!("[open-url] would open: {url}");
        }
    });

    // ---- audio visualizer: a rolling signed-level history the strip renders as a
    // scrolling waveform. + = Nova speaking, − = the user speaking, 0 = idle. A Slint
    // Timer calls wave_tick ~16×/s. ----
    let wave: Rc<VecModel<f32>> = Rc::new(VecModel::from(vec![0.0f32; 52]));
    ui.set_wave(ModelRc::from(wave.clone()));
    ui.on_wave_tick({
        let w = ui_weak.clone();
        let wave = wave.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let agent = ui.get_rt_agent_level();
                let mic = ui.get_rt_mic_level();
                // Nova wins ties: her inbound level is the reliable "she's speaking" signal
                // (the mic also picks up her echo, which we must NOT attribute to the user).
                let v = if agent > 0.03 {
                    agent.min(1.0)
                } else if mic > 0.03 {
                    -(mic.min(1.0))
                } else {
                    0.0
                };
                if wave.row_count() > 0 {
                    wave.remove(0);
                }
                wave.push(v);
            }
        }
    });

    // Restore persisted settings (rail position, theme) before the first frame.
    {
        let dir = tts::files_dir();
        if let Ok(v) = std::fs::read_to_string(format!("{}/setting_rail_pos.txt", dir)) {
            let v = v.trim();
            if matches!(v, "left" | "right" | "top" | "bottom") {
                ui.global::<VS>().set_rail_pos(v.into());
            }
        }
        if let Ok(v) = std::fs::read_to_string(format!("{}/setting_theme.txt", dir)) {
            match v.trim() {
                "dark" => ui.global::<VS>().set_dark(true),
                "light" => ui.global::<VS>().set_dark(false),
                _ => {}
            }
        }
    }

    // iOS: live interactive Mapbox map behind the "map" view. Slint reports the map region's
    // absolute geometry (report-map-geom) so we position a WKWebView there; map-active-changed
    // hides it when the user leaves the map view.
    #[cfg(target_os = "ios")]
    {
        let w = ui_weak.clone();
        ui.on_report_map_geom(move |x, y, wd, h| {
            if let Some(ui) = w.upgrade() {
                ios_map::show_at(ui.window(), x, y, wd, h);
            }
        });
        ui.on_map_active_changed(|active| {
            if !active {
                ios_map::hide();
            }
        });
    }

    ui.run()
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).unwrap();
    if let Err(e) = run_app() {
        eprintln!("[kaira-slint] {e}");
    }
}
