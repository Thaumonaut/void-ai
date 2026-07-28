//! iOS: an interactive Mapbox GL JS map in a WKWebView, positioned to cover the Slint
//! "map" view's tile region (reported from app.slint via report-map-geom). The Slint search
//! header and ETA/"Open in Maps" bar sit ABOVE and BELOW this region, so a plain sub-window
//! (webview on top, no transparency/passthrough) is enough — the chrome never overlaps it.
//! Hidden (kept alive) when the user leaves the map view. Main-thread only (Slint callbacks).
//!
//! The map's content is driven by Nova's `map` UI-control op: `set_data` pushes markers/center
//! JSON into the page's `window.updateMap(...)`, double-buffered (Rust holds the latest until
//! the webview exists; JS holds it until the map's `load` fires).

use std::cell::RefCell;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    static MAP_WEBVIEW: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    // Latest map data (JSON) from Nova, applied once the webview exists.
    static PENDING_JSON: RefCell<Option<String>> = const { RefCell::new(None) };
    // Repeating timer that pushes the phone's location into the map (so the "you are here" dot +
    // recenter target land as soon as a GPS fix arrives, and stay current as the user moves).
    static USER_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

// Mapbox GL JS. Public (pk.*) token — GL JS rejects secret sk.* tokens. `window.updateMap`
// takes { markers:[{lng,lat,label}], center:[lng,lat], zoom } and fits/flies to it.
// Public (pk.) Mapbox token for rendering the map — publishable, but injected at BUILD time from
// the gitignored .env.local (MAPBOX_PUBLIC_TOKEN) so it stays out of committed/open-source code.
// Substituted into MAPBOX_HTML's `__MAPBOX_PK__` placeholder when the web view loads.
const MAPBOX_PK: &str = match option_env!("MAPBOX_PUBLIC_TOKEN") {
    Some(t) => t,
    None => "",
};

