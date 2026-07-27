## ADDED Requirements

### Requirement: Congestion display
When online, the system SHALL display live traffic congestion along the route and surrounding roads.

#### Scenario: Traffic on the route
- **WHEN** the device is online and there is congestion on or near the route
- **THEN** the map shows congestion coloring for the affected segments

### Requirement: Traffic-aware routing and rerouting
When online, the system SHALL compute routes using live traffic and SHALL offer or switch to a faster route when traffic conditions change materially.

#### Scenario: Faster route appears due to traffic
- **WHEN** live traffic makes an alternative materially faster during guidance
- **THEN** the system surfaces (or, per settings, switches to) the faster route and reflects the new ETA

### Requirement: Fallback to non-traffic routing offline
When traffic data is unavailable (offline), the system SHALL route using non-traffic (historical/default) speeds without failing, and SHALL clearly reflect that traffic is unavailable.

#### Scenario: No traffic data available
- **WHEN** the device is offline or traffic data cannot be fetched
- **THEN** routing proceeds on non-traffic speeds and the UI indicates traffic is unavailable
