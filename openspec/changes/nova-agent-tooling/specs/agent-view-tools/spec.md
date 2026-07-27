## ADDED Requirements

### Requirement: Tools drive their view and summarize for the agent
Each content tool SHALL fetch its result, push it into the corresponding view over the RTVI UI-control channel, and return a short text summary to the agent so the agent reacts (does not recite) the on-screen content.

#### Scenario: Place search
- **WHEN** the agent calls `search_places(query, near)`
- **THEN** the map view is populated with pins and the agent receives a brief summary to speak, without reading coordinates or addresses aloud

#### Scenario: Directions
- **WHEN** the agent calls `get_directions(dest, mode)`
- **THEN** the map view shows the route and ETA and the agent receives a brief summary

### Requirement: Content tool set
The agent SHALL have tools for the core content types: places, directions, web search, image search, product search, and starting navigation.

#### Scenario: Image search collapses and switches
- **WHEN** the agent calls `search_images(query)`
- **THEN** the images view is shown and the Nova strip collapses to give it room

### Requirement: Thin UI-control actions
The agent SHALL be able to directly change view state via `set_view`, `collapse`, and `close_view` without fetching content.

#### Scenario: Make room for a view
- **WHEN** the agent calls `collapse`
- **THEN** the Nova strip collapses and the active view expands

### Requirement: UI-control contract over RTVI
The system SHALL define a documented UI-control message contract carried on the RTVI data channel, and the client SHALL apply received ops to the view surface.

#### Scenario: Client applies a server op
- **WHEN** the client receives a `server-message` UI-control op (e.g., `map`, `open_view`)
- **THEN** the client updates the corresponding view-surface state to match

### Requirement: Keyless client
The client SHALL NOT hold tool API keys; tools SHALL run server-side and stream results to the client.

#### Scenario: Client renders streamed results
- **WHEN** a tool produces results on the server
- **THEN** the server streams the rendered content/URLs to the client, which displays them without calling any tool API itself
