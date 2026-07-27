## ADDED Requirements

### Requirement: Calendar read into the agenda view
Nova SHALL read the user's calendar over a requested range and populate the `agenda` view, returning a short summary rather than reciting every event.

#### Scenario: What's my day
- **WHEN** the user asks what's on their calendar
- **THEN** Nova reads the calendar, fills the `agenda` view, and speaks a brief summary (not a full read-out of every event)

### Requirement: Calendar writes are guarded
Nova SHALL support creating and moving events, and every such write SHALL pass through `write-action-confirmation` before it commits.

#### Scenario: Move an event
- **WHEN** the user asks to move an event
- **THEN** Nova states the exact change and only writes it after the user confirms

### Requirement: Reminders and tasks
Nova SHALL add, list, and complete reminders/tasks, persisted per user so they survive reconnects and app restarts.

#### Scenario: Add a dated reminder
- **WHEN** the user says "remind me to call the vet at 4"
- **THEN** the reminder is persisted, shown in the `list` view, and registered to fire at the given time

#### Scenario: Complete a reminder
- **WHEN** the user marks a reminder done
- **THEN** it is removed from the active `list` view

### Requirement: Timers
Nova SHALL set and list timers as server-owned scheduled jobs so they fire even if the app is backgrounded or closed.

#### Scenario: Set a timer
- **WHEN** the user says "set a 10-minute timer"
- **THEN** a timer job is scheduled that will fire in 10 minutes regardless of app foreground state

### Requirement: Weather without a key
Nova SHALL answer weather questions using a keyless provider, requiring no new API secret.

#### Scenario: Weather query with no key configured
- **WHEN** the user asks for the weather and no weather API key is configured
- **THEN** Nova still returns a forecast (via the keyless provider)

### Requirement: Notes quick-capture
Nova SHALL capture and list free-text notes, persisted per user.

#### Scenario: Jot something down
- **WHEN** the user says "jot this down: ..."
- **THEN** the note is stored and appears in the `list` view

### Requirement: Agenda and list views
The client SHALL render an `agenda` view (day/calendar) and a `list` view (reminders/tasks/notes) on the view surface, block-based and consistent with the existing views.

#### Scenario: Client renders a new view op
- **WHEN** the client receives an `agenda` or `list` UI-control op
- **THEN** it opens and populates the corresponding view on the surface
