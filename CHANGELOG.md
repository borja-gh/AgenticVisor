# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [0.1.0-beta.1] - 2026-10-27

First public beta. Written in Rust as a single binary with the interface built in.

### Added

- Overview of disk usage per tool and category, with a safe-to-clean total.
- Per-agent views for Claude Code, Claude Desktop and Cursor: sessions, cache,
  orphaned data, configuration, skills, plugins, artifacts and canvases.
- Full-text search inside conversations (SQLite FTS5 with trigrams), with
  optional tool outputs.
- Readable paged preview of sessions (JSONL, chat databases and plans).
- Configuration viewer and editor for files without credentials, with JSON
  validation and a backup before every save. Secret values are masked.
- Enable or disable Claude Code skills and plugins, with a backup and a change log.
- Token usage per model and per session for Claude Code and Claude Desktop.
- Trash: items are moved, never deleted directly. Emptying it asks for confirmation.
- `--version`, `--port`, `--no-browser`, `scan-json` and `usage-json` options.

### Known limitations

- Tested on macOS (Apple Silicon) only. Linux and Windows builds are produced by
  CI but their tool locations have not been verified.
- Cursor does not record reliable token counts, so usage is shown for Claude only.
