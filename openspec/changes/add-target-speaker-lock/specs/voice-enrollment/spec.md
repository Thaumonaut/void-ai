## ADDED Requirements

### Requirement: One-time voice enrollment
The app SHALL let the user enroll their voice once — capturing a short sample through the same capture path used during a live session, computing a speaker embedding from it, and persisting that embedding as the enrolled voiceprint.

#### Scenario: Enrolling during onboarding
- **WHEN** a first-time user reaches the enrollment step and speaks the requested sample
- **THEN** the app captures the audio through the live capture path, computes a speaker embedding, and persists it as the user's voiceprint

#### Scenario: Sample too short or unusable
- **WHEN** the enrollment sample is too short or too quiet to produce a reliable embedding
- **THEN** the app does not persist a voiceprint and prompts the user to try the sample again

### Requirement: Re-enroll and reset
The app SHALL let the user re-record or clear their enrolled voiceprint after onboarding.

#### Scenario: Re-enrolling from Settings
- **WHEN** the user chooses to re-enroll or reset their voice from Settings
- **THEN** the app replaces or removes the stored voiceprint accordingly

### Requirement: Graceful un-enrolled state
When no valid voiceprint is stored, the system SHALL behave as if speaker locking is absent — every turn is treated as the user's.

#### Scenario: No voiceprint present
- **WHEN** a session starts and no valid enrolled voiceprint exists (never enrolled, reset, or corrupt)
- **THEN** the target-speaker lock is disabled for that session and behavior is identical to having no lock
