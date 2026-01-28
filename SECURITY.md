# Security Policy

## Project Status

> **Warning**: Agentik is currently in early development and is NOT suitable for production use. Security features are incomplete and the codebase has not undergone security auditing.

## Reporting a Vulnerability

If you discover a security vulnerability in Agentik, please report it responsibly:

1. **Do not** open a public issue for security vulnerabilities
2. Email the maintainers directly (or use GitHub's private vulnerability reporting if available)
3. Include as much detail as possible:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

## Security Considerations

### API Keys

Agentik handles API keys for AI providers. Users should:

- Never commit API keys to version control
- Use environment variables for sensitive credentials
- Review the `.gitignore` to ensure secrets are excluded

### Session Data

Session data is stored locally in:
- `~/.local/share/agentik/sessions.db` (SQLite database)
- `~/.local/share/agentik/sessions/` (message logs)

This data may contain:
- Conversation history
- File contents that were read or modified
- Tool execution logs

Users should be aware that this data is stored in plaintext on their local filesystem.

### Shell Execution

Agentik can execute shell commands on behalf of the user. This is a powerful feature that carries inherent risks. The tool should:

- Only execute commands explicitly requested
- Provide visibility into what commands are being run
- Not execute commands from untrusted sources

### Network Requests

Agentik makes network requests to:
- AI provider APIs (Anthropic, OpenAI, etc.)
- Web pages (when using web fetch tools)
- MCP servers (local or remote)

Users should be aware of what data is being sent to external services.

## Planned Security Features

The following security features are planned but not yet implemented:

- [ ] Sandboxed shell execution
- [ ] Command approval workflows
- [ ] Encrypted session storage
- [ ] Audit logging
- [ ] Rate limiting
- [ ] Budget enforcement

## Disclaimer

This software is provided "as is" without warranty of any kind. The authors are not responsible for any security incidents arising from the use of this software. Use at your own risk.
