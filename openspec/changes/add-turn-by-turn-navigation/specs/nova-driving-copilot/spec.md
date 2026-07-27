## ADDED Requirements

### Requirement: Spoken guidance in Nova's voice
The system SHALL speak navigation maneuvers in Nova's voice and personality using on-device TTS, functioning with no network.

#### Scenario: Maneuver announced offline
- **WHEN** an upcoming-maneuver event fires while the device has no network
- **THEN** Nova speaks the maneuver via on-device TTS, in character

### Requirement: Converse while navigating
The system SHALL allow the user to talk to Nova during an active guidance session, and Nova SHALL answer while preserving the guidance thread.

#### Scenario: User asks a question mid-trip
- **WHEN** the user speaks to Nova during navigation (e.g., "why did you reroute me?")
- **THEN** Nova answers conversationally without ending guidance, and resumes maneuver prompts as they come due

### Requirement: Guidance priority and non-loss
The system SHALL prioritize time-critical maneuver prompts over conversational speech and SHALL NOT drop a due maneuver prompt; conversational audio SHALL duck or defer as needed.

#### Scenario: Turn is due while Nova is mid-sentence
- **WHEN** a maneuver prompt becomes due while Nova is speaking a conversational reply
- **THEN** the system ensures the maneuver is announced in time (interrupting or ducking the conversational reply)

### Requirement: Reroute and arrival narration
The system SHALL have Nova narrate reroute and arrival events in character.

#### Scenario: Reroute narration
- **WHEN** a reroute occurs
- **THEN** Nova announces the reroute in character (e.g., acknowledging traffic or a missed turn)
