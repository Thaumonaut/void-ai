## ADDED Requirements

### Requirement: Flag-selected conversation engine
The bot SHALL select its conversation engine from `KAIRA_MODE`, defaulting to the cascade, and SHALL support at least one full-duplex/S2S engine as an alternative without code changes.

#### Scenario: Default is the cascade
- **WHEN** `KAIRA_MODE` is unset or `cascade`
- **THEN** the bot runs the STT→LLM→TTS cascade

#### Scenario: Select S2S
- **WHEN** `KAIRA_MODE=ultravox` (or `realtime`/`s2s`)
- **THEN** the bot runs the S2S engine with the same tools and UI-control bridge, dropping the separate STT/TTS services

### Requirement: Tools preserved across modes
Any selected engine SHALL still invoke Nova's tools and drive the views; a slow (1–3s) tool SHALL NOT hang or silence the conversation.

#### Scenario: Tool call in S2S mode
- **WHEN** a tool is called while running an S2S engine
- **THEN** the tool executes, results are pushed to the view, and the audio stream is covered (placeholder/async) rather than going silent or hanging

### Requirement: Cascade remains shippable and gains barge-in
The cascade SHALL remain the default shipping engine and SHALL support barge-in / natural turn-taking without requiring new third-party keys.

#### Scenario: User interrupts mid-response
- **WHEN** the user starts speaking while Nova is talking (cascade mode)
- **THEN** Nova yields to the user promptly (barge-in) without dropping the conversation

### Requirement: A/B evaluation is possible without regressions
Switching engines SHALL be a runtime flag only, so an S2S engine can be A/B-tested against the cascade on the same tools and prompt.

#### Scenario: A/B the same session shape
- **WHEN** the operator switches `KAIRA_MODE` between cascade and an S2S engine
- **THEN** the same tools, system prompt, and UI-control behavior apply in both, enabling a fair comparison
