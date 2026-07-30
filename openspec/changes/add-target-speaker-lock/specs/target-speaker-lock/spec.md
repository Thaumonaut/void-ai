## ADDED Requirements

### Requirement: Gate turns on speaker match
When a valid voiceprint is enrolled and the lock is enabled, the system SHALL verify each detected user turn against the enrolled voiceprint and act only on turns that match, dropping non-matching turns before they are transcribed or answered.

#### Scenario: A different person speaks
- **WHEN** someone other than the enrolled user speaks a turn and the lock is enabled
- **THEN** the turn is dropped — it is not transcribed, no reply is generated, and no view changes

#### Scenario: The enrolled user speaks
- **WHEN** the enrolled user speaks a turn
- **THEN** the turn is transcribed and answered as normal

### Requirement: Fail open on uncertainty
The gate SHALL default to acting when it cannot confidently reject — a turn too short or below the confidence floor to verify is treated as the user's, never silently discarded.

#### Scenario: Turn too short to verify
- **WHEN** a turn is shorter than the minimum needed for a reliable embedding, or the verification confidence is below the floor
- **THEN** the turn is accepted and answered rather than dropped

### Requirement: Floor-holding hysteresis
After accepting a turn from the user, the system SHALL hold the floor for a short window during which subsequent turns are gated at a looser threshold, so clipped follow-ups from the same user are not rejected.

#### Scenario: Clipped continuation
- **WHEN** the user has just been answered and immediately says a short follow-up like "stop" or "no, the other one" within the hold window
- **THEN** that follow-up is accepted rather than rejected for being too short or borderline

#### Scenario: Hold window expires
- **WHEN** the hold window elapses with no further accepted user speech
- **THEN** gating returns to the normal threshold

### Requirement: Lock is visible and can be disabled
The system SHALL indicate when the target-speaker lock is active and SHALL let the user turn it off, after which every turn is acted on regardless of speaker.

#### Scenario: Disabling the lock
- **WHEN** the user turns the target-speaker lock off
- **THEN** subsequent turns from any speaker are transcribed and answered, and the indicator reflects that the lock is inactive

### Requirement: Tunable, observable decisions
Verification thresholds and hysteresis timings SHALL be configurable (not hard-coded), and each gate decision SHALL be logged with its score and outcome so thresholds can be tuned against real recordings.

#### Scenario: Tuning against a recording
- **WHEN** an operator replays a noisy multi-speaker recording through the gate
- **THEN** each turn's decision (score, threshold, accepted or dropped, and why) is recorded, and the thresholds can be adjusted via configuration without code changes
