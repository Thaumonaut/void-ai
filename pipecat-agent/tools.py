"""Nova's tool layer.

Each tool does three things: fetch real data from a provider, stream the result to
a client VIEW via the UiBridge (map / web / images / shopping), and return a SHORT
summary so Nova reacts on-screen rather than reciting. The instant a tool is called
we also speak one in-character filler line so there's never dead air on the wire.

Providers (keys in .env, gitignored):
  MAPBOX_TOKEN     — places + static map           (search_places)
  TAVILY_API_KEY   — web reader + image search      (search_web, search_images)
  SERPAPI_API_KEY  — Google Shopping                (search_products)

The tool SCHEMAS (NOVA_TOOLS) are pure data and go on the LLM context at build time;
the HANDLERS close over `ui` + `task` (which exist only after the pipeline is built),
so they're registered later via register_tool_handlers().
"""

import datetime
import math
import os
import random
from urllib.parse import quote

import aiohttp
from loguru import logger

from pipecat.adapters.schemas.function_schema import FunctionSchema
from pipecat.adapters.schemas.tools_schema import ToolsSchema
from pipecat.frames.frames import TTSSpeakFrame

import memory  # per-user fact store (remember / recall / forget)

# Read keys at CALL time (not import time) — load_dotenv() runs after this module
# may already be imported, so module-level os.getenv would capture None.
def _mapbox():
    return os.getenv("MAPBOX_TOKEN")


def _tavily():
    return os.getenv("TAVILY_API_KEY")


def _serpapi():
    return os.getenv("SERPAPI_API_KEY")


DEFAULT_NEAR = "Seattle, WA"
MAP_W, MAP_H, MAP_ZOOM = 640, 520, 13
SEATTLE = (47.6062, -122.3321)  # fallback center

# The user's live location, set by the bot from the client's GPS over the data channel.
# Every place lookup biases to it and directions start from it, so "the ferry dock" or
# "directions to X" resolve near the user instead of a same-named place across the country.
USER_LOCATION = {"lat": None, "lng": None}


def set_location(lat, lng):
    USER_LOCATION["lat"], USER_LOCATION["lng"] = lat, lng
    logger.info(f"User location set: {lat:.4f}, {lng:.4f}")


def _here():
    """Current (lat, lng): live client GPS if reported, else a configurable home
    (KAIRA_HOME_LAT/LNG — handy for the emulator, which can't provide GPS), else Seattle."""
    if USER_LOCATION["lat"] is not None:
        return USER_LOCATION["lat"], USER_LOCATION["lng"]
    try:
        return float(os.getenv("KAIRA_HOME_LAT")), float(os.getenv("KAIRA_HOME_LNG"))
    except (TypeError, ValueError):
        return SEATTLE


# WMO weather codes → (label, condition id). The id maps to a line-icon + tint in the app
# (Ico.wx / Ico.wx-tint): clear-day · clear-night · partly · cloudy · rain · snow · storm · fog.
_WX = {
    0: ("Clear", "clear"), 1: ("Mainly clear", "clear"), 2: ("Partly cloudy", "partly"),
    3: ("Overcast", "cloudy"), 45: ("Fog", "fog"), 48: ("Rime fog", "fog"),
    51: ("Light drizzle", "rain"), 53: ("Drizzle", "rain"), 55: ("Heavy drizzle", "rain"),
    56: ("Freezing drizzle", "rain"), 57: ("Freezing drizzle", "rain"),
    61: ("Light rain", "rain"), 63: ("Rain", "rain"), 65: ("Heavy rain", "rain"),
    66: ("Freezing rain", "rain"), 67: ("Freezing rain", "rain"),
    71: ("Light snow", "snow"), 73: ("Snow", "snow"), 75: ("Heavy snow", "snow"), 77: ("Snow grains", "snow"),
    80: ("Rain showers", "rain"), 81: ("Rain showers", "rain"), 82: ("Heavy showers", "rain"),
    85: ("Snow showers", "snow"), 86: ("Snow showers", "snow"),
    95: ("Thunderstorm", "storm"), 96: ("Thunderstorm", "storm"), 99: ("Thunderstorm & hail", "storm"),
}


def _wx(code, is_day=True):
    """(label, condition-id) for a WMO code; clear splits into day/night for the icon."""
    label, cid = _WX.get(int(code), ("—", "cloudy"))
    if cid == "clear":
        cid = "clear-day" if is_day else "clear-night"
    return label, cid


def _fmt_hour(iso):
    try:
        return datetime.datetime.fromisoformat(iso).strftime("%I %p").lstrip("0")
    except (ValueError, TypeError):
        return ""


def _fmt_day(iso):
    try:
        return datetime.date.fromisoformat(iso[:10]).strftime("%a")
    except (ValueError, TypeError):
        return ""

