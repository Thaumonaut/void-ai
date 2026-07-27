## ADDED Requirements

### Requirement: Rolling cross-session summary
The system SHALL persist a short rolling summary of the conversation per user and re-inject it at the next session start, so recent context carries across the session boundary.

#### Scenario: Resume a prior thread
- **WHEN** a user references something from a previous session ("where'd we land on that flight thing?")
- **THEN** Nova resolves the reference using the persisted summary rather than treating it as new

### Requirement: Bounded summary
The rolling summary SHALL be capped in size; when the cap is reached, older content SHALL be compressed or dropped rather than growing unbounded.

#### Scenario: Summary stays within budget
- **WHEN** many sessions accumulate
- **THEN** the persisted summary remains within its size cap and injection cost stays bounded
