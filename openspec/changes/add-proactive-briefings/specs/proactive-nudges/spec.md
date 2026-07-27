## ADDED Requirements

### Requirement: Leave-by nudge
For a calendar event with a resolvable location, Nova SHALL deliver a timely "leave now / leave in N" nudge computed from live travel time so the user can arrive on time.

#### Scenario: Nudge before a located event
- **WHEN** an upcoming calendar event has a location and the user has a resolvable origin
- **THEN** Nova delivers a nudge at approximately `event_start − travel_time − prep_buffer`, using a live ETA

#### Scenario: No location, no false nudge
- **WHEN** an event has no resolvable location
- **THEN** no leave-by nudge is scheduled for it

### Requirement: Reminder and timer fires
Due reminders and timers SHALL fire via the proactive path — spoken if the user is connected, otherwise pushed.

#### Scenario: Reminder comes due while connected
- **WHEN** a reminder's time arrives and a session is connected
- **THEN** Nova speaks it

#### Scenario: Timer completes while app closed
- **WHEN** a timer completes and no session is connected
- **THEN** the user receives a push

### Requirement: Frequency caps and DND
Proactive nudges SHALL respect quiet hours and per-hour/day frequency caps, batching deferrable nudges into the next briefing rather than firing separately.

#### Scenario: Burst is capped
- **WHEN** more nudges would fire than the cap allows in a window
- **THEN** excess deferrable nudges are batched or delayed, not all delivered at once

### Requirement: Proactive speech does not collide with the user
A proactive spoken turn SHALL NOT interrupt an in-progress user utterance or exchange.

#### Scenario: User is mid-sentence
- **WHEN** a proactive turn is due but the user is currently speaking
- **THEN** Nova waits for a clear moment (or defers to push) instead of talking over the user
