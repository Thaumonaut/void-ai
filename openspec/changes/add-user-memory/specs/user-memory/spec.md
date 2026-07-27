## ADDED Requirements

### Requirement: Persistent per-user memory
The system SHALL persist user facts server-side, keyed to the authenticated user, so that facts survive across sessions, reconnects, and app restarts.

#### Scenario: Fact survives a new session
- **WHEN** a user tells Nova a fact in one session and later starts a new session
- **THEN** the fact is available to Nova in the new session without the user repeating it

#### Scenario: Per-user isolation
- **WHEN** two different authenticated users each store facts
- **THEN** neither user's facts are ever injected into or retrievable by the other

### Requirement: Context injection at session start
Nova SHALL be primed at session start with the user's profile plus a token-budgeted set of relevant facts, so she opens each session already knowing the user.

#### Scenario: Opens knowing the user
- **WHEN** a session begins for a user with stored facts
- **THEN** Nova's context includes the user's profile and relevant facts, within a fixed token budget, most-relevant first

#### Scenario: Empty memory is a no-op
- **WHEN** a session begins for a user with no stored facts
- **THEN** behavior is identical to having no memory system (no error, no empty preamble)

### Requirement: Memory tools
Nova SHALL be able to store, retrieve, and remove facts during a conversation via `remember`, `recall`, and `forget`.

#### Scenario: Remember then recall
- **WHEN** the user says "remember I hate cilantro" and later asks something where that matters
- **THEN** the fact is stored and reflected in Nova's later behavior or answers

#### Scenario: Forget
- **WHEN** the user tells Nova to forget a stored fact
- **THEN** the fact is removed and no longer appears in Nova's context, and Nova confirms what was removed

### Requirement: Proactive capture with consent for sensitive facts
Nova SHALL remember stable, salient facts on her own initiative, but SHALL confirm before storing sensitive facts (health, finances, relationships, precise home address).

#### Scenario: Casual salient fact captured silently
- **WHEN** the user mentions a stable preference or routine in passing
- **THEN** Nova may store it without an explicit instruction

#### Scenario: Sensitive fact gated
- **WHEN** a fact to be stored is sensitive (health, finances, relationships, precise address)
- **THEN** Nova confirms with the user before storing it
