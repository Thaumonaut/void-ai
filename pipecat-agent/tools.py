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

import math
import os
import random
from urllib.parse import quote

import aiohttp
from loguru import logger

from pipecat.adapters.schemas.function_schema import FunctionSchema
from pipecat.adapters.schemas.tools_schema import ToolsSchema
from pipecat.frames.frames import TTSSpeakFrame

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


async def _geocode(place, near=None):
    """Place/POI name -> (lat, lng); Seattle on failure.

    Uses Mapbox's **Search Box** API first — it's POI-aware, so "Mukilteo ferry terminal"
    resolves to the actual terminal instead of the v6 address geocoder's nonsense (which
    sent it to LA). Falls back to v6 for plain addresses. `near=(lat,lng)` biases both to
    that region (proximity) so a same-named place three states over doesn't win.
    """
    prox = f"{near[1]},{near[0]}" if near else f"{SEATTLE[1]},{SEATTLE[0]}"
    # 1) Search Box — best for landmarks / businesses / transit stops.
    try:
        data = await _get_json(
            "https://api.mapbox.com/search/searchbox/v1/forward",
            params={"q": place, "limit": "1", "country": "US",
                    "proximity": prox, "access_token": _mapbox()},
        )
        feats = data.get("features") or []
        if feats:
            lng, lat = feats[0]["geometry"]["coordinates"]
            return lat, lng
    except Exception:
        pass
    # 2) Fall back to the v6 address geocoder.
    try:
        data = await _get_json(
            "https://api.mapbox.com/search/geocode/v6/forward",
            params={"q": place, "limit": "1", "country": "us",
                    "proximity": prox, "access_token": _mapbox()},
        )
        lng, lat = data["features"][0]["geometry"]["coordinates"]
        return lat, lng
    except Exception:
        return SEATTLE


# ------------------------------------------------------------------------- the tools
def register_tool_handlers(llm, ui, task, speak_filler: bool = True):
    """Bind the handlers to this run's UiBridge + task and register them on the LLM.

    speak_filler: cascade mode pushes a TTSSpeakFrame filler line to cover the
    tool-fetch gap. In full-duplex/S2S mode (Ultravox) there is no TTS service in
    the pipeline and the S2S model covers the gap itself, so we skip it.
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
            data = await _get_json(
                "https://api.mapbox.com/search/searchbox/v1/forward",
                params={"q": q, "proximity": f"{clng},{clat}", "limit": "6",
                        "access_token": _mapbox()},
            )
            pins = []
            for f in data.get("features", []):
                p = f.get("properties", {})
                c = p.get("coordinates") or {}
                lat, lng = c.get("latitude"), c.get("longitude")
                if lat is None or lng is None:
                    g = f.get("geometry", {}).get("coordinates")
                    if g:
                        lng, lat = g[0], g[1]
                if lat is None or lng is None:
                    continue
                pins.append({
                    "name": p.get("name", ""),
                    "note": (p.get("poi_category", [""])[0] if p.get("poi_category")
                             else p.get("place_formatted", ""))[:38],
                    "dist": "", "rating": 0, "x": 0, "y": 0,
                    "lat": lat, "lng": lng,  # tap the place → open in Maps
                })
            # Numbered markers baked into the tile (so they always sit exactly on the map),
            # auto-fit to frame them all. The card list below carries the same order.
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
                "summary": f"Found {len(pins)} spots for '{q}' near {near}. On the map now.",
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

    llm.register_function("search_places", search_places)
    llm.register_function("search_web", search_web)
    llm.register_function("search_images", search_images)
    llm.register_function("search_products", search_products)
    llm.register_function("get_directions", get_directions)
    llm.register_function("start_navigation", start_navigation)
    logger.info("Registered Nova tools: search_places, search_web, search_images, "
                "search_products, get_directions, start_navigation")


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
        name="search_products",
        description=("Show SHOPPING results with prices and stores. Use when the user wants to buy something or "
                     "compare products/prices."),
        properties={"query": {"type": "string", "description": "The product to shop for."}},
        required=["query"],
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
])
