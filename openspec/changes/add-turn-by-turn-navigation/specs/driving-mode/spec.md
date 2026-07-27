## ADDED Requirements

### Requirement: Driving mode surface
The system SHALL present a dedicated driving mode showing the next-maneuver banner, current speed, and remaining distance/ETA, laid out for at-a-glance readability with large touch targets.

#### Scenario: Enter driving mode
- **WHEN** a guidance session starts
- **THEN** the driving mode surface is shown with the next maneuver, speed, and distance/ETA

### Requirement: Day/night appearance
The system SHALL adapt the driving mode appearance for day and night to remain legible and reduce glare.

#### Scenario: Night driving
- **WHEN** it is night (or dark mode is active)
- **THEN** the driving mode switches to a dark, low-glare appearance

### Requirement: Keep-awake and background operation
The system SHALL keep the screen awake during active guidance and SHALL continue navigation (position tracking + voice) when the app is backgrounded or the screen is off.

#### Scenario: Screen off while navigating
- **WHEN** the screen turns off or the app is backgrounded during guidance
- **THEN** position tracking and voice guidance continue, and re-opening the app resumes the driving mode in sync

### Requirement: Lane and next-step hints
When lane guidance and next-step data are available in the route, the system SHALL display which lane(s) to use and a preview of the step after the current maneuver.

#### Scenario: Complex intersection with lane data
- **WHEN** the upcoming maneuver has lane guidance data
- **THEN** the driving mode shows which lane(s) to use
