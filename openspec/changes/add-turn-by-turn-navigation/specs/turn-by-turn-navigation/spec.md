## ADDED Requirements

### Requirement: Guidance session lifecycle
The system SHALL start a guidance session from a computed route and emit a stream of structured events (progress, upcoming maneuver, reroute, arrival) that the UI and Nova consume. The session SHALL end on arrival or user cancel.

#### Scenario: Start guidance from a route
- **WHEN** the user confirms a route and starts navigation
- **THEN** a guidance session begins, the driving mode UI appears, and the first upcoming-maneuver and progress events are emitted

#### Scenario: Cancel guidance
- **WHEN** the user cancels navigation mid-trip
- **THEN** the session ends, guidance and voice stop, and the UI returns to the map/browse view

### Requirement: Maneuver guidance
The system SHALL track the device position against the route and announce each upcoming maneuver with its type and distance-to-maneuver, updating as the vehicle approaches.

#### Scenario: Approaching a turn
- **WHEN** the vehicle is within the announcement distance of the next maneuver
- **THEN** the UI shows the maneuver (e.g., "Turn left onto Pine St") with live distance, and a voice prompt is issued

### Requirement: Off-route detection and rerouting
The system SHALL detect when the device has departed the route beyond a tolerance and SHALL compute and switch to a new route, notifying the user.

#### Scenario: Driver misses a turn
- **WHEN** the device leaves the active route beyond the off-route tolerance
- **THEN** the system detects off-route, computes a new route to the destination, and announces the reroute

### Requirement: ETA and distance remaining
The system SHALL continuously report estimated time of arrival and remaining distance for the active route.

#### Scenario: Progress along the route
- **WHEN** the vehicle advances along the route
- **THEN** ETA and remaining distance update in the UI to reflect current progress

### Requirement: Arrival detection
The system SHALL detect arrival at the destination and end the guidance session with an arrival notification.

#### Scenario: Reaching the destination
- **WHEN** the device reaches the destination within the arrival radius
- **THEN** the system announces arrival and ends the guidance session
