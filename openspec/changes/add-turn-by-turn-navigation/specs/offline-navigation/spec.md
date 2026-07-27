## ADDED Requirements

### Requirement: Offline map rendering
The system SHALL render the navigation map from map data stored on the device, without any network request, for regions the user has downloaded.

#### Scenario: Render map with no network in a downloaded region
- **WHEN** the device has no network and is within a downloaded map region
- **THEN** the navigation map renders normally from on-device tiles (no blank screen)

### Requirement: On-device routing
The system SHALL compute routes and reroutes on the device from downloaded routing data, without a routing server, for regions the user has downloaded.

#### Scenario: Route with no network
- **WHEN** the user requests directions with no network in a downloaded region
- **THEN** the system computes a route on-device and starts guidance

#### Scenario: Reroute offline
- **WHEN** the device goes off-route while offline
- **THEN** the system computes a new route on-device without network

### Requirement: Predictive caching of the active route
While connected, the system SHALL pre-fetch the map tiles and route data along the remaining route so guidance continues if the network is lost mid-trip.

#### Scenario: Losing signal mid-trip
- **WHEN** the vehicle is navigating an active route and loses network in a dead zone
- **THEN** guidance (map, maneuvers, voice) continues uninterrupted from the pre-cached route data

### Requirement: Graceful online/offline degradation
The system SHALL prefer online, traffic-aware routing when connected and SHALL fall back to on-device routing when disconnected, without ending the guidance session or blanking the map.

#### Scenario: Connectivity drops during guidance
- **WHEN** the network drops during an active guidance session
- **THEN** the system continues guidance using on-device data and resumes online/traffic features when connectivity returns

### Requirement: Region download management
The system SHALL let the user download map + routing packs for chosen regions, report their storage size, and remove them, within a configurable storage budget.

#### Scenario: Download a region for offline use
- **WHEN** the user downloads a region over a connection
- **THEN** the region's map + routing data is stored on-device and becomes available for offline rendering and routing
