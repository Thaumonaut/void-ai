## ADDED Requirements

### Requirement: Confirm before outbound, destructive, or spending actions
Nova SHALL confirm with the user before any action that writes to an external account, is destructive, or spends money — stating the specific action — and SHALL only proceed after the user assents.

#### Scenario: Create a calendar event
- **WHEN** Nova is about to create or move a calendar event
- **THEN** she states the exact event/change and waits for the user's confirmation before committing

#### Scenario: Send or spend
- **WHEN** an action would send a message or spend money
- **THEN** Nova confirms the specifics before it happens

### Requirement: Reads are never blocked
Read-only actions SHALL NOT require confirmation.

#### Scenario: Reading the calendar
- **WHEN** the user asks what's on their calendar
- **THEN** Nova reads it immediately with no confirmation step

### Requirement: Confirmation is in-character but unambiguous
The confirmation prompt SHALL preserve Nova's persona while unambiguously naming the action to be taken.

#### Scenario: Confirmation wording
- **WHEN** Nova asks for confirmation of a write
- **THEN** the request is dry/in-character yet clearly identifies exactly what will happen if the user says yes
