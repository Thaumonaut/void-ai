## ADDED Requirements

### Requirement: On-demand briefing
Nova SHALL assemble a digest from the calendar, weather, and top reminders (plus a headline) on request, showing it on the surface and speaking a brief summary.

#### Scenario: What's my day
- **WHEN** the user asks "what's my day?"
- **THEN** Nova assembles calendar + weather + top reminders, shows the digest, and speaks a short summary

### Requirement: Scheduled briefing
Nova SHALL deliver a briefing at a user-set time, spoken via a proactive turn if the user is connected and via push if not.

#### Scenario: Morning briefing, app open
- **WHEN** the scheduled briefing time arrives and a session is connected
- **THEN** Nova speaks the briefing and paints the digest without the user asking

#### Scenario: Morning briefing, app closed
- **WHEN** the scheduled briefing time arrives and no session is connected
- **THEN** the briefing is delivered as a tappable push that opens the app to the digest

### Requirement: Quiet hours respected
Proactive briefings SHALL NOT be delivered during the user's quiet-hours window.

#### Scenario: Inside quiet hours
- **WHEN** a briefing would fire inside the quiet-hours window
- **THEN** it is suppressed or deferred to the next allowed time, not delivered