const MAPBOX_HTML: &str = r#"<!doctype html><html><head>
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<link href="https://unpkg.com/mapbox-gl@3/dist/mapbox-gl.css" rel="stylesheet">
<script src="https://unpkg.com/mapbox-gl@3/dist/mapbox-gl.js"></script>
<style>html,body,#map{margin:0;height:100%;width:100%;background:#dbe4d6}
.userdot{width:16px;height:16px;border-radius:50%;background:#1560ff;border:3px solid #fff;box-shadow:0 0 0 2px rgba(21,96,255,.35)}
#recenter{position:absolute;right:10px;bottom:12px;width:46px;height:46px;border-radius:23px;border:none;background:#fff;box-shadow:0 1px 5px rgba(0,0,0,.35);display:flex;align-items:center;justify-content:center;z-index:9;-webkit-tap-highlight-color:transparent}
#recenter:active{background:#eef1f6}</style>
</head><body>
<div id="map"></div>
<button id="recenter" aria-label="Center on my location" onclick="window.recenter()"><svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke='#1560ff' stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="3.5"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3"/></svg></button>
<div id="err" style="position:absolute;bottom:6px;left:6px;right:64px;color:#a00;background:rgba(255,255,255,.92);font:12px -apple-system,sans-serif;padding:5px 7px;border-radius:6px;z-index:9;display:none;white-space:pre-wrap"></div>
<script>
 function showErr(m){ var e=document.getElementById('err'); if(!m){ e.style.display='none'; return; } e.style.display='block'; e.textContent=String(m).slice(0,220); }
 window.onerror=function(m){ showErr('JS: '+m); };
 mapboxgl.accessToken='__MAPBOX_PK__';
 const map=new mapboxgl.Map({container:'map',style:'mapbox://styles/mapbox/streets-v12',center:__INIT_CENTER__,zoom:__INIT_ZOOM__});
 map.addControl(new mapboxgl.NavigationControl(),'top-right');
 let markers=[], pending=null, ready=false;
 // The phone's location, pushed from native (window.setUser). Drops a "you are here" dot and
 // centers on the user the first time — unless Nova's data has already claimed the view.
 let userLoc=null, userMarker=null, centered=false;
 window.setUser=function(lng,lat){
   userLoc=[lng,lat];
   if(!userMarker){ var el=document.createElement('div'); el.className='userdot'; userMarker=new mapboxgl.Marker({element:el}).setLngLat(userLoc).addTo(map); }
   else { userMarker.setLngLat(userLoc); }
   if(!centered){ map.jumpTo({center:userLoc,zoom:14}); centered=true; }
 };
 window.recenter=function(){ if(userLoc){ showErr(''); map.flyTo({center:userLoc,zoom:15,duration:600}); } else { showErr('finding your location… (grant Location if you haven\'t)'); } };
 // Tapping a place in the list flies the map to it and zooms in.
 window.focusPin=function(lng,lat){ centered=true; map.flyTo({center:[lng,lat],zoom:16,duration:700}); };
 map.on('load',function(){ ready=true; if(pending){ apply(pending); pending=null; } });
 window.updateMap=function(d){ if(!ready){ pending=d; return; } apply(d); };
 function clearMarkers(){ markers.forEach(function(m){ m.remove(); }); markers=[]; }
 function addMarker(lng,lat,label,color){
   var m=new mapboxgl.Marker({color:color||'#ef4d2a'}).setLngLat([lng,lat]);
   if(label){ m.setPopup(new mapboxgl.Popup({offset:24}).setText(label)); }
   m.addTo(map); markers.push(m);
 }
 function removeRoute(){ ['route','route-casing'].forEach(function(id){ if(map.getLayer(id)) map.removeLayer(id); }); if(map.getSource('route')) map.removeSource('route'); }
 function drawRoute(geo){
   var data={type:'Feature',geometry:geo};
   if(map.getSource('route')){ map.getSource('route').setData(data); return; }
   map.addSource('route',{type:'geojson',data:data});
   map.addLayer({id:'route-casing',type:'line',source:'route',layout:{'line-join':'round','line-cap':'round'},
     paint:{'line-color':'#ffffff','line-width':9}});
   map.addLayer({id:'route',type:'line',source:'route',layout:{'line-join':'round','line-cap':'round'},
     paint:{'line-color':'#1560ff','line-width':5}});
 }
 function fitCoords(coords){ if(!coords.length) return; var b=new mapboxgl.LngLatBounds();
   coords.forEach(function(c){ b.extend(c); }); map.fitBounds(b,{padding:64,maxZoom:16,duration:600}); }
 function fetchRoute(o,dst,mode){
   var url='https://api.mapbox.com/directions/v5/mapbox/'+(mode||'driving')+'/'+o[0]+','+o[1]+';'+dst[0]+','+dst[1]+
     '?geometries=geojson&overview=full&access_token='+mapboxgl.accessToken;
   showErr('fetching route…');
   fetch(url).then(function(r){return r.json();}).then(function(j){
     if(j.routes&&j.routes[0]){ showErr(''); var g=j.routes[0].geometry; drawRoute(g); fitCoords(g.coordinates); }
     else { showErr('no route: '+(j.message||j.code||JSON.stringify(j).slice(0,120))); }
   }).catch(function(e){ showErr('fetch: '+((e&&e.message)||e)); });
 }
 function apply(d){
   if((d.markers&&d.markers.length)||d.route||d.center){ centered=true; }
   clearMarkers(); removeRoute();
   var pts=d.markers||[];
   pts.forEach(function(p){ addMarker(p.lng,p.lat,p.label); });
   var r=d.route;
   if(r){
     if(r.dest) addMarker(r.dest[0],r.dest[1],r.destLabel||'Destination','#128577');
     if(r.origin) addMarker(r.origin[0],r.origin[1],'Start','#ef4d2a');
     if(r.geometry){ drawRoute(r.geometry); fitCoords(r.geometry.coordinates); }
     else if(r.origin&&r.dest){ fetchRoute(r.origin,r.dest,r.mode); }
     else if(r.dest){ map.flyTo({center:r.dest,zoom:14,duration:600}); }
   } else if(pts.length>1){ fitCoords(pts.map(function(p){return [p.lng,p.lat];})); }
   else if(pts.length===1){ map.flyTo({center:[pts[0].lng,pts[0].lat],zoom:14,duration:600}); }
   else if(d.center){ map.flyTo({center:d.center,zoom:d.zoom||12,duration:600}); }
 }
</script>
</body></html>"#;

fn host_view(window: &slint::Window) -> Option<*mut AnyObject> {
    MainThreadMarker::new()?;
    let sh = window.window_handle();
    match sh.window_handle().ok()?.as_raw() {
        RawWindowHandle::UiKit(h) => Some(h.ui_view.as_ptr() as *mut AnyObject),
        _ => None,
    }
}

unsafe fn eval_js(webview: &AnyObject, js: &str) {
    let ns = NSString::from_str(js);
    let no_handler: *const AnyObject = core::ptr::null();
    let _: () = msg_send![webview, evaluateJavaScript: &*ns, completionHandler: no_handler];
}

/// Push the latest map content (JSON: { markers:[{lng,lat,label}], center?, zoom? }). Buffered
/// until the webview exists; the page itself buffers until the Mapbox `load` event.
pub fn set_data(json: &str) {
    PENDING_JSON.with(|p| *p.borrow_mut() = Some(json.to_string()));
    apply_pending();
}

fn apply_pending() {
    unsafe {
        MAP_WEBVIEW.with(|m| {
            let m = m.borrow();
            let Some(wv) = m.as_ref() else { return };
            PENDING_JSON.with(|p| {
                if let Some(json) = p.borrow().as_ref() {
                    eval_js(&**wv, &format!("window.updateMap({json});"));
                }
            });
        });
    }
}

/// Show (creating on first call) the Mapbox webview and position it to (x, y, w, h) in
/// points. Called from Slint's report-map-geom whenever the map region appears or resizes.
pub fn show_at(window: &slint::Window, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    // Opening the map is enough to want GPS — start it here (idempotent) so we don't depend on the
    // user having connected first; this also triggers the location prompt if it's undetermined.
    crate::ios_location::start();
    // Keep pushing the fix into the webview so the dot + recenter target appear as soon as a fix
    // lands (the initial one-shot pushes can all miss if the fix or the CDN isn't ready yet).
    USER_TIMER.with(|t| {
        if t.borrow().is_none() {
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::Repeated,
                std::time::Duration::from_millis(1500),
                push_user,
            );
            *t.borrow_mut() = Some(timer);
        }
    });
    let Some(host_ptr) = host_view(window) else { return };
    let frame = CGRect {
        origin: CGPoint { x: x as f64, y: y as f64 },
        size: CGSize { width: w as f64, height: h as f64 },
    };
    unsafe {
        let exists = MAP_WEBVIEW.with(|m| m.borrow().is_some());
        if !exists {
            let config: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            let alloc: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let webview: Retained<AnyObject> =
                msg_send![alloc, initWithFrame: frame, configuration: &*config];
            let _: () = msg_send![&*webview, setOpaque: false];
            // Seed the map's initial center with the phone's location so it doesn't flash the
            // Seattle default before the first setUser push (falls back to Seattle if no fix yet).
            let (clng, clat, zoom) = match crate::ios_location::last_location() {
                Some((lat, lng)) => (lng, lat, 14),
                None => (-122.3321_f64, 47.6062_f64, 11),
            };
            let html_str = MAPBOX_HTML
                .replace("__MAPBOX_PK__", MAPBOX_PK)
                .replace("__INIT_CENTER__", &format!("[{clng},{clat}]"))
                .replace("__INIT_ZOOM__", &zoom.to_string());
            let html = NSString::from_str(&html_str);
            let base = NSString::from_str("https://kaira.local/");
            if let Some(url) = NSURL::URLWithString(&base) {
                let _: *mut AnyObject = msg_send![&*webview, loadHTMLString: &*html, baseURL: &*url];
            }
            let host: &AnyObject = &*host_ptr;
            let _: () = msg_send![host, addSubview: &*webview];
            MAP_WEBVIEW.with(|m| *m.borrow_mut() = Some(webview));
            // window.updateMap isn't defined until the CDN + inline scripts load, so an
            // immediate eval no-ops. Retry a few times; updateMap buffers internally until
            // the Mapbox 'load' event, so repeated calls are idempotent (last data wins).
            for ms in [500u64, 1200, 2500, 4500] {
                slint::Timer::single_shot(std::time::Duration::from_millis(ms), || {
                    apply_pending();
                    push_user();
                });
            }
        }
        MAP_WEBVIEW.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let _: () = msg_send![&**wv, setFrame: frame];
                let _: () = msg_send![&**wv, setHidden: false];
            }
        });
        // Refresh the "you are here" dot / center each time the map appears or resizes.
        push_user();
        // Apply any map data that arrived before the webview existed.
        apply_pending();
    }
}

