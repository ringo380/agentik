# Agentik Development Guide

## Overview

Agentik is a CLI-based agentic AI tool that combines the best of Aider, Claude Code, and Codex. It supports multiple AI providers, MCP integration, intelligent session management, and cost tracking.

## Architecture

```
agentik/
├── crates/                      # Rust crates (Cargo workspace)
│   ├── agentik-core/            # Core types, config, errors
│   ├── agentik-providers/       # Multi-provider AI abstraction
│   ├── agentik-session/         # Session persistence, compaction
│   ├── agentik-agent/           # Agent orchestration, planning
│   ├── agentik-tools/           # Built-in tools (file, shell, git)
│   ├── agentik-mcp/             # MCP client integration
│   ├── agentik-repomap/         # Tree-sitter repo mapping
│   ├── agentik-metrics/         # Cost tracking, benchmarks
│   └── agentik-cli/             # CLI binary
├── mcp-servers/                 # TypeScript MCP server implementations
└── data/                        # Prompts, pricing data
```

## Build Commands

### Rust (Main CLI)

```bash
# Build all crates
cargo build

# Build release binary
cargo build --release

# Run the CLI
cargo run -- [args]

# Run tests
cargo test

# Run specific crate tests
cargo test -p agentik-core

# Check without building
cargo check

# Format code
cargo fmt

# Lint
cargo clippy
```

### TypeScript (MCP Servers)

```bash
cd mcp-servers

# Install dependencies
npm install

# Build
npm run build

# Watch mode
npm run dev

# Lint
npm run lint
```

## Key Crates

### agentik-core
Core types used across all crates:
- `Message`, `Role`, `Content` - Conversation primitives
- `ToolDefinition`, `ToolCall`, `ToolResult` - Tool system types
- `Session`, `SessionState`, `SessionMetadata` - Session types
- `Config` - Configuration system (figment-based)
- `Error`, `Result` - Error types

### agentik-providers
Multi-provider AI abstraction:
- `Provider` trait - Core provider interface
- `ToolCapable` trait - Tool calling capability
- Implementations for Anthropic, OpenAI, Ollama
- `ProviderRegistry` - Provider management

### agentik-session
Session and context management:
- SQLite-backed session storage
- Context window management
- Compaction and summarization
- Session resume/recovery

### agentik-agent
Agent orchestration:
- Core agent loop
- Planning mode
- Architect/Editor separation
- "Anytime" question asking

### agentik-tools
Built-in tool implementations:
- File operations (read, write, edit, glob, grep)
- Shell execution (sandboxed)
- Git operations
- Web fetch/search

### agentik-repomap
Repository mapping:
- Tree-sitter multi-language parsing
- Dependency graph construction
- PageRank-based file ranking

### agentik-mcp
MCP integration:
- MCP client for connecting to servers
- stdio and HTTP transports
- Tool discovery and registration

### agentik-metrics
Usage and cost tracking:
- Token and cost tracking
- Budget enforcement
- Model benchmark data

### agentik-cli
CLI application:
- clap-based command parsing
- ratatui TUI interface
- Slash commands
- Output formatting

## Development Workflow

1. **Adding a new feature**: Start in the appropriate crate, add types to agentik-core if shared
2. **Adding a provider**: Implement `Provider` and optionally `ToolCapable` in agentik-providers
3. **Adding a tool**: Implement in agentik-tools and register in `ToolRegistry`
4. **Adding an MCP server**: Create in mcp-servers/src/{name}

## Configuration

Config files (in order of priority):
1. `.agentik/config.local.toml` - Project local (gitignored)
2. `.agentik/config.toml` - Project shared
3. `~/.config/agentik/config.toml` - User config
4. Environment variables (`AGENTIK_*`)

## Testing

```bash
# Run all tests
cargo test

# Run with logging
RUST_LOG=debug cargo test

# Run specific test
cargo test test_name

# Integration tests
cargo test --test integration
```

## Code Style

- Use `rustfmt` for Rust formatting
- Use `clippy` for linting
- Follow Rust API guidelines
- Document public APIs with rustdoc
- Use `thiserror` for error types
- Use `anyhow` for error propagation in binaries

## Release Process

1. Update version in workspace Cargo.toml
2. Update CHANGELOG.md
3. Run full test suite
4. Build release binaries: `cargo build --release`
5. Create git tag
6. Publish to crates.io (if applicable)
