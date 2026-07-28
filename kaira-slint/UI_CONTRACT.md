# VOID_AI ⟷ Nova — UI-control contract

How Nova (the Pipecat bot) drives the client's dynamic view surface, and how the
client reports manual user actions back for context.

**Transport.** The existing RTVI `chat` WebRTC data channel (ordered, bidirectional).
Every message is JSON in the RTVI envelope:

```json
{ "label": "rtvi-ai", "type": "<type>", "data": { ... } }
```

The client already routes by `label == "rtvi-ai"` then `type`. We add two `type`s:
`server-message` (Nova → client, UI control) and `client-message` (client → Nova, context).
Existing `user-transcription` / `bot-transcription` are unchanged.

**Views** are identified by these ids: `chat` · `images` · `web` · `map` · `products` · `docs` ·
`videos` · `weather`. (`chat` is permanent and cannot be closed.)

Keep payloads small — the target runs on spotty 3G. Images/thumbnails are URLs the
client fetches lazily (placeholder while loading); `web`/`doc` blocks are pre-simplified
server-side, never raw HTML.

---

## Nova → client  (`type: "server-message"`)

`data.op` selects the action.

### View control (no content)
| `op` | fields | effect |
|---|---|---|
| `open_view` | `view` | ensure the view is open, switch to it |
| `close_view` | `view` | close the view (ignored for `chat`) |
| `focus_view` | `view` | switch to an already-open view (no-op if closed) |
| `collapse` | `on: bool` | collapse (`true`) / expand (`false`) Nova |
| `fullscreen` | `on: bool` | enter / exit full-screen view mode |
| `navigate` | `url`, `destination?`, `lat?`, `lng?`, `mode?` | open `url` (a maps deep link) in the phone's nav app — plan-then-handoff turn-by-turn (Android `ACTION_VIEW` intent / iOS openURL) |

### Content (each opens + switches its view; `collapse: true` also collapses Nova)
```jsonc
{ "op":"images",   "query?":"…", "msg?":"…", "collapse?":true,
  "items":[ { "full":"https://…", "thumb?":"https://…", "cap?":"stairs_01.jpg" } ] }

{ "op":"map",      "query?":"coffee near Kemang", "center?":{"lat":-6.26,"lng":106.81},
  "collapse?":true,
  "pins":[ { "name":"Titik Temu", "note?":"Specialty", "dist?":"280 m",
             "rating?":4.7, "lat?":-6.26, "lng?":106.81, "kind?":"cafe" } ],
  "route?":{ "eta":"12 min", "dist":"3.1 km", "steps?":["Head north…"] } }

{ "op":"products", "query?":"concrete planter", "collapse?":true,
  "items":[ { "name":"Raw Concrete Planter", "price":"$38", "store":"ETSY",
              "thumb?":"https://…", "url?":"https://…" } ] }

{ "op":"web",      "url":"https://…", "title?":"…", "collapse?":false,
  "blocks":[ {"h":"Heading"}, {"p":"Paragraph…"}, {"img":"https://…"} ],
  "tabs?":[ {"title":"…","url":"…"} ], "active?":0 }

{ "op":"doc",      "name":"Report.pdf", "page?":"1 / 12", "collapse?":false,
  "blocks":[ {"h2":"Title"}, {"p":"…"}, {"rule":true} ] }

{ "op":"weather",  "place":"Seattle, WA", "temp":"62°", "cond":"Partly cloudy",
  "icon":"partly", "feels":"60°", "hi":"68°", "lo":"54°", "humidity":"72%",
  "wind":"8 mph", "is_day":true, "collapse?":true,
  // all values are pre-formatted display strings; `icon` is a condition id:
  //   clear-day · clear-night · partly · cloudy · rain · snow · storm · fog
  "hours":[ { "t":"3 PM", "temp":"63°", "icon":"partly" } ],
  "days":[  { "d":"Today", "hi":"68°", "lo":"54°", "icon":"partly" } ] }
```

Sent from the bot via `UiBridge` (see `ui.py`), e.g. `await ui.map(pins, query=…)`.

---

## client → Nova  (`type: "client-message"`)  — defined; wired in a later phase

So Nova has context when the **user** drives the UI manually.
```jsonc
{ "op":"view_changed", "view":"map" }
{ "op":"view_opened",  "view":"products" }
{ "op":"view_closed",  "view":"web" }
{ "op":"clicked",      "kind":"product"|"place"|"image"|"link", "label?":"…", "url?":"…" }
{ "op":"interrupt" }              // user tapped the waveform (VAD already handles barge-in)
```
Bot side handles these via `transport.event_handler("on_app_message")` → inject a short
`developer`/`system` context line into `LLMContext`. Not implemented in Phase 1.

---

## Status

- **Phase 1 (this):** contract + plumbing. Bot can send every `server-message` op
  (`ui.py::UiBridge`, queued as `OutputTransportMessageUrgentFrame`). Client parses
  `server-message` and hands `data` to a `UiControlCb` (currently logs; Phase 2 drives
  the Slint model).
- **Phase 2:** Slint UI consumes `UiControlCb` and renders the views.
- **Phase 3:** tool layer calls `UiBridge` methods with real data.
- **Later:** `client-message` handling for user-driven context.