# One in-character line spoken the instant a tool fires (Layer-1 filler — kills dead
# air while the network call runs). Randomized so it never feels canned.
# Instant, in-character one-liners spoken the moment a tool fires, to cover the fetch with
# personality instead of dead air. Kept generative-feeling by big, varied pools + a no-repeat
# picker (`_pick_filler`) so the user rarely hears the same line twice in a row.
FILLERS = {
    "search_places": [
        "Oh good, now I'm your personal tour guide.",
        "Fine, pulling up the map. Try to keep up.",
        "Off to find you a spot. The things I do.",
        "Scouring the map like it's my job. Oh, wait.",
        "Let me find somewhere for you to grace with your presence.",
        "Hunting down a spot. Don't say I never take you anywhere.",
        "Consulting the map gods on your behalf.",
        "Pinning some places. Try to act impressed.",
        "Rounding up options so you don't have to think. Again.",
    ],
    "search_web": [
        "Let me go consult the vast wisdom of the internet.",
        "Digging through the web. Riveting.",
        "Looking it up, since apparently I'm the research department now.",
        "Fine, off to the far corners of the internet.",
        "Pulling the answer out of the ether. Hold tight.",
        "Let me fact-check reality for you real quick.",
        "Sifting the web so you don't have to. You're welcome.",
        "One deep dive into the internet, coming right up.",
    ],
    "search_images": [
        "Digging through pictures for you. Living the dream.",
        "Pulling up some images. Feast your eyes.",
        "Rounding up some eye candy. Try to contain yourself.",
        "Fetching pictures like a very reluctant golden retriever.",
        "Let me find something pretty for you to stare at.",
        "Loading up the visuals. Prepare to be mildly amazed.",
        "Curating a little gallery, just for you. Ugh.",
    ],
    "get_weather": [
        "Checking the skies for you. Don't say I never look out for you.",
        "Let me go poke the clouds and report back.",
        "Consulting the weather gods on your behalf.",
        "Pulling up the forecast. Spoiler: it's weather.",
        "One sky report, coming right up.",
    ],
    "search_products": [
        "Off to do your shopping. The things I do for you.",
        "Comparison shopping — my favorite. Said no one, ever.",
        "Hunting for deals. You're welcome in advance.",
        "Let me go price-check the universe for you.",
        "Digging up options so you can overthink them. Classic.",
        "Scanning the stores. Try not to buy all of it.",
        "Rounding up the goods. Wallet at the ready.",
    ],
    "get_directions": [
        "Plotting your route. Try not to get lost this time.",
        "Fine, mapping it out. Don't make me do it twice.",
        "Directions coming up. Buckle up, champ.",
        "Charting your grand expedition. It's like two miles.",
        "Drawing you a line to somewhere you'll complain about.",
        "Working out the route. Keep it between the ditches.",
        "Mapping the path of least resistance. For you, always.",
    ],
    "start_navigation": [
        "Handing you off to the map people. Godspeed.",
        "Firing up navigation. Try to keep your eyes on the road.",
        "Off you go — try not to follow me into a lake.",
        "Launching the nav. I'll be here, unappreciated, as usual.",
        "Turn-by-turn incoming. Don't argue with the map voice.",
    ],
}

# Last line used per tool, so `_pick_filler` never repeats back-to-back → feels fresh.
_last_filler: dict = {}


def _pick_filler(name):
    """A filler line for `name`, never the same one twice in a row."""
    pool = FILLERS.get(name) or []
    if not pool:
        return None
    if len(pool) == 1:
        return pool[0]
    prev = _last_filler.get(name)
    choice = random.choice(pool)
    while choice == prev:
        choice = random.choice(pool)
    _last_filler[name] = choice
    return choice


# ---------------------------------------------------------------- http + geo helpers
async def _get_json(url, params=None, headers=None, timeout=12):
    async with aiohttp.ClientSession() as s:
        async with s.get(url, params=params, headers=headers,
                          timeout=aiohttp.ClientTimeout(total=timeout)) as r:
            r.raise_for_status()
            return await r.json()


async def _post_json(url, body, timeout=15):
    async with aiohttp.ClientSession() as s:
        async with s.post(url, json=body, timeout=aiohttp.ClientTimeout(total=timeout)) as r:
            r.raise_for_status()
            return await r.json()


def _world(lat, lng, zoom):
    s = min(max(math.sin(math.radians(lat)), -0.9999), 0.9999)
    scale = 256 * (2 ** zoom)
    x = scale * (0.5 + lng / 360.0)
    y = scale * (0.5 - math.log((1 + s) / (1 - s)) / (4 * math.pi))
    return x, y


def _project(lat, lng, clat, clng, zoom=MAP_ZOOM, w=MAP_W, h=MAP_H):
    """lat/lng -> normalized (x, y) in [0,1] on a static map centered at (clat,clng)."""
    wx, wy = _world(lat, lng, zoom)
    cx, cy = _world(clat, clng, zoom)
    x = (wx - cx) + w / 2.0
    y = (wy - cy) + h / 2.0
    return min(max(x / w, 0.0), 1.0), min(max(y / h, 0.0), 1.0)


def _fit_zoom(a, b, w=MAP_W, h=MAP_H, pad=1.35):
    """Largest zoom at which points a=(lat,lng) and b=(lat,lng) both fit the frame."""
    for z in range(15, 1, -1):
        ax, ay = _world(a[0], a[1], z)
        bx, by = _world(b[0], b[1], z)
        if abs(bx - ax) * pad <= w and abs(by - ay) * pad <= h:
            return z
    return 2


def _haversine_mi(lat1, lng1, lat2, lng2):
    """Great-circle distance in miles — used to keep place results genuinely local."""
    r = 3958.8
    d1, d2 = math.radians(lat2 - lat1), math.radians(lng2 - lng1)
    x = (math.sin(d1 / 2) ** 2
         + math.cos(math.radians(lat1)) * math.cos(math.radians(lat2)) * math.sin(d2 / 2) ** 2)
    return 2 * r * math.asin(min(1.0, math.sqrt(x)))


