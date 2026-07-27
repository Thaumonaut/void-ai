## ADDED Requirements

### Requirement: Auto-reconnect on unexpected drop
When an established voice session drops for a reason other than a user disconnect, the system SHALL attempt to reconnect automatically, and SHALL give up after a bounded number of consecutive failed attempts with a retry affordance.

#### Scenario: Transient network blip
- **WHEN** a live session's peer connection drops (not user-initiated)
- **THEN** the system automatically re-establishes the session, showing a "reconnecting…" state

#### Scenario: Persistent failure
- **WHEN** reconnection fails repeatedly
- **THEN** the system stops retrying and presents a "connection failed — tap to retry" state that reconnects on tap

### Requirement: Bounded connect and teardown
The system SHALL NOT hang indefinitely on connect or teardown: the signaling request SHALL time out, and session teardown SHALL be time-boxed and forcibly aborted if it overruns.

#### Scenario: Unreachable bot
- **WHEN** the bot IP is unreachable/black-holed
- **THEN** the connect attempt fails within a few seconds (not the OS TCP timeout) and the reconnect logic proceeds

### Requirement: Truthful connection state in the UI
The UI SHALL reflect the actual connection state — connecting, reconnecting, live, or failed — and SHALL NOT indicate "connected/live" before the peer connection is actually established.

#### Scenario: Connecting is not "live"
- **WHEN** the user starts a session but the peer connection is not yet up
- **THEN** the UI shows a connecting/status indication, not the "live/talking" state

#### Scenario: Failure resets intent
- **WHEN** the session terminally fails
- **THEN** the UI resets to a not-connected/retry state so the primary action becomes reconnect
