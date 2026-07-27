"""VOID_AI UI-control bridge — lets Nova drive the client's dynamic view surface.

Messages ride the existing RTVI 'chat' WebRTC data channel as transport
app-messages, wrapped in the envelope the Slint client already parses:

    {"label": "rtvi-ai", "type": "server-message", "data": {"op": <op>, ...}}

Using OutputTransportMessageUrgentFrame (not RTVIServerMessageFrame) means this
works with the minimal webrtc-rs client, which does NOT do the full RTVI
client-ready handshake an RTVIProcessor would wait for.

See UI_CONTRACT.md (kaira-slint repo) for the full message spec. The tool layer
(Phase 3) calls these methods — each search tool populates a view AND returns a
short text summary for Nova to react to.
"""

from typing import Any, Optional

from pipecat.frames.frames import OutputTransportMessageUrgentFrame

_LABEL = "rtvi-ai"


class UiBridge:
    """Sends UI-control messages to the client over the pipeline's transport."""

    def __init__(self, task):
        self._task = task

    async def _send(self, op: str, **fields: Any) -> None:
        # Drop None fields so payloads stay small (matters on 3G). False/0 are kept.
        data = {"op": op}
        data.update({k: v for k, v in fields.items() if v is not None})
        await self._task.queue_frames(
            [
                OutputTransportMessageUrgentFrame(
                    message={"label": _LABEL, "type": "server-message", "data": data}
                )
            ]
        )

    # --- view control (no content) ---
    async def open_view(self, view: str) -> None:
        await self._send("open_view", view=view)

    async def close_view(self, view: str) -> None:
        await self._send("close_view", view=view)

    async def focus_view(self, view: str) -> None:
        await self._send("focus_view", view=view)

    async def collapse(self, on: bool = True) -> None:
        await self._send("collapse", on=on)

    async def fullscreen(self, on: bool = True) -> None:
        await self._send("fullscreen", on=on)

    # --- content (each opens + switches its view; collapse=True also collapses Nova) ---
    async def images(
        self, items: list, query: Optional[str] = None,
        msg: Optional[str] = None, collapse: bool = True,
    ) -> None:
        await self._send("images", items=items, query=query, msg=msg, collapse=collapse)

    async def map(
        self, pins: list, query: Optional[str] = None, img: Optional[str] = None,
        center: Optional[dict] = None, route: Optional[dict] = None, collapse: bool = True,
    ) -> None:
        # img = static map URL (client fetches + shows it); pins carry normalized x/y overlays.
        await self._send("map", pins=pins, query=query, img=img, center=center, route=route, collapse=collapse)

    async def products(
        self, items: list, query: Optional[str] = None, collapse: bool = True,
    ) -> None:
        await self._send("products", items=items, query=query, collapse=collapse)

    async def web(
        self, url: str, title: Optional[str] = None, blocks: Optional[list] = None,
        tabs: Optional[list] = None, active: Optional[int] = None, collapse: bool = False,
    ) -> None:
        await self._send("web", url=url, title=title, blocks=blocks, tabs=tabs, active=active, collapse=collapse)

    async def doc(
        self, name: str, blocks: Optional[list] = None,
        page: Optional[str] = None, collapse: bool = False,
    ) -> None:
        await self._send("doc", name=name, blocks=blocks, page=page, collapse=collapse)

    # --- hand off to the phone's native nav app (plan-then-handoff; see UI_CONTRACT.md) ---
    async def navigate(
        self, url: Optional[str] = None, destination: Optional[str] = None,
        lat: Optional[float] = None, lng: Optional[float] = None, mode: Optional[str] = None,
    ) -> None:
        # Client opens `url` (a maps deep link) via an Android intent / iOS openURL.
        await self._send("navigate", url=url, destination=destination, lat=lat, lng=lng, mode=mode)
