## ADDED Requirements

### Requirement: Swipe to switch persona
The app SHALL let the user switch the active agent between Nova and Kaira by swiping horizontally on the agent icon, without disrupting the existing tap and long-press behaviors.

#### Scenario: Swipe toggles the persona
- **WHEN** the user swipes horizontally across the agent icon past the drag threshold
- **THEN** the active persona toggles (Nova ⇄ Kaira) and the agent icon, name, and accent palette update to the newly-selected persona

#### Scenario: Tap and long-press preserved
- **WHEN** the user taps or long-presses the agent icon
- **THEN** the existing connect/disconnect and long-press behaviors run, and no persona switch occurs

### Requirement: Per-persona identity
Each persona SHALL carry its own identity — name, accent palette, and agent-icon kind — defined as data so additional personas can be added without new UI branching.

#### Scenario: Identity reflects the selected persona
- **WHEN** a persona is selected
- **THEN** the app shows that persona's name and accent palette, and renders the constellation orb for Nova or the character avatar for Kaira

### Requirement: Reconnect to the selected persona's bot
Because a persona's voice, prompt, and tools are fixed when the bot builds the session, the app SHALL connect to the bot instance serving the selected persona; switching persona while connected SHALL reconnect to that persona's bot.

#### Scenario: Switch while connected
- **WHEN** the user switches persona while a session is live
- **THEN** the app disconnects and reconnects to the newly-selected persona's bot URL, and the user is now talking to that persona (e.g., Kaira greets as a GP in her voice)

#### Scenario: Two-bot model
- **WHEN** the app connects for a given persona
- **THEN** it targets that persona's own bot instance (Nova and Kaira run as separate instances via `KAIRA_PERSONA`), with no persona parameter required in the bot protocol

### Requirement: Default persona
The app SHALL start with a defined default persona.

#### Scenario: Fresh launch
- **WHEN** the app launches with no prior selection
- **THEN** the default persona (Nova) is active
