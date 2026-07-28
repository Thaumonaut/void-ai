## ADDED Requirements

### Requirement: Character avatar with idle pose
A persona MAY use a character avatar as its agent icon; when it does, the app SHALL show a static idle pose while the agent is not speaking.

#### Scenario: Kaira idle
- **WHEN** the Kaira persona is active and the agent is not speaking
- **THEN** the agent icon shows Kaira's idle pose

### Requirement: Talking animation driven by speaking state
While an avatar persona's agent is speaking, the app SHALL play its talking animation (frame-cycled), returning to the idle pose when speech stops.

#### Scenario: Kaira speaks
- **WHEN** Kaira is speaking (the agent-speaking flag is set)
- **THEN** the avatar cycles its talking frames, and returns to the idle pose when she stops

#### Scenario: Speaking state source
- **WHEN** the agent-speaking state changes
- **THEN** the avatar animation follows it, using the same signal that mutes the mic during agent playback

### Requirement: Per-persona icon kind
The agent-icon component SHALL render according to each persona's icon kind — the constellation orb for Nova, the character avatar for Kaira — selected from persona data.

#### Scenario: Icon matches persona
- **WHEN** the active persona changes
- **THEN** the agent icon renders that persona's icon kind (orb vs avatar) without other UI changes
