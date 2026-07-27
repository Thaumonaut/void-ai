## ADDED Requirements

### Requirement: No dead air during tool latency
The moment the agent invokes a tool, the system SHALL immediately speak one in-character line so there is never silence while a slow (1–3s) network fetch runs.

#### Scenario: Slow tool call
- **WHEN** a tool call begins
- **THEN** the agent speaks a curated in-character filler line for that tool immediately, before the result returns

#### Scenario: Line variety
- **WHEN** the same tool is used repeatedly
- **THEN** the filler line is lightly randomized so it does not sound scripted

### Requirement: Avoid double-talk
When the model itself emits a spoken one-liner alongside the tool call, the system SHALL suppress the curated filler line to avoid talking over itself.

#### Scenario: Model emits its own line
- **WHEN** the model produces a spoken one-liner with the tool call
- **THEN** the curated filler line for that call is suppressed

### Requirement: React, don't recite, after results
After results render on screen, the agent SHALL react to them conversationally rather than reading the on-screen data (street names, prices, URLs) aloud.

#### Scenario: Results land
- **WHEN** tool results have been pushed to the view
- **THEN** the agent comments in character without reciting the on-screen details
