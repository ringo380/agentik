# Agentik

> **Warning**: This project is under active development and is not yet functional. It should not be used in production environments. APIs and features may change without notice.

A CLI-based agentic AI tool that combines the best ideas from Aider, Claude Code, and Codex. Agentik provides multi-provider AI support, MCP integration, intelligent session management, and cost tracking.

## Status

**Work in Progress** - This project is in early development. Core features are being implemented and the codebase is subject to significant changes.

### Development Progress

- [x] Core types and abstractions
- [x] Multi-provider AI abstraction (Anthropic, OpenAI, local)
- [x] Session persistence with SQLite
- [x] Context window management and compaction
- [ ] Agent orchestration and planning
- [ ] Built-in tools (file, shell, git, web)
- [ ] MCP client integration
- [ ] Repository mapping with tree-sitter
- [ ] TUI interface
- [ ] Cost tracking and budgets

## Architecture

Agentik is built as a Rust workspace with multiple crates:

```
agentik/
├── crates/
│   ├── agentik-core/        # Core types, config, errors
│   ├── agentik-providers/   # Multi-provider AI abstraction
│   ├── agentik-session/     # Session persistence, compaction
│   ├── agentik-agent/       # Agent orchestration, planning
│   ├── agentik-tools/       # Built-in tools
│   ├── agentik-mcp/         # MCP client integration
│   ├── agentik-repomap/     # Tree-sitter repo mapping
│   ├── agentik-metrics/     # Cost tracking, benchmarks
│   └── agentik-cli/         # CLI binary
└── mcp-servers/             # TypeScript MCP servers
```

## Prerequisites

- Rust 1.75+ (for building)
- Node.js 18+ (for MCP servers)
- SQLite 3.35+ (included via rusqlite)

## Building from Source

```bash
# Clone the repository
git clone https://github.com/ringo380/agentik.git
cd agentik

# Build all crates
cargo build

# Run tests
cargo test

# Build release binary
cargo build --release
```

## Configuration

Agentik uses a layered configuration system:

1. `.agentik/config.local.toml` - Project local (gitignored)
2. `.agentik/config.toml` - Project shared
3. `~/.config/agentik/config.toml` - User config
4. Environment variables (`AGENTIK_*`)

### API Keys

Set your provider API keys as environment variables:

```bash
export ANTHROPIC_API_KEY="your-key"
export OPENAI_API_KEY="your-key"
```

## Planned Usage

> Note: These commands are not yet functional.

```bash
# Start interactive mode
agentik

# Start with an initial prompt
agentik "Explain this codebase"

# Continue most recent session
agentik -c

# Resume specific session
agentik -r abc123

# Non-interactive mode
agentik -p "Fix the bug in main.rs"

# Session management
agentik session list
agentik session show <id>
agentik session export <id> --format markdown
```

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

As this project is in early development, please open an issue to discuss major changes before submitting a pull request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

Agentik draws inspiration from:
- [Aider](https://github.com/paul-gauthier/aider) - AI pair programming
- [Claude Code](https://claude.ai) - Anthropic's coding assistant
- [OpenAI Codex](https://openai.com/blog/openai-codex) - Code generation

## Disclaimer

This software is provided "as is", without warranty of any kind. Use at your own risk. The authors are not responsible for any damages or losses arising from the use of this software.
