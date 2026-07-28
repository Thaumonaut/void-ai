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
#[cfg(target_os = "ios")]
mod ios_photos;
#[cfg(target_os = "ios")]
mod ios_web;
#[cfg(target_os = "ios")]
mod ios_video;
#[cfg(target_os = "ios")]
mod ios_haptics;
// Talk (realtime WebRTC) runs on Android + iOS. Lab (sherpa STT/TTS) is Android-only.
#[cfg(any(target_os = "android", target_os = "ios"))]
mod realtime;
mod permissions; // cross-platform mic/location permission façade (iOS/Android/desktop)
mod soniox;
#[cfg(target_os = "android")]
mod stt;
mod tts; // kept cross-platform: Talk needs tts::Resampler + tts::files_dir()
use slint::{Model, ModelRc, VecModel};

/// Prompt for microphone (required) + location (optional) permission, then reflect the
/// outcome into `VS.mic-perm` / `VS.loc-perm`. Shared by the onboarding CTA and the
/// connect gate. The mic result arrives asynchronously (iOS block / Android poll), so it
/// hops back onto the Slint event loop before touching the UI.
fn request_permissions(w: &slint::Weak<MainWindow>) {
    // Location prompt is fire-and-forget; refresh its status best-effort (never blocks Talk).
    permissions::request_location();
    if let Some(ui) = w.upgrade() {
        ui.global::<VS>().set_loc_perm(permissions::location_status().as_i32());
    }
    let w = w.clone();
    permissions::request_mic(move |p| {
        let _ = w.upgrade_in_event_loop(move |ui| {
            ui.global::<VS>().set_mic_perm(p.as_i32());
            match p {
                permissions::Perm::Granted => {
                    if !ui.get_rt_connected() {
                        ui.set_rt_status("Microphone ready — tap to talk".into());
                    }
                }
                permissions::Perm::Denied => {
                    ui.set_rt_status("Microphone blocked — enable it in Settings".into());
                }
                permissions::Perm::Undetermined => {}
            }
        });
    });
}
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
        "map" => "📍", "products" => "🛍", "docs" => "📄", "videos" => "🎬", "weather" => "🌤", _ => "•",
    }
}
fn label_for(id: &str) -> &'static str {
    match id {
        "chat" => "Chat", "images" => "Images", "web" => "Web",
        "map" => "Map", "products" => "Shopping", "docs" => "Documents", "videos" => "Videos",
        "weather" => "Weather", _ => "View",
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

/// Decode a picked photo's encoded bytes, add it to the Images view (at the front) as an
/// "Imported photo", and open it in the lightbox. No URL — it's local pixels.
#[cfg(target_os = "ios")]
fn import_picked_image(ui: &MainWindow, encoded: &[u8]) {
    let Ok(dyn_img) = image::load_from_memory(encoded) else {
        eprintln!("[photos] decode failed ({} bytes)", encoded.len());
        return;
    };
    let rgba = dyn_img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    buf.make_mut_bytes().copy_from_slice(rgba.as_raw());
    let item = ImageItem {
        cap: "Imported photo".into(),
        url: "".into(),
        tint: tint(0),
        pic: slint::Image::from_rgba8(buf),
        failed: false,
    };
    let vs = ui.global::<VS>();
    if let Some(vm) = vs.get_images().as_any().downcast_ref::<VecModel<ImageItem>>() {
        vm.insert(0, item);
    } else {
        vs.set_images(ModelRc::from(Rc::new(VecModel::from(vec![item]))));
    }
    bump_images_gen();
    vs.set_images_query("Imported".into());
    ensure_open(ui, "images");
    vs.set_current_view("images".into());
    vs.set_lightbox_index(0);
    vs.set_lightbox_open(true);
}

/// Re-encode a picked photo as a small JPEG (≤768px) and base64 it, to hand to the bot over
/// the data channel so Nova (duplex/Gemini) can see the imported image.
#[cfg(target_os = "ios")]
fn encode_jpeg_b64(encoded: &[u8]) -> Option<String> {
    use base64::Engine;
    let img = image::load_from_memory(encoded).ok()?;
    let small = img.resize(768, 768, image::imageops::FilterType::Triangle);
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(small.to_rgb8())
        .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(&jpeg))
}

/// Which model slot an async-fetched image belongs to.
#[derive(Clone, Copy)]
enum ImgSlot {
    Images(usize),
    Products(usize),
    Videos(usize),
    Map,
}