/// Fly the interactive map to a place tapped in the list and zoom in. No-op if the webview
/// isn't up yet; `window.focusPin` guards its own readiness.
pub fn focus(lat: f64, lng: f64) {
    unsafe {
        MAP_WEBVIEW.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                eval_js(&**wv, &format!("window.focusPin&&window.focusPin({lng},{lat});"));
            }
        });
    }
}

/// Push the phone's current location into the map: a "you are here" dot + first-open centering.
/// No-op if there's no fix yet or the webview isn't up; `window.setUser` guards its own readiness.
fn push_user() {
    if let Some((lat, lng)) = crate::ios_location::last_location() {
        unsafe {
            MAP_WEBVIEW.with(|m| {
                if let Some(wv) = m.borrow().as_ref() {
                    let hidden: bool = msg_send![&**wv, isHidden];
                    if hidden {
                        return;
                    }
                    eval_js(&**wv, &format!("window.setUser&&window.setUser({lng},{lat});"));
                }
            });
        }
    }
}

/// Hide the map webview (kept alive so the map + tiles don't reload next time).
pub fn hide() {
    set_hidden(true);
}

/// Toggle the map webview's visibility without reframing it. Used to tuck the native
/// webview behind Slint overlays (add-view sheet, settings, lightbox) that would otherwise
/// be drawn under it, then reveal it again when they close. No-op if it doesn't exist yet.
pub fn set_hidden(hidden: bool) {
    unsafe {
        MAP_WEBVIEW.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let _: () = msg_send![&**wv, setHidden: hidden];
            }
        });
    }
}
