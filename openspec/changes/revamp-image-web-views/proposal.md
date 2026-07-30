# Revamp the Images + Web views into a navigable, persistent history

## Why

The view surface is a **live mirror of Nova's latest action** — every UI-control op *replaces*
the view's content — so a second image search wipes the first and there's no way back. The Web
view is worse: it only shows Nova's *summary blocks*, not the real page, so it "might as well be a
card in the chat." The user wants both views rethought: keep history, make Web actually useful,
and let the chat act as the index.

## What changes

Both views become a **tabbed history** (the Web view already has the tab model — Images gets it too):

1. **Images tabbed history** — each `images` search is a tab; new searches **append** (don't
   replace); a tab strip flips between them; capped to a limit.
2. **Web real reader** — each Web tab loads the **actual URL in a WKWebView** (mini-browser,
   reusing the maps webview infra); Nova's summary becomes a collapsible "Nova's take."
3. **Chat → tab jump** — tapping an images/web card in the chat switches to that view and selects
   its tab (chat is the index).
4. **Cross-session persistence** — batches (image URLs+captions+query; web url+title+summary) are
   stored to the app's files dir and restored on launch, capped to the last N.
5. **Video search + view + vision** — a new `search_videos` tool + Video view: a results grid
   (like Images) whose items PLAY in a WKWebView (like the browser), with the same tabbed history.
   Video vision (duplex): grab the current player frame → send it as an image so Nova can answer
   "what's happening in this?" — reusing the image-vision path.

## Impact

- `kaira-slint`: new `IMAGES_TABS` history (mirrors `WEB_TABS`); image tab-strip UI; Web view
  swaps summary-blocks for a WKWebView reader; chat cards gain tap-to-jump; a small on-disk
  history store. iOS-first (WKWebView), Android reader is a follow-up.
- No bot/server change — the `images`/`web` UI-control ops are unchanged; the client just keeps
  and navigates history instead of discarding it.
