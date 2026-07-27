## ADDED Requirements

### Requirement: Authenticated session establishment
The bot SHALL reject `/api/offer` (and `/ice`) requests that do not present valid authentication, and SHALL rate-limit by caller.

#### Scenario: Unauthenticated caller
- **WHEN** a request to `/api/offer` lacks valid auth
- **THEN** the bot rejects it without starting a session or spending any paid API quota

#### Scenario: Excessive requests
- **WHEN** a single caller exceeds the rate limit
- **THEN** further requests are throttled/rejected

### Requirement: Encrypted signaling
Signaling SHALL be served over TLS, and the client's transport-security exception SHALL be scoped to the bot host only.

#### Scenario: Signaling over TLS
- **WHEN** the client establishes a session
- **THEN** the offer/answer exchange occurs over HTTPS, not plaintext HTTP

### Requirement: No secrets or PII in logs
The bot SHALL NOT write API keys or user location to logs.

#### Scenario: Tool provider errors
- **WHEN** a tool's upstream call errors (4xx/5xx/timeout)
- **THEN** the log records the provider + status code but NOT the API key or full request URL

#### Scenario: Location handling
- **WHEN** the client sends its GPS location
- **THEN** the raw coordinates are not logged at info level

### Requirement: Repointable endpoint
The client SHALL reach the bot by a resolvable domain so the backend can be moved without shipping a new app build.

#### Scenario: Backend moves
- **WHEN** the bot is redeployed to a new host
- **THEN** existing clients reach it via DNS without an app update
