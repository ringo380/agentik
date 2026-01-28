# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

> **Note**: This project is in early development. No stable releases have been made yet.

### Added

- Initial project structure with Rust workspace
- Core types and abstractions (`agentik-core`)
  - Message and conversation primitives
  - Session and state management types
  - Tool definitions and execution types
  - Configuration system with figment
- Multi-provider AI abstraction (`agentik-providers`)
  - Anthropic Claude support
  - OpenAI GPT support
  - Local/Ollama provider stub
  - Provider registry
- Session management (`agentik-session`)
  - SQLite-backed session storage
  - JSONL append-only message logs
  - Context window management
  - Automatic compaction with summarization
  - Session recovery with prefix matching
- Agent orchestration stubs (`agentik-agent`)
- Built-in tools stubs (`agentik-tools`)
- MCP client stubs (`agentik-mcp`)
- Repository mapping stubs (`agentik-repomap`)
- Usage tracking stubs (`agentik-metrics`)
- CLI application (`agentik-cli`)
  - Session list/show/export/delete commands
  - Continue (`-c`) and resume (`-r`) flags
  - Provider and config subcommands
- TypeScript MCP server stubs

### Changed

- Nothing yet

### Deprecated

- Nothing yet

### Removed

- Nothing yet

### Fixed

- Nothing yet

### Security

- Nothing yet

---

## Version History

No stable versions have been released yet. This project is under active development.

When the first stable release is made, it will be documented here following semantic versioning.