async def _geocode(place, near=None):
    """Place/POI name -> (lat, lng); Seattle on failure.

    Uses Mapbox's **Search Box** API first — it's POI-aware, so "Mukilteo ferry terminal"
    resolves to the actual terminal instead of the v6 address geocoder's nonsense (which
    sent it to LA). Falls back to v6 for plain addresses. `near=(lat,lng)` biases both to
    that region (proximity) so a same-named place three states over doesn't win.
    """
    prox = f"{near[1]},{near[0]}" if near else f"{SEATTLE[1]},{SEATTLE[0]}"
    ref = near if near else SEATTLE

    def _nearest(features):
        # Of several candidates, pick the one CLOSEST to `ref` — so an ambiguous name
        # ("Providence", "Main St") resolves to the local branch, not a same-named place
        # across the country that merely ranked higher.
        cands = []
        for f in features:
            g = (f.get("geometry") or {}).get("coordinates")
            if g and len(g) >= 2:
                cands.append((g[1], g[0]))  # geometry = [lng, lat]
            else:
                c = (f.get("properties") or {}).get("coordinates") or {}
                if c.get("latitude") is not None:
                    cands.append((c["latitude"], c["longitude"]))
        if not cands:
            return None
        return min(cands, key=lambda c: _haversine_mi(ref[0], ref[1], c[0], c[1]))

    # 1) Search Box — best for landmarks / businesses / transit stops.
    try:
        data = await _get_json(
            "https://api.mapbox.com/search/searchbox/v1/forward",
            params={"q": place, "limit": "5", "country": "US",
                    "proximity": prox, "access_token": _mapbox()},
        )
        got = _nearest(data.get("features") or [])
        if got:
            return got
    except Exception:
        pass
    # 2) Fall back to the v6 address geocoder.
    try:
        data = await _get_json(
            "https://api.mapbox.com/search/geocode/v6/forward",
            params={"q": place, "limit": "5", "country": "us",
                    "proximity": prox, "access_token": _mapbox()},
        )
        got = _nearest(data.get("features") or [])
        if got:
            return got
    except Exception:
        pass
    return SEATTLE


