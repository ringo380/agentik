# Contributing to Agentik

Thank you for your interest in contributing to Agentik! This document provides guidelines and information for contributors.

> **Note**: Agentik is in early development. The codebase is evolving rapidly and major architectural changes may occur. Please open an issue to discuss significant changes before investing time in a pull request.

## Development Status

This project is a **work in progress**. Many features are incomplete or not yet implemented. We appreciate your patience and understanding as we build out the core functionality.

## How to Contribute

### Reporting Issues

- Check existing issues to avoid duplicates
- Use the issue templates when available
- Provide as much context as possible
- Include steps to reproduce for bugs

### Suggesting Features

- Open an issue with the "enhancement" label
- Describe the use case and expected behavior
- Be open to discussion about implementation approaches

### Code Contributions

1. **Fork the repository** and create your branch from `main`
2. **Open an issue first** for significant changes
3. **Write tests** for new functionality
4. **Follow the code style** (run `cargo fmt` and `cargo clippy`)
5. **Update documentation** as needed
6. **Submit a pull request**

## Development Setup

### Prerequisites

- Rust 1.75 or later
- Node.js 18 or later (for MCP servers)
- Git

### Building

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/agentik.git
cd agentik

# Build all crates
cargo build

# Run tests
cargo test

# Check formatting
cargo fmt --check

# Run linter
cargo clippy
```

### Project Structure

```
crates/
├── agentik-core/        # Core types - start here
├── agentik-providers/   # AI provider implementations
├── agentik-session/     # Session and context management
├── agentik-agent/       # Agent orchestration
├── agentik-tools/       # Built-in tools
├── agentik-mcp/         # MCP integration
├── agentik-repomap/     # Repository analysis
├── agentik-metrics/     # Usage tracking
└── agentik-cli/         # CLI application
```

## Code Style

### Rust

- Use `rustfmt` for formatting
- Use `clippy` for linting
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Document public APIs with rustdoc comments
- Use `thiserror` for error types
- Prefer explicit error handling over `.unwrap()`

### TypeScript (MCP Servers)

- Use ESLint and Prettier
- Follow TypeScript best practices
- Document exported functions

## Commit Messages

- Use present tense ("Add feature" not "Added feature")
- Use imperative mood ("Move cursor to..." not "Moves cursor to...")
- Keep the first line under 72 characters
- Reference issues when applicable

Example:
```
Add session recovery by ID prefix

Implement resume_by_prefix() to allow users to resume sessions
using partial IDs like "abc123" instead of full UUIDs.

Fixes #42
```

## Pull Requests

- Keep PRs focused on a single change
- Include tests for new functionality
- Update documentation as needed
- Ensure CI passes before requesting review
- Be responsive to feedback

## Testing

- Write unit tests for new functionality
- Place tests in the same file using `#[cfg(test)]`
- Use `tempfile` for tests that need filesystem access
- Run `cargo test` before submitting PRs

## License

By contributing to Agentik, you agree that your contributions will be licensed under the MIT License.

## Questions?

Feel free to open an issue for any questions about contributing. We're happy to help!
