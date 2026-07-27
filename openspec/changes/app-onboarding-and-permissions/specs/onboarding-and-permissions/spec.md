## ADDED Requirements

### Requirement: First-run onboarding
On first launch, the app SHALL orient the user (what Nova is / does) and prime the microphone (and location) permissions with rationale before requesting them.

#### Scenario: First launch
- **WHEN** the app is launched for the first time
- **THEN** an onboarding flow explains the app and requests mic/location permission with context

### Requirement: In-app permission request
The app SHALL request microphone (and location) permission through the platform, without relying on out-of-band tools.

#### Scenario: Granting mic access
- **WHEN** the user proceeds through onboarding and grants the mic permission
- **THEN** the app can capture audio and start a voice session

### Requirement: Denied-permission state
When a required permission is denied or blocked, the app SHALL show a clear, recoverable state instead of failing silently.

#### Scenario: Mic denied
- **WHEN** microphone permission is denied
- **THEN** the Talk surface shows a "microphone blocked — enable in Settings" state and does not silently attempt to connect a dead mic