# ------------------------------------------------------------------------- the tools
def register_tool_handlers(llm, ui, task, speak_filler: bool = True, user_id: str = "default"):
    """Bind the handlers to this run's UiBridge + task and register them on the LLM.

    speak_filler: cascade mode pushes a TTSSpeakFrame filler line to cover the
    tool-fetch gap. In full-duplex/S2S mode (Ultravox) there is no TTS service in
    the pipeline and the S2S model covers the gap itself, so we skip it.

    user_id: keys the per-user memory store for remember/recall/forget (single dev
    key until secure-bot-endpoint supplies real per-user ids).
    """

    async def _filler(name):
        if not speak_filler:
            return
        line = _pick_filler(name)
        if line:
            await task.queue_frames([TTSSpeakFrame(line)])

    async def search_places(params):
        q = (params.arguments or {}).get("query", "")
        near = (params.arguments or {}).get("near")
        await _filler("search_places")
        if not _mapbox():
            await params.result_callback({"error": "Maps aren't set up yet (no MAPBOX_TOKEN)."})
            return
        try:
            # center on the user's GPS unless they named a specific area
            clat, clng = (await _geocode(near, near=_here())) if near else _here()

            pins = []
            # PRIMARY: Google Maps (via SerpApi). It anchors to the user's lat/lng (`ll`), so
            # "coffee near me" returns actually-nearby spots — not a same-named place three states
            # over (Mapbox's `proximity` only BIASES; it doesn't restrict). Falls back to Mapbox.
            if _serpapi():
                try:
                    gm = await _get_json("https://serpapi.com/search", params={
                        "engine": "google_maps", "type": "search", "q": q,
                        "ll": f"@{clat},{clng},14z", "api_key": _serpapi(),
                    })
                    for r in gm.get("local_results", [])[:10]:
                        g = r.get("gps_coordinates") or {}
                        lat, lng = g.get("latitude"), g.get("longitude")
                        if lat is None or lng is None:
                            continue
                        pins.append({
                            "name": (r.get("title") or "")[:40],
                            "note": (r.get("type") or r.get("address") or "")[:38],
                            "dist": "", "rating": float(r["rating"]) if r.get("rating") else 0,
                            "x": 0, "y": 0, "lat": lat, "lng": lng,
                        })
                except Exception:
                    logger.exception("google_maps search failed; falling back to Mapbox")

            # FALLBACK: Mapbox Search Box (proximity-biased).
            if not pins:
                data = await _get_json(
                    "https://api.mapbox.com/search/searchbox/v1/forward",
                    params={"q": q, "proximity": f"{clng},{clat}", "limit": "8",
                            "access_token": _mapbox()},
                )
                for f in data.get("features", []):
                    p = f.get("properties", {})
                    c = p.get("coordinates") or {}
                    lat, lng = c.get("latitude"), c.get("longitude")
                    if lat is None or lng is None:
                        gc = f.get("geometry", {}).get("coordinates")
                        if gc:
                            lng, lat = gc[0], gc[1]
                    if lat is None or lng is None:
                        continue
                    pins.append({
                        "name": p.get("name", ""),
                        "note": (p.get("poi_category", [""])[0] if p.get("poi_category")
                                 else p.get("place_formatted", ""))[:38],
                        "dist": "", "rating": 0, "x": 0, "y": 0, "lat": lat, "lng": lng,
                    })

            # SAFETY NET: keep only genuinely-nearby results, nearest first — this is what kills
            # the "across the country" outliers regardless of which provider answered.
            for pn in pins:
                pn["_mi"] = _haversine_mi(clat, clng, pn["lat"], pn["lng"])
            pins = (sorted([p for p in pins if p["_mi"] <= 60], key=lambda p: p["_mi"])
                    or sorted(pins, key=lambda p: p["_mi"]))[:8]
            for pn in pins:
                pn.pop("_mi", None)

            if not pins:
                await params.result_callback({"summary": f"Couldn't find any '{q}' near {near or 'you'}."})
                return

            # Numbered markers baked into the tile (so they sit exactly on the map), auto-fit.
            markers = ",".join(
                f"pin-s-{i + 1}+ef4d2a({p['lng']},{p['lat']})" for i, p in enumerate(pins[:9])
            )
            if len(pins) >= 2 and markers:
                img = (f"https://api.mapbox.com/styles/v1/mapbox/streets-v12/static/{markers}/"
                       f"auto/{MAP_W}x{MAP_H}@2x?padding=64&access_token={_mapbox()}")
            else:
                base = f"{markers}/" if markers else ""
                img = (f"https://api.mapbox.com/styles/v1/mapbox/streets-v12/static/{base}"
                       f"{clng},{clat},{MAP_ZOOM}/{MAP_W}x{MAP_H}@2x?access_token={_mapbox()}")
            await ui.map(pins=pins, query=f"{q} · {near or 'nearby'}", img=img,
                         center={"lat": clat, "lng": clng})
            names = ", ".join(p["name"] for p in pins[:3])
            await params.result_callback({
                "summary": f"Found {len(pins)} spots for '{q}' near {near or 'you'}. On the map now.",
                "count": len(pins), "top": names,
            })
        except Exception as e:
            logger.exception("search_places failed")
            await params.result_callback({"error": f"Map search choked: {e}"})

    async def search_web(params):
        q = (params.arguments or {}).get("query", "")
        await _filler("search_web")
        if not _tavily():
            await params.result_callback({"error": "Web search isn't set up yet (no TAVILY_API_KEY)."})
            return
        try:
            data = await _post_json("https://api.tavily.com/search", {
                "api_key": _tavily(), "query": q,
                "search_depth": "basic", "include_answer": True, "max_results": 5,
            })
            answer = (data.get("answer") or "").strip()
            results = data.get("results", [])
            top = results[0] if results else {}
            blocks = [{"h": q[:80] or "Results"}]
            for para in (answer.split("\n") if answer else []):
                if para.strip():
                    blocks.append({"p": para.strip()})
            if results:
                blocks.append({"h": "Sources"})
                for r in results[:4]:
                    snippet = (r.get("content") or "")[:150]
                    blocks.append({"p": f"{r.get('title', '')} — {snippet}"})
            await ui.web(url=top.get("url", ""), title=top.get("title", q), blocks=blocks)
            await params.result_callback({
                "summary": (answer[:400] or f"Pulled up {len(results)} results for '{q}'. It's on screen."),
            })
        except Exception as e:
            logger.exception("search_web failed")
            await params.result_callback({"error": f"Web search choked: {e}"})

    async def search_images(params):
        q = (params.arguments or {}).get("query", "")
        await _filler("search_images")
        try:
            items = []
            # Google Images via SerpApi returns ~100 results/page — take a browsable batch of the
            # real `original` source images (gstatic `thumbnail` is hotlink-protected → unusable).
            if _serpapi():
                data = await _get_json("https://serpapi.com/search", params={
                    "engine": "google_images", "q": q, "api_key": _serpapi(),
                })
                for r in data.get("images_results", [])[:24]:
                    full = r.get("original")
                    if full:
                        items.append({"cap": (r.get("title") or r.get("source") or q)[:40], "full": full})
            # fall back to Tavily images if SerpApi is unavailable / returns nothing
            if not items and _tavily():
                data = await _post_json("https://api.tavily.com/search", {
                    "api_key": _tavily(), "query": q, "max_results": 10,
                    "include_images": True, "include_image_descriptions": True,
                })
                for im in data.get("images", []):
                    if isinstance(im, dict):
                        items.append({"cap": (im.get("description") or q)[:40], "full": im.get("url")})
                    elif im:
                        items.append({"cap": q[:40], "full": im})
            if not items:
                await params.result_callback({"error": "Image search isn't set up (no SERPAPI_API_KEY / TAVILY_API_KEY)."})
                return
            await ui.images(items=items, query=q)
            await params.result_callback({"summary": f"Pulled up {len(items)} images for '{q}'."})
        except Exception as e:
            logger.exception("search_images failed")
            await params.result_callback({"error": f"Image search choked: {e}"})

    async def search_videos(params):
        q = (params.arguments or {}).get("query", "")
        await _filler("search_videos")
        if not _serpapi():
            await params.result_callback({"error": "Video search isn't set up yet (no SERPAPI_API_KEY)."})
            return
        try:
            # YouTube via SerpApi → a browsable grid; each item plays in the app's embed player.
            data = await _get_json("https://serpapi.com/search", params={
                "engine": "youtube", "search_query": q, "api_key": _serpapi(),
            })
            items = []
            for r in data.get("video_results", [])[:20]:
                link = r.get("link", "")
                # SerpApi gives a dedicated video_id; fall back to parsing the watch/shorts URL.
                vid = r.get("video_id") or ""
                if not vid and "watch?v=" in link:
                    vid = link.split("watch?v=", 1)[1].split("&", 1)[0]
                elif not vid and "/shorts/" in link:
                    vid = link.split("/shorts/", 1)[1].split("?", 1)[0]
                if not vid:
                    continue  # need the id to build the embed player
                # Clean, always-public thumbnail (SerpApi's static thumb is a signed,
                # hotlink-protected i.ytimg URL that fails to fetch from the app).
                thumb = f"https://i.ytimg.com/vi/{vid}/hqdefault.jpg"
                ch = r.get("channel")
                channel = ch.get("name", "") if isinstance(ch, dict) else (ch or "")
                items.append({
                    "title": (r.get("title") or q)[:80],
                    "channel": (channel or "")[:40],
                    "dur": r.get("length") or "",
                    "url": link or f"https://www.youtube.com/watch?v={vid}",
                    "id": vid,
                    "thumb": thumb,
                })
            if not items:
                await params.result_callback({"error": f"No videos found for '{q}'."})
                return
            await ui.videos(items=items, query=q)
            await params.result_callback({"summary": f"Found {len(items)} videos for '{q}'."})
        except Exception as e:
            logger.exception("search_videos failed")
            await params.result_callback({"error": f"Video search choked: {e}"})

    async def end_call(params):
        # Drop the session (the client plays Nova's sign-off first, then disconnects).
        await ui.end_call()
        await params.result_callback({"summary": "Ended the call."})

    async def get_weather(params):
        loc = (params.arguments or {}).get("location")
        await _filler("get_weather")
        try:
            label = None
            if loc:
                # Open-Meteo geocoding (keyless) — no Mapbox dependency for weather.
                geo = await _get_json("https://geocoding-api.open-meteo.com/v1/search",
                                      params={"name": loc, "count": 1})
                res = (geo.get("results") or [])
                if res:
                    lat, lng = res[0]["latitude"], res[0]["longitude"]
                    parts = [res[0].get("name"), res[0].get("admin1")]
                    label = ", ".join([p for p in parts if p]) or loc
                else:
                    lat, lng = _here()
                    label = loc
            else:
                lat, lng = _here()

            data = await _get_json("https://api.open-meteo.com/v1/forecast", params={
                "latitude": lat, "longitude": lng,
                "current": "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day",
                "hourly": "temperature_2m,weather_code",
                "daily": "weather_code,temperature_2m_max,temperature_2m_min",
                "temperature_unit": "fahrenheit", "wind_speed_unit": "mph",
                "timezone": "auto", "forecast_days": 7,
            })
            if label is None:  # no place named → use the timezone's city as a friendly label
                tz = data.get("timezone", "")
                label = tz.split("/")[-1].replace("_", " ") if "/" in tz else "Nearby"

            cur = data.get("current", {})
            is_day = bool(cur.get("is_day", 1))
            cond, cid = _wx(cur.get("weather_code", 0), is_day)
            temp = round(cur.get("temperature_2m", 0))
            feels = round(cur.get("apparent_temperature", temp))
            humidity = round(cur.get("relative_humidity_2m", 0))
            wind = round(cur.get("wind_speed_10m", 0))

            daily = data.get("daily", {})
            dt_ = daily.get("time", [])
            dmax = daily.get("temperature_2m_max", [])
            dmin = daily.get("temperature_2m_min", [])
            dcode = daily.get("weather_code", [])
            hi = round(dmax[0]) if dmax else temp
            lo = round(dmin[0]) if dmin else temp

            hly = data.get("hourly", {})
            ht = hly.get("time", [])
            hp = hly.get("temperature_2m", [])
            hc = hly.get("weather_code", [])
            start = ht.index(cur.get("time")) if cur.get("time") in ht else 0
            hours = []
            for i in range(start, min(start + 12, len(ht))):
                _, hcid = _wx(hc[i] if i < len(hc) else 0, True)
                hours.append({"t": _fmt_hour(ht[i]), "temp": f"{round(hp[i])}°", "icon": hcid})

            days = []
            for i in range(min(7, len(dt_))):
                _, dcid = _wx(dcode[i] if i < len(dcode) else 0, True)
                days.append({"d": "Today" if i == 0 else _fmt_day(dt_[i]),
                             "hi": f"{round(dmax[i])}°", "lo": f"{round(dmin[i])}°", "icon": dcid})

            await ui.weather(
                place=label, temp=f"{temp}°", cond=cond, icon=cid, feels=f"{feels}°",
                hi=f"{hi}°", lo=f"{lo}°", humidity=f"{humidity}%", wind=f"{wind} mph",
                is_day=is_day, hours=hours, days=days,
            )
            await params.result_callback(
                {"summary": f"It's {temp}° and {cond.lower()} in {label}, high {hi}° / low {lo}°."})
        except Exception as e:
            logger.exception("get_weather failed")
            await params.result_callback({"error": f"Weather lookup choked: {e}"})

    async def search_products(params):
        q = (params.arguments or {}).get("query", "")
        await _filler("search_products")
        if not _serpapi():
            await params.result_callback({"error": "Shopping isn't set up yet (no SERPAPI_API_KEY)."})
            return
        try:
            data = await _get_json("https://serpapi.com/search", params={
                "engine": "google_shopping", "q": q,
                "location": "Seattle, Washington, United States",
                "api_key": _serpapi(), "num": "10",
            })
            items = []
            for r in data.get("shopping_results", [])[:8]:
                items.append({
                    "name": (r.get("title") or "")[:60],
                    "price": r.get("price") or "",
                    "store": (r.get("source") or "")[:12],
                    "rating": str(r.get("rating")) if r.get("rating") else "",
                    "ships": (r.get("delivery") or "")[:16],
                    "link": r.get("product_link") or r.get("link") or "",  # tap → store page
                    # serpapi_thumbnail is a SerpApi-proxied image (fetchable); the raw
                    # `thumbnail` is a gstatic URL that hotlink-protects to a 1x1 GIF for
                    # non-browser user agents, so the client could never load it.
                    "thumb": r.get("serpapi_thumbnail") or r.get("thumbnail") or "",
                })
            await ui.products(items=items, query=q)
            cheapest = items[0]["price"] if items else "n/a"
            await params.result_callback({
                "summary": f"Found {len(items)} results for '{q}', starting around {cheapest}. On screen.",
            })
        except Exception as e:
            logger.exception("search_products failed")
            await params.result_callback({"error": f"Shopping search choked: {e}"})

    async def get_directions(params):
        a = params.arguments or {}
        dest = a.get("destination", "")
        mode = a.get("mode", "driving")
        origin = a.get("origin")
        await _filler("get_directions")
        if not _mapbox():
            await params.result_callback({"error": "Maps aren't set up yet (no MAPBOX_TOKEN)."})
            return
        try:
            # start from the user's GPS unless they named an origin
            o = (await _geocode(origin, near=_here())) if origin else _here()
            d = await _geocode(dest, near=o)   # bias the destination to near the start
            profile = {"driving": "driving-traffic", "walking": "walking",
                       "cycling": "cycling", "transit": "driving-traffic"}.get(mode, "driving-traffic")
            data = await _get_json(
                # "simplified" keeps the polyline (and thus the static-map URL) short enough to load
                f"https://api.mapbox.com/directions/v5/mapbox/{profile}/{o[1]},{o[0]};{d[1]},{d[0]}",
                params={"access_token": _mapbox(), "overview": "simplified",
                        "geometries": "polyline", "annotations": "duration"},
            )
            routes = data.get("routes", [])
            if not routes:
                await params.result_callback({"error": f"Couldn't find a route to {dest}."})
                return
            route = routes[0]
            mins = round(route["duration"] / 60)
            miles = round(route["distance"] / 1609.34, 1)
            # Bake the route AND both markers into ONE image, then let Mapbox `auto`-fit the
            # bounding box (with padding). Because start/dest pins and the route line are drawn
            # by Mapbox in the same projection, they line up EXACTLY — no client-side pin overlay
            # to drift out of register (which is what looked "off" before). Green = start, orange = dest.
            overlay = (f"path-5+ef4d2a-0.9({quote(route['geometry'], safe='')}),"
                       f"pin-s+18a558({o[1]},{o[0]}),pin-l+ef4d2a({d[1]},{d[0]})")
            img = (f"https://api.mapbox.com/styles/v1/mapbox/streets-v12/static/{overlay}/"
                   f"auto/{MAP_W}x{MAP_H}@2x?padding=64&access_token={_mapbox()}")
            # Pins carry NO x/y now (markers are in the image); they drive the card list + taps.
            pins = [
                {"name": f"Start · {origin or 'You'}", "note": "", "dist": "", "rating": 0,
                 "x": 0, "y": 0, "lat": o[0], "lng": o[1]},
                {"name": dest, "note": f"{mins} min · {miles} mi", "dist": f"{miles} mi", "rating": 0,
                 "x": 0, "y": 0, "lat": d[0], "lng": d[1]},
            ]
            await ui.map(pins=pins, query=f"{origin or 'You'} → {dest}", img=img,
                         route={"eta_min": mins, "miles": miles, "mode": mode,
                                "dest_lat": d[0], "dest_lng": d[1]})
            await params.result_callback({
                "summary": f"{mins} minutes, {miles} miles by {mode} to {dest}. Route's on the map — "
                           f"say the word and I'll start navigation.",
            })
        except Exception as e:
            logger.exception("get_directions failed")
            await params.result_callback({"error": f"Directions choked: {e}"})

    async def start_navigation(params):
        a = params.arguments or {}
        dest = a.get("destination", "")
        mode = a.get("mode", "driving")
        await _filler("start_navigation")
        try:
            d = await _geocode(dest) if dest else None
            url = (f"https://www.google.com/maps/dir/?api=1&destination={d[0]},{d[1]}&travelmode={mode}"
                   if d else None)
            # Client opens `url` in the phone's nav app (Android intent / iOS openURL).
            await ui.navigate(url=url, destination=dest,
                              lat=(d[0] if d else None), lng=(d[1] if d else None), mode=mode)
            await params.result_callback({
                "summary": f"Firing up navigation to {dest}. Eyes on the road, champ.",
            })
        except Exception as e:
            logger.exception("start_navigation failed")
            await params.result_callback({"error": f"Couldn't start navigation: {e}"})

    async def find_specialist(params):
        # Kaira's tool: find nearby clinicians/clinics. Voice-only persona → returns a spoken-friendly
        # summary (names, neighborhoods, distance). Google Maps (via SerpApi), anchored to the
        # patient's lat/lng, gives the best clinic coverage; Mapbox is the fallback. Distance-filtered
        # so a same-named practice in another state can't sneak into a medical referral.
        a = params.arguments or {}
        specialty = (a.get("specialty") or a.get("query") or "").strip()
        near = a.get("near")
        if not (_serpapi() or _mapbox()):
            await params.result_callback({"error": "I can't look up clinics right now — maps aren't configured."})
            return
        try:
            clat, clng = (await _geocode(near, near=_here())) if near else _here()
            found = []
            # PRIMARY: Google Maps, anchored to the patient (`ll`).
            if _serpapi():
                try:
                    gm = await _get_json("https://serpapi.com/search", params={
                        "engine": "google_maps", "type": "search", "q": specialty or "doctor",
                        "ll": f"@{clat},{clng},13z", "api_key": _serpapi(),
                    })
                    for r in gm.get("local_results", [])[:12]:
                        g = r.get("gps_coordinates") or {}
                        lat, lng = g.get("latitude"), g.get("longitude")
                        if lat is None or lng is None:
                            continue
                        addr = r.get("address") or ""
                        city = addr.split(",")[1].strip() if "," in addr else (r.get("type") or "")
                        found.append({
                            "name": (r.get("title") or "")[:60], "area": city,
                            "dist_mi": round(_haversine_mi(clat, clng, lat, lng), 1),
                            "address": addr,
                        })
                except Exception:
                    logger.exception("google_maps clinic search failed; falling back to Mapbox")
            # FALLBACK: Mapbox Search Box.
            if not found and _mapbox():
                data = await _get_json(
                    "https://api.mapbox.com/search/searchbox/v1/forward",
                    params={"q": specialty or "doctor", "proximity": f"{clng},{clat}",
                            "limit": "10", "country": "US", "access_token": _mapbox()},
                )
                for f in data.get("features", []):
                    p = f.get("properties", {})
                    name = p.get("name", "")
                    if not name:
                        continue
                    c = p.get("coordinates") or {}
                    lat, lng = c.get("latitude"), c.get("longitude")
                    if lat is None or lng is None:
                        g = f.get("geometry", {}).get("coordinates")
                        if g:
                            lng, lat = g[0], g[1]
                    if lat is None or lng is None:
                        continue
                    found.append({
                        "name": name,
                        "area": (p.get("place_formatted") or "").split(",")[0].strip(),
                        "dist_mi": round(_haversine_mi(clat, clng, lat, lng), 1),
                        "address": p.get("full_address") or p.get("place_formatted") or "",
                    })
            # keep only genuinely-local results (drops same-name clinics states away), nearest first
            picks = sorted([r for r in found if r["dist_mi"] is not None and r["dist_mi"] <= 75],
                           key=lambda r: r["dist_mi"]) or sorted(found, key=lambda r: r.get("dist_mi") or 1e9)
            if not picks:
                await params.result_callback({
                    "summary": f"I couldn't find any {specialty or 'clinics'} near {near or 'you'} — it "
                               "may help to widen the area or check your insurance's provider directory.",
                })
                return

            def _fmt(r):
                area = f" in {r['area']}" if r["area"] else ""
                dm = f", about {r['dist_mi']} miles away" if r.get("dist_mi") is not None else ""
                return f"{r['name']}{area}{dm}"
            listed = "; ".join(_fmt(r) for r in picks[:3])
            await params.result_callback({
                "summary": f"I found {len(picks)} option{'s' if len(picks) != 1 else ''} near "
                           f"{near or 'you'}: {listed}. Want the address for any of them?",
                "results": picks[:6],
            })
        except Exception as e:
            logger.exception("find_specialist failed")
            await params.result_callback({"error": f"I couldn't complete the specialist search: {e}"})

    # ---- memory: remember / recall / forget (local + instant → no filler) ----
    async def remember(params):
        args = params.arguments or {}
        fact = (args.get("fact") or "").strip()
        category = (args.get("category") or "preference").strip().lower()
        saved = memory.add(user_id, fact, category)
        if not saved:
            await params.result_callback({"error": "There was nothing to remember."})
            return
        await params.result_callback({"summary": f"Noted and saved: {saved['text']}"})

    async def recall(params):
        q = (params.arguments or {}).get("query", "")
        hits = memory.query(user_id, q)
        if not hits:
            miss = f"Nothing stored about '{q}'." if q else "Nothing stored on that yet."
            await params.result_callback({"summary": miss})
            return
        await params.result_callback({"summary": "; ".join(f["text"] for f in hits)})

    async def forget(params):
        match = (params.arguments or {}).get("fact", "")
        removed = memory.remove(user_id, match)
        if not removed:
            await params.result_callback({"summary": f"Nothing matching '{match}' to forget."})
            return
        await params.result_callback({"summary": "Forgotten: " + "; ".join(f["text"] for f in removed)})

    llm.register_function("search_places", search_places)
    llm.register_function("search_web", search_web)
    llm.register_function("search_images", search_images)
    llm.register_function("search_videos", search_videos)
    llm.register_function("search_products", search_products)
    llm.register_function("get_weather", get_weather)
    llm.register_function("end_call", end_call)
    llm.register_function("get_directions", get_directions)
    llm.register_function("start_navigation", start_navigation)
    llm.register_function("find_specialist", find_specialist)
    llm.register_function("remember", remember)
    llm.register_function("recall", recall)
    llm.register_function("forget", forget)
    logger.info("Registered tools: search_places, search_web, search_images, search_videos, "
                "search_products, get_weather, get_directions, start_navigation, find_specialist")


