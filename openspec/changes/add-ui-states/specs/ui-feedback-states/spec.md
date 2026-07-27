## ADDED Requirements

### Requirement: Tool-in-progress feedback
While a tool the agent invoked is still running, the target view SHALL show a visible working/loading indication.

#### Scenario: Slow search
- **WHEN** the agent calls a slow tool (e.g. places/web/shopping) and results have not yet arrived
- **THEN** the target view shows a working/skeleton state rather than nothing

### Requirement: Failed-fetch states
When a resource fetch (image, map tile, product thumbnail) fails, the view SHALL show a failure indication with a way to retry, not a permanent placeholder.

#### Scenario: Image fails to load
- **WHEN** an image fetch fails
- **THEN** a broken-image indication with tap-to-retry replaces the placeholder tint (which currently persists forever)

### Requirement: Empty states
A view with no content SHALL show a purposeful empty state, and SHALL NOT render count/query headers built from empty data.

#### Scenario: Open an empty view
- **WHEN** the user opens a view before the agent has populated it
- **THEN** the view shows an empty-state hint, not `FOUND 0 · ""`

### Requirement: Offline indication
The app SHALL detect loss of network and indicate it, and SHALL explain the state on actions that require connectivity.

#### Scenario: Device goes offline
- **WHEN** the device loses network
- **THEN** the app shows an offline indication and the connect action reflects that it can't reach the bot
