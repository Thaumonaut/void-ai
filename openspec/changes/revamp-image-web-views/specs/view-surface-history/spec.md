## ADDED Requirements

### Requirement: Image search history is navigable
Each `images` UI-control op SHALL append a search to a capped in-session history instead of replacing the current results, and the Images view SHALL let the user flip between past searches.

#### Scenario: A second image search keeps the first
- **WHEN** Nova runs a second `images` search while a first is showing
- **THEN** both searches are retained as separate tabs and the newest is shown, with a chip strip to return to the earlier one

#### Scenario: History is capped
- **WHEN** more than the image-history limit of searches accumulate in a session
- **THEN** the oldest searches are evicted so the history stays bounded

### Requirement: Web view is a real in-app reader
On iOS the Web view SHALL load the active tab's actual URL in a WKWebView (a real, scrollable page), with Nova's summary blocks available on demand as "Nova's take". Platforms without the native reader SHALL fall back to the summary-block reader.

#### Scenario: Opening a web result shows the live page
- **WHEN** Nova returns a `web` result on iOS
- **THEN** the real page loads in an in-app webview positioned over the content region

#### Scenario: Nova's take toggles the summary
- **WHEN** the user taps "Nova's take"
- **THEN** the live page is tucked away and Nova's summary blocks are shown, and tapping again restores the page

#### Scenario: Non-iOS fallback
- **WHEN** the platform has no native web reader
- **THEN** the summary-block reader is shown instead of a blank webview

### Requirement: Chat is the index to results
Nova's image, web, and video results SHALL drop a compact, tappable card into the chat timeline; tapping a card SHALL switch to that view and select the corresponding tab, resolving to the correct item even after the history has been capped.

#### Scenario: Tapping a chat card jumps to its tab
- **WHEN** the user taps a result card in the chat
- **THEN** the app switches to that view and selects the tab for that specific result

#### Scenario: Evicted target is not shown
- **WHEN** a card's target search has been evicted from the capped history
- **THEN** the card is dropped from the timeline rather than jumping to the wrong result

### Requirement: Search history survives app restart
The image, web, and video search history SHALL be persisted to the app's files directory (capped) and restored into the tab strips on launch, on mobile only. The live conversation transcript is not persisted.

#### Scenario: History restored on next launch
- **WHEN** the app is relaunched after prior searches
- **THEN** the Images/Web/Videos views are reachable in the nav with the prior searches populated, and thumbnails are re-fetched lazily

### Requirement: Video search, playback, and vision
A `search_videos` tool SHALL return a `videos` op that populates a Video view (a results grid with tabbed history like Images). Tapping a result SHALL play it in an in-app player (YouTube embed on iOS). While a video plays on a full-duplex session, the current frame SHALL be captured periodically and sent to the agent so it can answer questions about the video.

#### Scenario: Watching a found video
- **WHEN** the user taps a video result on iOS
- **THEN** the video plays inline in the app's player, with controls to go back to the grid or open it in YouTube

#### Scenario: Asking about the playing video (duplex)
- **WHEN** a video is playing on a full-duplex session and the user asks what's happening
- **THEN** a recent frame of the video has been sent to the agent as a vision image so it can answer