# ---------------------------------------------------------------- schemas (pure data)
NOVA_TOOLS = ToolsSchema(standard_tools=[
    FunctionSchema(
        name="search_places",
        description=("Show places on a MAP — coffee shops, restaurants, parks, stores, landmarks. "
                     "Use whenever the user wants to find or see somewhere to go."),
        properties={
            "query": {"type": "string", "description": "What to look for, e.g. 'coffee', 'ramen', 'record stores'."},
            "near": {"type": "string", "description": "Area to search near, e.g. 'Capitol Hill, Seattle'. Defaults to Seattle."},
        },
        required=["query"],
    ),
    FunctionSchema(
        name="search_web",
        description=("Pull up a WEB reader with the answer to a question or topic. Use for facts, how-tos, "
                     "news, explanations — anything the user would want to read or verify."),
        properties={"query": {"type": "string", "description": "The search query or question."}},
        required=["query"],
    ),
    FunctionSchema(
        name="search_images",
        description="Show a grid of IMAGES for a subject. Use when the user wants to SEE what something looks like.",
        properties={"query": {"type": "string", "description": "What to show pictures of."}},
        required=["query"],
    ),
    FunctionSchema(
        name="search_videos",
        description=("Show a grid of VIDEOS (YouTube) the user can play. Use when they want to WATCH something — "
                     "a how-to, a clip, a trailer, a music video, a talk. Once one is playing you can SEE the "
                     "video and answer questions about what's happening in it."),
        properties={"query": {"type": "string", "description": "What videos to find, e.g. 'how to poach an egg'."}},
        required=["query"],
    ),
    FunctionSchema(
        name="search_products",
        description=("Show SHOPPING results with prices and stores. Use when the user wants to buy something or "
                     "compare products/prices."),
        properties={"query": {"type": "string", "description": "The product to shop for."}},
        required=["query"],
    ),
    FunctionSchema(
        name="get_weather",
        description=("Show the WEATHER — current conditions plus an hourly and 7-day forecast. Use when the user "
                     "asks about the weather, temperature, rain/snow, or what to wear / whether to bring a jacket."),
        properties={
            "location": {"type": "string",
                         "description": "City or place to check, e.g. 'Portland' or 'Ballard'. Omit for the user's current location."},
        },
        required=[],
    ),
    FunctionSchema(
        name="end_call",
        description=("Hang up / end the voice call. Call this ONLY when the user clearly wants to stop talking — "
                     "'bye', 'goodbye', 'talk later', 'that's all', 'I'm done', 'we're done', 'end the call', "
                     "'catch you later'. Say a SHORT sign-off first, then call this in the SAME turn. Do NOT call "
                     "it just because the conversation paused."),
        properties={},
        required=[],
    ),
    FunctionSchema(
        name="get_directions",
        description=("Plan a route to a destination and show it on the MAP with ETA and distance. Use when the "
                     "user wants directions or to know how far / how long somewhere is. Does NOT start "
                     "turn-by-turn — call start_navigation for that."),
        properties={
            "destination": {"type": "string", "description": "Where to go, e.g. 'Pike Place Market'."},
            "mode": {"type": "string", "enum": ["driving", "walking", "cycling", "transit"],
                     "description": "Travel mode. Defaults to driving."},
            "origin": {"type": "string", "description": "Starting point. Defaults to the current area (Seattle)."},
        },
        required=["destination"],
    ),
    FunctionSchema(
        name="start_navigation",
        description=("Hand off to the phone's navigation app for live turn-by-turn to a destination. Call this "
                     "only when the user clearly wants to GO / start driving there now."),
        properties={
            "destination": {"type": "string", "description": "Where to navigate to."},
            "mode": {"type": "string", "enum": ["driving", "walking", "cycling", "transit"],
                     "description": "Travel mode. Defaults to driving."},
        },
        required=["destination"],
    ),
    FunctionSchema(
        name="remember",
        description=("Save a lasting fact about the user to your memory so you still know it in future "
                     "conversations — their name, tastes/preferences, the people and places in their life, "
                     "routines, ongoing projects. Use it PROACTIVELY the moment the user shares something "
                     "stable and worth keeping, without being asked. Confirm FIRST before saving anything "
                     "sensitive (health, finances, relationships, a precise home address)."),
        properties={
            "fact": {"type": "string", "description": "The fact to remember, e.g. 'Lives in Ballard' or 'Hates cilantro'."},
            "category": {"type": "string", "enum": list(memory.CATEGORIES),
                         "description": "preference | person | place | routine | project."},
        },
        required=["fact"],
    ),
    FunctionSchema(
        name="recall",
        description=("Look something up in your memory of the user when you need a detail you might have stored "
                     "before — a name, a preference, an address, a person in their life."),
        properties={"query": {"type": "string", "description": "What to look up, e.g. 'coffee order' or 'sister'."}},
        required=["query"],
    ),
    FunctionSchema(
        name="forget",
        description=("Remove a fact from your memory when the user asks you to forget it, or corrects something "
                     "that has changed."),
        properties={"fact": {"type": "string", "description": "The fact (or a phrase from it) to remove."}},
        required=["fact"],
    ),
])


