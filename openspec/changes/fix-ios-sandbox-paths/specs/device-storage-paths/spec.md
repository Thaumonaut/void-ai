## ADDED Requirements

### Requirement: On-device writable storage path
On iOS, `files_dir()` SHALL resolve to a directory the app can read and write within its sandbox on a real device (not a developer host path).

#### Scenario: Settings persist on a physical device
- **WHEN** the user changes a setting (theme, rail position) on a real device
- **THEN** the setting is written and restored on next launch

#### Scenario: Config override is readable on device
- **WHEN** a config file (e.g. `realtime_url.txt`) exists in the app's storage on a real device
- **THEN** the app reads and applies it (the backend URL can be repointed without a rebuild)

### Requirement: No developer-absolute runtime defaults
Runtime file paths SHALL NOT default to a specific developer machine's absolute path.

#### Scenario: Runs on another machine/device
- **WHEN** the app runs on any device or a different developer's machine
- **THEN** storage paths resolve correctly without editing source