thread_local! {
    // Bumped whenever the images/products model is replaced, so a slow async fetch that
    // lands after a NEW search can't write its image into the new model's row (audit #9 —
    // stale-async overwrite). Fetch captures the gen at spawn; the landing drops if it moved.
    static IMAGES_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static PRODUCTS_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static VIDEOS_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    // Last static-map tile URL, so tap-to-retry can re-fetch it (map-unavailable affordance).
    static LAST_MAP_URL: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn bump_images_gen() {
    IMAGES_GEN.with(|g| g.set(g.get().wrapping_add(1)));
}
fn bump_products_gen() {
    PRODUCTS_GEN.with(|g| g.set(g.get().wrapping_add(1)));
}
fn bump_videos_gen() {
    VIDEOS_GEN.with(|g| g.set(g.get().wrapping_add(1)));
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
    // Generation of the model this fetch targets, captured at spawn.
    let gen = match slot {
        ImgSlot::Images(_) => IMAGES_GEN.with(|g| g.get()),
        ImgSlot::Products(_) => PRODUCTS_GEN.with(|g| g.get()),
        ImgSlot::Videos(_) => VIDEOS_GEN.with(|g| g.get()),
        ImgSlot::Map => 0,
    };
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        // On failure, mark the row `failed` (broken-image state) instead of silently
        // returning — otherwise the placeholder tint persists forever and looks loaded.
        let result = fetch_pixels(&url);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            // Drop a stale result — the model was replaced since this fetch started (#9).
            let stale = match slot {
                ImgSlot::Images(_) => IMAGES_GEN.with(|g| g.get()) != gen,
                ImgSlot::Products(_) => PRODUCTS_GEN.with(|g| g.get()) != gen,
                ImgSlot::Videos(_) => VIDEOS_GEN.with(|g| g.get()) != gen,
                ImgSlot::Map => false,
            };
            if stale {
                return;
            }
            let vs = ui.global::<VS>();
            let ok = result.is_some();
            let img = result.map(slint::Image::from_rgba8);
            match slot {
                ImgSlot::Map => {
                    // The static tile is the map on desktop/Android; on iOS the live webview
                    // covers it, so its load state (not this fetch) drives map-failed there.
                    #[cfg(not(target_os = "ios"))]
                    vs.set_map_failed(!ok);
                    if let Some(img) = img {
                        vs.set_map_image(img);
                    }
                }
                ImgSlot::Images(i) => {
                    if let Some(vm) = vs.get_images().as_any().downcast_ref::<VecModel<ImageItem>>() {
                        if let Some(mut row) = vm.row_data(i) {
                            if let Some(img) = img {
                                row.pic = img;
                            }
                            row.failed = !ok;
                            vm.set_row_data(i, row);
                        }
                    }
                }
                ImgSlot::Products(i) => {
                    if let Some(vm) = vs.get_products().as_any().downcast_ref::<VecModel<Product>>() {
                        if let Some(mut row) = vm.row_data(i) {
                            if let Some(img) = img {
                                row.pic = img;
                            }
                            row.failed = !ok;
                            vm.set_row_data(i, row);
                        }
                    }
                }
                ImgSlot::Videos(i) => {
                    if let Some(vm) = vs.get_videos().as_any().downcast_ref::<VecModel<VideoItem>>() {
                        if let Some(mut row) = vm.row_data(i) {
                            if let Some(img) = img {
                                row.pic = img;
                            }
                            row.failed = !ok;
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
    id: u64, // stable across tab open/close so chat cards can find their tab after eviction
    url: String,
    title: String,
    blocks: Vec<Block>,
}
thread_local! {
    static WEB_TABS: std::cell::RefCell<Vec<WebTab>> = const { std::cell::RefCell::new(Vec::new()) };
    static WEB_ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static WEB_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One image search kept in history (metadata; thumbnails are re-fetched when the tab is shown).
struct ImageBatch {
    id: u64,
    query: String,
    items: Vec<(String, String)>, // (caption, url)
}
const IMAGE_TAB_LIMIT: usize = 12; // cap the in-session history

thread_local! {
    static IMAGES_TABS: std::cell::RefCell<Vec<ImageBatch>> = const { std::cell::RefCell::new(Vec::new()) };
    static IMAGES_ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static IMG_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One video result (a YouTube video Nova surfaced). `vid` is the YouTube id (for the embed
/// player); `url` is the watch URL (for "open in YouTube"); `thumb` is the preview image URL.
#[derive(Clone)]
struct VideoData {
    title: String,
    channel: String,
    dur: String,
    url: String,
    vid: String,
    thumb: String,
}
/// One video search kept in history (mirrors ImageBatch).
struct VideoBatch {
    id: u64,
    query: String,
    items: Vec<VideoData>,
}
const VIDEO_TAB_LIMIT: usize = 12;

thread_local! {
    static VIDEO_TABS: std::cell::RefCell<Vec<VideoBatch>> = const { std::cell::RefCell::new(Vec::new()) };
    static VIDEOS_ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static VID_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

// iOS: while a video is playing, periodically snapshot the frame and push it to Nova (vision).
#[cfg(target_os = "ios")]
thread_local! {
    static VIDEO_VISION_TIMER: std::cell::RefCell<Option<slint::Timer>> = const { std::cell::RefCell::new(None) };
}

/// The chat timeline interleaves spoken turns (from the transcript) with tappable result
/// CARDS Nova drops when a tool lands (an image search / a web page / a video search). Each
/// card is anchored after the Nth spoken turn present when it was created, and references its
/// target by a stable `id` (resolved to the current tab index at render time, so it still
/// points right after the history is capped/evicted).
struct ChatCard {
    anchor: usize,      // number of spoken turns present when the card was dropped
    kind: &'static str, // "images" | "web" | "videos"
    id: u64,            // stable id of the target batch/tab
    title: String,
    sub: String,
}
thread_local! {
    static CHAT_CARDS: std::cell::RefCell<Vec<ChatCard>> = const { std::cell::RefCell::new(Vec::new()) };
    static LAST_TRANSCRIPT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static CHAT_TURNS_N: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn next_seq(cell: &'static std::thread::LocalKey<std::cell::Cell<u64>>) -> u64 {
    cell.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        v
    })
}

/// Resolve a chat card's stable id to its CURRENT index in that kind's history, or None if it
/// has been evicted/closed (the card is then dropped from the timeline).
fn resolve_card_tab(kind: &str, id: u64) -> Option<usize> {
    match kind {
        "images" => IMAGES_TABS.with(|t| t.borrow().iter().position(|b| b.id == id)),
        "web" => WEB_TABS.with(|t| t.borrow().iter().position(|b| b.id == id)),
        "videos" => VIDEO_TABS.with(|t| t.borrow().iter().position(|b| b.id == id)),
        _ => None,
    }
}

/// Drop a result card into the chat timeline, anchored at the current spoken-turn count, and
/// re-render the chat so it shows immediately (before the next transcript update).
fn push_chat_card(ui: &MainWindow, kind: &'static str, id: u64, title: String, sub: String) {
    let anchor = CHAT_TURNS_N.with(|n| n.get());
    CHAT_CARDS.with(|c| c.borrow_mut().push(ChatCard { anchor, kind, id, title, sub }));
    render_chat(ui);
}

// ---- cross-session view history (Stage 4) ----
// The image/web search history is persisted to <files_dir>/view_history.json so past searches
// survive an app restart (mobile only — desktop is a dev loop and shouldn't accrue state). The
// chat transcript itself is NOT persisted; only the searches (they repopulate the tab strips).
#[cfg(any(target_os = "android", target_os = "ios"))]
mod persist {
    use super::*;

    const PERSIST_WEB_LIMIT: usize = 16; // web tabs are uncapped in-session; cap what we store

    #[derive(serde::Serialize, serde::Deserialize)]
    struct PBlock {
        kind: String,
        text: String,
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PBatch {
        query: String,
        items: Vec<(String, String)>,
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PWeb {
        url: String,
        title: String,
        blocks: Vec<PBlock>,
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PVid {
        title: String,
        channel: String,
        dur: String,
        url: String,
        vid: String,
        thumb: String,
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PVidBatch {
        query: String,
        items: Vec<PVid>,
    }
    #[derive(serde::Serialize, serde::Deserialize, Default)]
    struct PHistory {
        #[serde(default)]
        images: Vec<PBatch>,
        #[serde(default)]
        web: Vec<PWeb>,
        #[serde(default)]
        videos: Vec<PVidBatch>,
    }

    fn history_path() -> String {
        format!("{}/view_history.json", crate::tts::files_dir())
    }

    /// Snapshot the in-session image/web history to disk (capped). Cheap small-JSON write.
    pub(crate) fn save() {
        let images: Vec<PBatch> = IMAGES_TABS.with(|t| {
            t.borrow()
                .iter()
                .map(|b| PBatch { query: b.query.clone(), items: b.items.clone() })
                .collect()
        });
        let mut web: Vec<PWeb> = WEB_TABS.with(|t| {
            t.borrow()
                .iter()
                .map(|w| PWeb {
                    url: w.url.clone(),
                    title: w.title.clone(),
                    blocks: w
                        .blocks
                        .iter()
                        .map(|b| PBlock { kind: b.kind.to_string(), text: b.text.to_string() })
                        .collect(),
                })
                .collect()
        });
        if web.len() > PERSIST_WEB_LIMIT {
            web.drain(..web.len() - PERSIST_WEB_LIMIT);
        }
        let videos: Vec<PVidBatch> = VIDEO_TABS.with(|t| {
            t.borrow()
                .iter()
                .map(|b| PVidBatch {
                    query: b.query.clone(),
                    items: b
                        .items
                        .iter()
                        .map(|d| PVid {
                            title: d.title.clone(),
                            channel: d.channel.clone(),
                            dur: d.dur.clone(),
                            url: d.url.clone(),
                            vid: d.vid.clone(),
                            thumb: d.thumb.clone(),
                        })
                        .collect(),
                })
                .collect()
        });
        let h = PHistory { images, web, videos };
        if let Ok(json) = serde_json::to_string(&h) {
            let _ = std::fs::write(history_path(), json);
        }
    }

    /// Restore the persisted history into the tab strips on launch (fresh ids). Opens the
    /// Images/Web views in the nav so the restored history is reachable. Does not switch views.
    pub(crate) fn load(ui: &MainWindow) {
        let Ok(json) = std::fs::read_to_string(history_path()) else { return };
        let h: PHistory = match serde_json::from_str(&json) {
            Ok(h) => h,
            Err(_) => return,
        };
        if !h.images.is_empty() {
            IMAGES_TABS.with(|t| {
                let mut t = t.borrow_mut();
                for b in h.images {
                    t.push(ImageBatch { id: next_seq(&IMG_SEQ), query: b.query, items: b.items });
                }
                IMAGES_ACTIVE.with(|a| a.set(t.len().saturating_sub(1)));
            });
            ensure_open(ui, "images");
            sync_images(ui);
        }
        if !h.web.is_empty() {
            WEB_TABS.with(|t| {
                let mut t = t.borrow_mut();
                for w in h.web {
                    let blocks = w
                        .blocks
                        .into_iter()
                        .map(|b| Block { kind: b.kind.into(), text: b.text.into() })
                        .collect();
                    t.push(WebTab { id: next_seq(&WEB_SEQ), url: w.url, title: w.title, blocks });
                }
                WEB_ACTIVE.with(|a| a.set(t.len().saturating_sub(1)));
            });
            ensure_open(ui, "web");
            sync_web(ui);
        }
        if !h.videos.is_empty() {
            VIDEO_TABS.with(|t| {
                let mut t = t.borrow_mut();
                for b in h.videos {
                    let items = b
                        .items
                        .into_iter()
                        .map(|d| VideoData {
                            title: d.title,
                            channel: d.channel,
                            dur: d.dur,
                            url: d.url,
                            vid: d.vid,
                            thumb: d.thumb,
                        })
                        .collect();
                    t.push(VideoBatch { id: next_seq(&VID_SEQ), query: b.query, items });
                }
                VIDEOS_ACTIVE.with(|a| a.set(t.len().saturating_sub(1)));
            });
            ensure_open(ui, "videos");
            sync_videos(ui);
        }
    }
}

/// Persist the search history (no-op off-device).
fn save_history() {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    persist::save();
}

/// Push VS.images-* (count, titles, active index, active batch's grid) from the batch history,
/// and (re)start the thumbnail fetches for the active batch. Mirrors `sync_web`.
fn sync_images(ui: &MainWindow) {
    let vs = ui.global::<VS>();
    let (query, urls) = IMAGES_TABS.with(|tabs| {
        let tabs = tabs.borrow();
        vs.set_image_tabs(tabs.len() as i32);
        let titles: Vec<slint::SharedString> =
            tabs.iter().map(|t| t.query.as_str().into()).collect();
        vs.set_image_tab_titles(ModelRc::from(Rc::new(VecModel::from(titles))));
        let active = if tabs.is_empty() { 0 } else { IMAGES_ACTIVE.with(|a| a.get()).min(tabs.len() - 1) };
        IMAGES_ACTIVE.with(|a| a.set(active));
        vs.set_image_active_tab(active as i32);
        match tabs.get(active) {
            Some(b) => {
                let items: Vec<ImageItem> = b
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, (cap, url))| ImageItem {
                        cap: cap.as_str().into(),
                        url: url.as_str().into(),
                        tint: tint(i),
                        pic: Default::default(),
                        failed: false,
                    })
                    .collect();
                let urls: Vec<String> = b.items.iter().map(|(_, u)| u.clone()).collect();
                vs.set_images(ModelRc::from(Rc::new(VecModel::from(items))));
                (b.query.clone(), urls)
            }
            None => {
                vs.set_images(ModelRc::from(Rc::new(VecModel::<ImageItem>::default())));
                (String::new(), Vec::new())
            }
        }
    });
    bump_images_gen();
    vs.set_images_query(query.into());
    for (i, u) in urls.into_iter().enumerate() {
        spawn_image_fetch(ui, u, ImgSlot::Images(i));
    }
}

/// Push VS.video-* (count, titles, active index, active grid) from the batch history, and
/// (re)start the thumbnail fetches for the active batch. Mirrors `sync_images`.
/// The clean, always-public YouTube thumbnail for a video id — immune to the signed,
/// hotlink-protected (WebP-serving) URLs older bot builds / persisted history stored. Falls
/// back to the stored URL only if there's no usable id.
fn yt_thumb(vid: &str, stored: &str) -> String {
    let v: String = vid
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if v.is_empty() {
        stored.to_string()
    } else {
        format!("https://i.ytimg.com/vi/{v}/hqdefault.jpg")
    }
}

fn sync_videos(ui: &MainWindow) {
    let vs = ui.global::<VS>();
    let (query, thumbs) = VIDEO_TABS.with(|tabs| {
        let tabs = tabs.borrow();
        vs.set_video_tabs(tabs.len() as i32);
        let titles: Vec<slint::SharedString> =
            tabs.iter().map(|t| t.query.as_str().into()).collect();
        vs.set_video_tab_titles(ModelRc::from(Rc::new(VecModel::from(titles))));
        let active = if tabs.is_empty() { 0 } else { VIDEOS_ACTIVE.with(|a| a.get()).min(tabs.len() - 1) };
        VIDEOS_ACTIVE.with(|a| a.set(active));
        vs.set_video_active_tab(active as i32);
        match tabs.get(active) {
            Some(b) => {
                let items: Vec<VideoItem> = b
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, d)| VideoItem {
                        title: d.title.as_str().into(),
                        channel: d.channel.as_str().into(),
                        dur: d.dur.as_str().into(),
                        url: d.url.as_str().into(),
                        thumb: yt_thumb(&d.vid, &d.thumb).as_str().into(),
                        tint: tint(i),
                        pic: Default::default(),
                        failed: false,
                    })
                    .collect();
                let thumbs: Vec<String> = b.items.iter().map(|d| yt_thumb(&d.vid, &d.thumb)).collect();
                vs.set_videos(ModelRc::from(Rc::new(VecModel::from(items))));
                (b.query.clone(), thumbs)
            }
            None => {
                vs.set_videos(ModelRc::from(Rc::new(VecModel::<VideoItem>::default())));
                (String::new(), Vec::new())
            }
        }
    });
    bump_videos_gen();
    vs.set_videos_query(query.into());
    for (i, u) in thumbs.into_iter().enumerate() {
        spawn_image_fetch(ui, u, ImgSlot::Videos(i));
    }
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
    // iOS: point the live WKWebView reader at the active tab's real URL.
    #[cfg(target_os = "ios")]
    {
        let url = vs.get_web_url().to_string();
        ios_web::load_url(&url);
    }
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
    // Content landing clears any "Nova is working" pill for the view being shown.
    let switch = |ui: &MainWindow, view: &str, collapse_default: bool| {
        ensure_open(ui, view);
        ui.global::<VS>().set_current_view(view.into());
        ui.global::<VS>().set_working_view("".into());
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
        // Nova decided the conversation is over (user said goodbye) → arm the hangup; the UI drops
        // the session once her sign-off finishes playing (see hangup-grace in app.slint).
        "end_call" | "hangup" => vs.set_hangup_armed(true),
        "images" => {
            // History: each search APPENDS a tab instead of replacing, so you can flip back
            // through the session's searches (capped to IMAGE_TAB_LIMIT). sync_images then paints
            // the newly-active batch + kicks off its thumbnail fetches.
            let items: Vec<(String, String)> = v
                .get("items")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .map(|it| {
                            (
                                it.get("cap").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                it.get("full").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let query = sget("query");
            let count = items.len();
            let id = next_seq(&IMG_SEQ);
            IMAGES_TABS.with(|tabs| {
                let mut tabs = tabs.borrow_mut();
                tabs.push(ImageBatch { id, query: query.clone(), items });
                if tabs.len() > IMAGE_TAB_LIMIT {
                    let over = tabs.len() - IMAGE_TAB_LIMIT;
                    tabs.drain(..over);
                }
                IMAGES_ACTIVE.with(|a| a.set(tabs.len() - 1));
            });
            let sub = format!("{count} image{}", if count == 1 { "" } else { "s" });
            push_chat_card(ui, "images", id, query, sub);
            switch(ui, "images", true);
            sync_images(ui);
            save_history();
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
            let map_url = sget("img");
            LAST_MAP_URL.with(|u| *u.borrow_mut() = map_url.clone());
            vs.set_map_failed(false);
            spawn_image_fetch(ui, map_url, ImgSlot::Map);
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
                            failed: false,
                            thumb: p.get("thumb").and_then(|x| x.as_str()).unwrap_or("").into(),
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
            bump_products_gen();
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
            let id = next_seq(&WEB_SEQ);
            let card_title = title.clone();
            let card_sub = url.clone();
            WEB_TABS.with(|tabs| {
                let mut tabs = tabs.borrow_mut();
                tabs.push(WebTab { id, url, title, blocks });
                WEB_ACTIVE.with(|a| a.set(tabs.len() - 1));
            });
            push_chat_card(ui, "web", id, card_title, card_sub);
            sync_web(ui);
            switch(ui, "web", false);
            save_history();
        }
        "videos" => {
            // Video search history (mirrors `images`): each search APPENDS a tab. The results
            // are a grid; tapping one plays it in a WKWebView (YouTube embed).
            let items: Vec<VideoData> = v
                .get("items")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .map(|it| {
                            let g = |k: &str| {
                                it.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
                            };
                            VideoData {
                                title: g("title"),
                                channel: g("channel"),
                                dur: g("dur"),
                                url: g("url"),
                                vid: g("id"),
                                thumb: g("thumb"),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let query = sget("query");
            let count = items.len();
            let id = next_seq(&VID_SEQ);
            VIDEO_TABS.with(|tabs| {
                let mut tabs = tabs.borrow_mut();
                tabs.push(VideoBatch { id, query: query.clone(), items });
                if tabs.len() > VIDEO_TAB_LIMIT {
                    let over = tabs.len() - VIDEO_TAB_LIMIT;
                    tabs.drain(..over);
                }
                VIDEOS_ACTIVE.with(|a| a.set(tabs.len() - 1));
            });
            let sub = format!("{count} video{}", if count == 1 { "" } else { "s" });
            push_chat_card(ui, "videos", id, query, sub);
            vs.set_playing_video_id("".into()); // a new search shows the grid, not a player
            switch(ui, "videos", true);
            sync_videos(ui);
            save_history();
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
        "weather" => {
            let hours: Vec<WeatherHour> = v
                .get("hours")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .map(|it| {
                            let g = |k: &str| it.get(k).and_then(|x| x.as_str()).unwrap_or("");
                            WeatherHour { time: g("t").into(), temp: g("temp").into(), icon: g("icon").into() }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let days: Vec<WeatherDay> = v
                .get("days")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .map(|it| {
                            let g = |k: &str| it.get(k).and_then(|x| x.as_str()).unwrap_or("");
                            WeatherDay {
                                day: g("d").into(),
                                hi: g("hi").into(),
                                lo: g("lo").into(),
                                icon: g("icon").into(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            vs.set_weather_place(sget("place").into());
            vs.set_weather_temp(sget("temp").into());
            vs.set_weather_cond(sget("cond").into());
            vs.set_weather_icon(sget("icon").into());
            vs.set_weather_feels(sget("feels").into());
            vs.set_weather_hi(sget("hi").into());
            vs.set_weather_lo(sget("lo").into());
            vs.set_weather_humidity(sget("humidity").into());
            vs.set_weather_wind(sget("wind").into());
            vs.set_weather_day(bget("is_day", true));
            vs.set_weather_hours(ModelRc::from(Rc::new(VecModel::from(hours))));
            vs.set_weather_days(ModelRc::from(Rc::new(VecModel::from(days))));
            vs.set_weather_ok(true);
            switch(ui, "weather", true);
        }
        // Nova signals she's running a slow tool for `view`: show a "working" pill there until
        // the matching content op lands (which clears it via `switch`). Optional `label`.
        "working" => {
            let view = sget("view");
            if !view.is_empty() {
                ensure_open(ui, &view);
                vs.set_current_view(view.as_str().into());
                vs.set_working_label(sget("label").into());
                vs.set_working_view(view.as_str().into());
            }
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
fn parse_turns(rendered: &str) -> Vec<ChatTurn> {
    rendered
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
                kind: "".into(),
                tab: 0,
                sub: "".into(),
            }
        })
        .collect()
}

/// Store the latest transcript and re-render the interleaved chat timeline.
fn set_chat_turns(ui: &MainWindow, rendered: &str) {
    LAST_TRANSCRIPT.with(|t| *t.borrow_mut() = rendered.to_string());
    render_chat(ui);
}

/// Build the chat model by weaving the tool-result cards into the spoken turns: after emitting
/// the Nth spoken turn, emit every card anchored at N. Cards whose target was evicted are
/// skipped. Also refreshes each surviving card's `tab` to its current index.
fn render_chat(ui: &MainWindow) {
    let turns = LAST_TRANSCRIPT.with(|t| parse_turns(&t.borrow()));
    let n = turns.len();
    CHAT_TURNS_N.with(|c| c.set(n));

    let mut out: Vec<ChatTurn> = Vec::new();
    let mut emit_cards_at = |out: &mut Vec<ChatTurn>, upto: usize| {
        CHAT_CARDS.with(|cards| {
            for card in cards.borrow().iter().filter(|c| c.anchor.min(n) == upto) {
                if let Some(tab) = resolve_card_tab(card.kind, card.id) {
                    out.push(ChatTurn {
                        who: "".into(),
                        text: card.title.as_str().into(),
                        is_user: false,
                        kind: card.kind.into(),
                        tab: tab as i32,
                        sub: card.sub.as_str().into(),
                    });
                }
            }
        });
    };

    emit_cards_at(&mut out, 0);
    for (k, turn) in turns.into_iter().enumerate() {
        out.push(turn);
        emit_cards_at(&mut out, k + 1);
    }
    ui.global::<VS>()
        .set_chat_turns(ModelRc::from(Rc::new(VecModel::from(out))));
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
    // ---- image search history: flip to a past search ----
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_switch_image_tab(move |i: i32| {
            if let Some(ui) = w.upgrade() {
                IMAGES_ACTIVE.with(|a| a.set(i.max(0) as usize));
                sync_images(&ui);
            }
        });
    }
    // ---- chat result card → jump to that view + select its tab ----
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_chat_jump(move |kind: slint::SharedString, tab: i32| {
            let Some(ui) = w.upgrade() else { return };
            let tab = tab.max(0) as usize;
            match kind.as_str() {
                "images" => {
                    IMAGES_ACTIVE.with(|a| a.set(tab));
                    ensure_open(&ui, "images");
                    ui.global::<VS>().set_current_view("images".into());
                    sync_images(&ui);
                }
                "web" => {
                    WEB_ACTIVE.with(|a| a.set(tab));
                    ensure_open(&ui, "web");
                    ui.global::<VS>().set_current_view("web".into());
                    sync_web(&ui);
                }
                "videos" => {
                    VIDEOS_ACTIVE.with(|a| a.set(tab));
                    ensure_open(&ui, "videos");
                    ui.global::<VS>().set_current_view("videos".into());
                    ui.global::<VS>().set_playing_video_id("".into());
                    sync_videos(&ui);
                }
                _ => {}
            }
        });
    }
    // ---- video search history + player controls ----
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_switch_video_tab(move |i: i32| {
            if let Some(ui) = w.upgrade() {
                VIDEOS_ACTIVE.with(|a| a.set(i.max(0) as usize));
                ui.global::<VS>().set_playing_video_id("".into());
                sync_videos(&ui);
            }
        });
    }
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_play_video(move |i: i32| {
            let Some(ui) = w.upgrade() else { return };
            let i = i.max(0) as usize;
            let picked = VIDEO_TABS.with(|tabs| {
                let tabs = tabs.borrow();
                let active = VIDEOS_ACTIVE.with(|a| a.get());
                tabs.get(active).and_then(|b| b.items.get(i)).cloned()
            });
            if let Some(d) = picked {
                let vs = ui.global::<VS>();
                vs.set_playing_video_id(d.vid.as_str().into());
                vs.set_playing_video_title(d.title.as_str().into());
                vs.set_playing_video_url(d.url.as_str().into());
                #[cfg(target_os = "ios")]
                if !d.vid.is_empty() {
                    ios_video::play(&d.vid);
                }
            }
        });
    }
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_close_video_player(move || {
            if let Some(ui) = w.upgrade() {
                ui.global::<VS>().set_playing_video_id("".into());
                #[cfg(target_os = "ios")]
                ios_video::stop();
            }
        });
    }
    {
        let w = ui_weak.clone();
        ui.global::<VS>().on_open_video_external(move || {
            if let Some(ui) = w.upgrade() {
                let url = ui.global::<VS>().get_playing_video_url().to_string();
                if !url.is_empty() {
                    #[cfg(target_os = "android")]
                    if let Err(e) = crate::native_stt::launch_url(&url) {
                        eprintln!("[video] open failed: {e}");
                    }
                    #[cfg(target_os = "ios")]
                    crate::ios_url::launch_url(&url);
                }
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
                save_history();
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

    // Sample-content seed for a visual check without a live bot: desktop via VOIDAI_DEMO=1, or
    // an Android debug build (env vars don't reach the app there). iOS boots to a CLEAN SLATE —
    // real usage, empty views — so it's deliberately excluded (re-add `target_os = "ios"` below
    // to demo on the sim).
    if std::env::var("VOIDAI_DEMO").is_ok()
        || cfg!(all(target_os = "android", debug_assertions))
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
    // Two-bot ENGINE A/B: the PORT picks the pipeline — cascade bot (:8080) vs duplex/Gemini-Live
    // bot (:8081), each a fixed-engine instance. The persona (agent) is a per-connection ?agent=
    // param, so either engine can run either agent. Override per engine via
    // realtime_url_cascade.txt / realtime_url_duplex.txt in the app's files dir.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn bot_url_for(engine_index: i32, agent: &str) -> String {
        #[cfg(target_os = "ios")]
        let (cascade, duplex) = (
            "http://143.198.134.89:8080/api/offer",
            "http://143.198.134.89:8081/api/offer",
        );
        #[cfg(not(target_os = "ios"))]
        let (cascade, duplex) = (
            "http://10.0.2.2:7860/api/offer",
            "http://10.0.2.2:7861/api/offer",
        );
        let (default, fname) = if engine_index == 1 {
            (duplex, "realtime_url_duplex.txt")
        } else {
            (cascade, "realtime_url_cascade.txt")
        };
        let base = std::fs::read_to_string(format!("{}/{}", tts::files_dir(), fname))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default.to_string());
        // Persona (agent) → ?agent=nova|kaira; auth rides an x-bot-token header, so the query
        // string is free; honor an existing query in an override URL.
        let sep = if base.contains('?') { '&' } else { '?' };
        format!("{base}{sep}agent={agent}")
    }

    ui.on_realtime_toggle({
        let w = ui_weak.clone();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let now = !ui.get_rt_connected();
                #[cfg(target_os = "ios")]
                ios_haptics::tap();
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    if now {
                        // Never open a session with a dead mic. Gate on the true permission
                        // state and steer the tap: blocked → OS Settings; never-asked → prompt.
                        let mic = permissions::mic_status();
                        ui.global::<VS>().set_mic_perm(mic.as_i32());
                        match mic {
                            permissions::Perm::Denied => {
                                ui.set_rt_connected(false);
                                ui.set_rt_status("Microphone blocked — enable it in Settings".into());
                                permissions::open_app_settings();
                                return;
                            }
                            permissions::Perm::Undetermined => {
                                ui.set_rt_connected(false);
                                ui.set_rt_status("Allow microphone access to talk…".into());
                                request_permissions(&w);
                                return;
                            }
                            permissions::Perm::Granted => {}
                        }
                        ui.set_rt_connected(true);
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
                        let agent = ui.global::<Persona>().get_id();
                        let url = bot_url_for(ui.global::<Engine>().get_index(), agent.as_str());
                        ui.set_rt_failed(false);
                        ui.set_rt_status("connecting…".into());
                        realtime.connect(url);
                    } else {
                        ui.set_rt_connected(false);
                        realtime.disconnect();
                    }
                }
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                {
                    ui.set_rt_connected(now);
                    ui.set_rt_status("Realtime needs Android or iOS.".into());
                }
            }
        }
    });

    // Swipe on the agent icon switched persona → reconnect to that persona's bot if we're live.
    ui.on_persona_switched({
        let w = ui_weak.clone();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                #[cfg(target_os = "ios")]
                ios_haptics::tap();
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    if ui.get_rt_connected() {
                        let agent = ui.global::<Persona>().get_id();
                        let url = bot_url_for(ui.global::<Engine>().get_index(), agent.as_str());
                        ui.set_rt_failed(false);
                        ui.set_rt_status("switching…".into());
                        realtime.connect(url);
                    }
                }
            }
        }
    });

    // A/B engine flipped (Cascade/Duplex) → reconnect with the new ?mode= if we're live.
    ui.on_engine_switched({
        let w = ui_weak.clone();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    if ui.get_rt_connected() {
                        let agent = ui.global::<Persona>().get_id();
                        let url = bot_url_for(ui.global::<Engine>().get_index(), agent.as_str());
                        ui.set_rt_failed(false);
                        ui.set_rt_status("switching…".into());
                        realtime.connect(url);
                    }
                }
            }
        }
    });

    // Mic mute → tell the realtime client to send silence instead of the mic.
    ui.on_mic_mute({
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move |muted| {
            #[cfg(target_os = "ios")]
            ios_haptics::tap();
            #[cfg(any(target_os = "android", target_os = "ios"))]
            realtime.set_muted(muted);
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let _ = muted;
        }
    });

    // Lightbox image opened/switched → hand its URL to the bot so Nova (Gemini/duplex) can see it.
    // index < 0 means the lightbox closed → clear.
    ui.on_view_image({
        let w = ui_weak.clone();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move |idx: i32| {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            if let Some(ui) = w.upgrade() {
                if idx < 0 {
                    realtime.set_viewing(String::new()); // lightbox closed → clear
                } else {
                    let url = ui
                        .global::<VS>()
                        .get_images()
                        .as_any()
                        .downcast_ref::<VecModel<ImageItem>>()
                        .and_then(|vm| vm.row_data(idx as usize))
                        .map(|it| it.url.to_string())
                        .unwrap_or_default();
                    // Searched images carry a URL → send it. An IMPORTED photo has an empty URL;
                    // its pixels were already sent on import, so leave VIEWING alone (don't clear).
                    if !url.is_empty() {
                        realtime.set_viewing(url);
                    }
                }
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let _ = idx;
        }
    });

    // Upload button → open the iOS photo picker. When a photo comes back, decode it into the
    // Images view + open it, and (duplex only) ship its resized JPEG to the bot so Nova sees it.
    #[cfg(target_os = "ios")]
    {
        let w = ui_weak.clone();
        ios_photos::set_on_picked(move |encoded: Vec<u8>| {
            if let Some(ui) = w.upgrade() {
                import_picked_image(&ui, &encoded);
                if let Some(b64) = encode_jpeg_b64(&encoded) {
                    realtime::set_viewing_bytes(b64); // free fn — no Rc<Realtime> to capture
                }
            }
        });
    }
    ui.on_pick_image({
        let w = ui_weak.clone();
        move || {
            #[cfg(target_os = "ios")]
            if let Some(ui) = w.upgrade() {
                ios_photos::present(ui.window());
            }
            #[cfg(not(target_os = "ios"))]
            let _ = &w;
        }
    });

    // Tap the waveform → cut Nova off (instant local silence + tell the bot to stop).
    ui.on_interrupt_nova({
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            #[cfg(target_os = "ios")]
            ios_haptics::tap();
            #[cfg(any(target_os = "android", target_os = "ios"))]
            realtime.interrupt();
        }
    });

    // Nova ended the call (user said goodbye) → drop the realtime session. Fired by the UI once
    // her sign-off has played (hangup-grace), so we don't cut her off mid-farewell.
    ui.on_hang_up({
        let w = ui_weak.clone();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let realtime = realtime.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                ui.set_rt_connected(false);
            }
            #[cfg(target_os = "ios")]
            ios_haptics::tap();
            #[cfg(any(target_os = "android", target_os = "ios"))]
            realtime.disconnect();
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
    // Tap a place in the map list → fly the interactive map to it (iOS). Other platforms have no
    // live map, so fall back to the system Maps app (the previous behaviour).
    ui.on_focus_pin(move |lat, lng| {
        if lat == 0.0 && lng == 0.0 {
            return;
        }
        #[cfg(target_os = "ios")]
        ios_map::focus(lat as f64, lng as f64);
        #[cfg(target_os = "android")]
        {
            let _ = crate::native_stt::launch_url(&format!(
                "https://www.google.com/maps/search/?api=1&query={lat},{lng}"
            ));
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        eprintln!("[focus-pin] {lat},{lng}");
    });

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

    // First-run + recovery permission prompts (mic required, location optional). Fired by
    // the onboarding CTA and by the connect gate when the mic was never asked for.
    ui.on_request_permissions({
        let w = ui_weak.clone();
        move || request_permissions(&w)
    });

    // Take a blocked user to this app's OS Settings page so they can flip the mic switch.
    ui.on_open_app_settings(move || permissions::open_app_settings());

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
        if let Ok(v) = std::fs::read_to_string(format!("{}/setting_engine.txt", dir)) {
            // A/B engine: remember the last Cascade/Duplex pick across launches.
            ui.global::<Engine>().set_index(if v.trim() == "duplex" { 1 } else { 0 });
        }
        // Seed the UI with the true OS permission state so the Talk surface can show a
        // blocked mic immediately (not just after a failed connect attempt).
        ui.global::<VS>().set_mic_perm(permissions::mic_status().as_i32());
        ui.global::<VS>().set_loc_perm(permissions::location_status().as_i32());
        // First run (no persisted "seen" flag) → show the onboarding + permission-priming
        // overlay before the user can tap Talk.
        let seen = std::fs::read_to_string(format!("{}/setting_onboarding_seen.txt", dir))
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        if !seen {
            ui.global::<VS>().set_onboarding_open(true);
        }
    }

    // Tap-to-retry a failed image/product thumbnail: clear `failed` and re-fetch its URL.
    ui.on_retry_image({
        let w = ui_weak.clone();
        move |i: i32| {
            if let Some(ui) = w.upgrade() {
                let vs = ui.global::<VS>();
                if let Some(vm) = vs.get_images().as_any().downcast_ref::<VecModel<ImageItem>>() {
                    if let Some(mut row) = vm.row_data(i as usize) {
                        let url = row.url.to_string();
                        row.failed = false;
                        vm.set_row_data(i as usize, row);
                        if !url.is_empty() {
                            spawn_image_fetch(&ui, url, ImgSlot::Images(i as usize));
                        }
                    }
                }
            }
        }
    });
    ui.on_retry_product({
        let w = ui_weak.clone();
        move |i: i32| {
            if let Some(ui) = w.upgrade() {
                let vs = ui.global::<VS>();
                if let Some(vm) = vs.get_products().as_any().downcast_ref::<VecModel<Product>>() {
                    if let Some(mut row) = vm.row_data(i as usize) {
                        let url = row.thumb.to_string();
                        row.failed = false;
                        vm.set_row_data(i as usize, row);
                        if !url.is_empty() {
                            spawn_image_fetch(&ui, url, ImgSlot::Products(i as usize));
                        }
                    }
                }
            }
        }
    });

    // Tap-to-retry the static map tile after a failed load.
    ui.on_retry_map({
        let w = ui_weak.clone();
        move || {
            if let Some(ui) = w.upgrade() {
                let url = LAST_MAP_URL.with(|u| u.borrow().clone());
                ui.global::<VS>().set_map_failed(false);
                if !url.is_empty() {
                    spawn_image_fetch(&ui, url, ImgSlot::Map);
                }
            }
        }
    });

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
        // Tuck the native map webview behind Slint overlays (add-view sheet, settings, …) while
        // one is up, then reveal it again once they all close — but only if we're still on the map.
        let w2 = ui_weak.clone();
        ui.on_set_map_obscured(move |obscured| {
            let on_map = w2
                .upgrade()
                .map(|ui| ui.global::<VS>().get_current_view().as_str() == "map")
                .unwrap_or(false);
            ios_map::set_hidden(obscured || !on_map);
        });
    }

    // iOS: mirror the OS "Reduce Motion" accessibility setting so the looping idle animations
    // (breathing avatars, pulses) go still for users who ask for less motion. Read once at start.
    #[cfg(target_os = "ios")]
    {
        extern "C" {
            fn UIAccessibilityIsReduceMotionEnabled() -> bool;
        }
        let reduce = unsafe { UIAccessibilityIsReduceMotionEnabled() };
        ui.global::<VS>().set_reduce_motion(reduce);
    }

    // iOS: real in-app web reader — a WKWebView positioned to the web content region, hidden
    // when the web view is inactive or a Slint overlay needs to draw over it (mirrors the map).
    #[cfg(target_os = "ios")]
    {
        ui.global::<VS>().set_has_web_reader(true);
        let w = ui_weak.clone();
        ui.on_report_web_geom(move |x, y, wd, h| {
            if let Some(ui) = w.upgrade() {
                ios_web::show_at(ui.window(), x, y, wd, h);
            }
        });
        ui.on_web_active_changed(|active| {
            if !active {
                ios_web::hide();
            }
        });
        let w2 = ui_weak.clone();
        ui.on_set_web_obscured(move |obscured| {
            let on_web = w2
                .upgrade()
                .map(|ui| ui.global::<VS>().get_current_view().as_str() == "web")
                .unwrap_or(false);
            ios_web::set_hidden(obscured || !on_web);
        });
    }

    // iOS: video player webview positioning + frame-vision timer (mirrors the web reader).
    #[cfg(target_os = "ios")]
    {
        let w = ui_weak.clone();
        ui.on_report_video_geom(move |x, y, wd, h| {
            if let Some(ui) = w.upgrade() {
                ios_video::show_at(ui.window(), x, y, wd, h);
            }
        });
        let w2 = ui_weak.clone();
        ui.on_video_active_changed(move |active| {
            if active {
                // Snapshot the current frame every 3s so Nova can answer "what's happening?".
                VIDEO_VISION_TIMER.with(|t| {
                    let timer = slint::Timer::default();
                    timer.start(
                        slint::TimerMode::Repeated,
                        std::time::Duration::from_secs(3),
                        || ios_video::snapshot_to_vision(),
                    );
                    *t.borrow_mut() = Some(timer);
                });
            } else {
                VIDEO_VISION_TIMER.with(|t| t.borrow_mut().take());
                ios_video::stop();
                // Leaving the view (or ending playback) drops the player back to the grid.
                if let Some(ui) = w2.upgrade() {
                    ui.global::<VS>().set_playing_video_id("".into());
                }
            }
        });
        let w3 = ui_weak.clone();
        ui.on_set_video_obscured(move |obscured| {
            let playing = w3
                .upgrade()
                .map(|ui| {
                    let vs = ui.global::<VS>();
                    vs.get_current_view().as_str() == "videos"
                        && !vs.get_playing_video_id().is_empty()
                })
                .unwrap_or(false);
            ios_video::set_hidden(obscured || !playing);
        });
    }

    // Restore the persisted image/web search history into the tab strips (mobile only).
    #[cfg(any(target_os = "android", target_os = "ios"))]
    persist::load(&ui);

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