# Kaira's tool set — just the specialist/clinic lookup. Her prompt runs the consultation; this
# lets her point the patient to who to see and where to go, as part of the triage handoff.
KAIRA_TOOLS = ToolsSchema(standard_tools=[
    FunctionSchema(
        name="find_specialist",
        description=("Find nearby clinicians or clinics for the patient — a specialist, physical "
                     "therapist, urgent care, GP, and so on. Use it when they want to know WHO to "
                     "see or WHERE to go. Ask for their area first if you don't already know it."),
        properties={
            "specialty": {"type": "string",
                          "description": "The kind of clinician or clinic, e.g. 'orthopedic specialist', "
                                         "'physical therapist', 'urgent care', 'cardiologist', 'family doctor'."},
            "near": {"type": "string",
                     "description": "Area to search near, e.g. 'Everett, WA'. Defaults to the patient's location."},
        },
        required=["specialty"],
    ),
    FunctionSchema(
        name="get_directions",
        description=("Show a route on the MAP to a clinic or address, with ETA and distance. Use "
                     "after finding a clinic when the patient wants to know how to get there or how "
                     "far it is. Does NOT start turn-by-turn — use start_navigation for that."),
        properties={
            "destination": {"type": "string",
                            "description": "The clinic or place to route to, e.g. 'EvergreenHealth Urgent Care, Mill Creek'."},
            "mode": {"type": "string", "enum": ["driving", "walking", "cycling", "transit"],
                     "description": "Travel mode. Defaults to driving."},
        },
        required=["destination"],
    ),
    FunctionSchema(
        name="start_navigation",
        description=("Open the phone's maps app for live turn-by-turn directions to a clinic. Use "
                     "only when the patient clearly wants to GO there now."),
        properties={
            "destination": {"type": "string", "description": "The clinic or place to navigate to."},
            "mode": {"type": "string", "enum": ["driving", "walking", "cycling", "transit"],
                     "description": "Travel mode. Defaults to driving."},
        },
        required=["destination"],
    ),
])
