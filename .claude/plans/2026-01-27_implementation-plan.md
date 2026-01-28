# Agentik Implementation Plan

## Vision
A CLI-based agentic AI tool combining the best of Aider, Claude Code, and Codex - supporting all major AI providers with built-in rankings, cost tracking, MCP integration, and intelligent session management.

## Technology Stack
- **Core**: Rust (single binary, performance, type safety)
- **MCP Servers**: TypeScript (npm ecosystem, protocol SDK)
- **Build**: Cargo workspace + npm workspaces

## Architecture Overview

```
agentik/
├── Cargo.toml                    # Workspace manifest
├── crates/
│   ├── agentik-core/             # Domain types, config, errors
│   ├── agentik-providers/        # Multi-provider abstraction layer
│   ├── agentik-session/          # Session persistence, compaction
│   ├── agentik-agent/            # Agent orchestration, planning
│   ├── agentik-tools/            # Built-in tools (file, shell, git)
│   ├── agentik-mcp/              # MCP client integration
│   ├── agentik-repomap/          # Tree-sitter based repo mapping
│   ├── agentik-metrics/          # Cost tracking, benchmarks
│   └── agentik-cli/              # CLI binary (clap + ratatui)
├── mcp-servers/                  # TypeScript MCP implementations
│   ├── package.json
│   └── src/
└── data/
    ├── prompts/                  # System prompts
    └── pricing.json              # Provider pricing data
```

---

## Phase 1: Foundation (Weeks 1-2)

### 1.1 Project Scaffolding
- [ ] Initialize Cargo workspace with all crate stubs
- [ ] Set up TypeScript MCP workspace
- [ ] Create CLAUDE.md with development instructions
- [ ] Configure CI/CD (GitHub Actions)

### 1.2 Core Types (`agentik-core`)
```rust
// Key types to implement
- Message, Role, Content (conversation primitives)
- ToolDefinition, ToolCall, ToolResult
- Session, SessionState, SessionMetadata
- Config (figment-based hierarchical config)
- Error types with thiserror
```

### 1.3 Configuration System
- [ ] TOML config file parsing (~/.config/agentik/config.toml)
- [ ] Environment variable support (AGENTIK_*, provider API keys)
- [ ] Per-project config (.agentik/config.toml)
- [ ] Secure credential storage (system keychain integration)

### 1.4 Basic CLI (`agentik-cli`)
- [ ] Clap-based command parsing
- [ ] Top-level commands: `agentik`, `agentik session`, `agentik config`
- [ ] Interactive REPL mode with ratatui
- [ ] Basic input/output with syntax highlighting (syntect)

---

## Phase 2: Provider Layer (Weeks 3-4)

### 2.1 Provider Abstraction (`agentik-providers`)
```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn available_models(&self) -> Vec<ModelInfo>;
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn complete_stream(&self, request: CompletionRequest) -> Result<StreamResponse>;
}

pub trait ToolCapable: Provider {
    fn format_tools(&self, tools: &[ToolDefinition]) -> Value;
    fn parse_tool_calls(&self, response: &CompletionResponse) -> Result<Vec<ToolCall>>;
}
```

### 2.2 Provider Implementations
- [ ] **Anthropic**: Claude models via anthropic-rs or raw reqwest
- [ ] **OpenAI**: GPT models, compatible with OpenRouter
- [ ] **Local**: Ollama wrapper (OpenAI-compatible API)
- [ ] Provider registry for dynamic selection
- [ ] Streaming support with tokio channels

### 2.3 Tool Calling Normalization
- [ ] Unified tool schema format
- [ ] Provider-specific serialization
- [ ] Response parsing and normalization

---

## Phase 3: Session Management (Weeks 5-6)

### 3.1 Session Storage (`agentik-session`)
- [ ] SQLite-backed session index
- [ ] JSONL append-only message logs
- [ ] Session lifecycle state machine (active → suspended → archived)
- [ ] Resume/continue functionality
- [ ] Session forking

### 3.2 Context Window Management
- [ ] Token counting per provider (tiktoken for OpenAI, Anthropic's method)
- [ ] Dynamic budget allocation
- [ ] Compaction trigger (threshold-based + manual)
- [ ] Smart summarization with key decision preservation

### 3.3 Conversation Flow
```
┌──────────┐    ┌──────────┐    ┌──────────┐
│  Input   │───►│  Agent   │───►│ Provider │
└──────────┘    │ (tools)  │    └──────────┘
                └────┬─────┘
                     │
              ┌──────▼─────┐
              │  Session   │
              │  Storage   │
              └────────────┘
```

---

## Phase 4: Repository Map (Weeks 7-8)

### 4.1 Tree-Sitter Integration (`agentik-repomap`)
- [ ] Multi-language parsing (TS, Python, Rust, Go, Java, etc.)
- [ ] Symbol extraction (functions, classes, types)
- [ ] Signature and docstring capture

### 4.2 Dependency Graph
- [ ] Import/export analysis
- [ ] PageRank-based file ranking
- [ ] Query-specific relevance boosting

### 4.3 Context-Aware Serialization
- [ ] Token-budget aware output
- [ ] Focus file expansion
- [ ] Incremental updates via file watching

---

## Phase 5: Built-in Tools (Weeks 9-10)

### 5.1 File Operations (`agentik-tools`)
- [ ] Read (with line ranges, syntax highlighting)
- [ ] Write (with backup, confirmation)
- [ ] Edit (search/replace, unified diff)
- [ ] Glob, Grep (ripgrep integration)

### 5.2 Shell Execution
- [ ] Sandboxed command execution
- [ ] Directory restrictions
- [ ] Command allowlist/blocklist
- [ ] Timeout handling

### 5.3 Git Integration
- [ ] Status, diff, log queries
- [ ] Automatic commit with AI-generated messages
- [ ] Undo (revert last AI commit)
- [ ] Branch management

---

## Phase 6: Agent Orchestration (Weeks 11-12)

### 6.1 Core Agent Loop (`agentik-agent`)
```rust
pub struct Agent {
    provider: Arc<dyn Provider>,
    session: Session,
    tools: ToolRegistry,
    mode: AgentMode,
}

impl Agent {
    pub async fn process(&mut self, input: &str) -> Result<Response> {
        // 1. Build context (system prompt + repo map + conversation)
        // 2. Call provider
        // 3. Handle tool calls
        // 4. Persist to session
        // 5. Check for questions to ask
    }
}
```

### 6.2 Planning Mode
- [ ] Read-only exploration phase
- [ ] Task decomposition
- [ ] Plan file generation (markdown)
- [ ] Approval workflow
- [ ] Tool restrictions (no writes in planning)

### 6.3 Architect/Editor Separation
- [ ] Dual-model configuration
- [ ] High-level model for planning
- [ ] Cost-effective model for implementation
- [ ] Automatic mode switching

### 6.4 "Anytime" Question Asking
- [ ] Ambiguity detection (vague references, missing params)
- [ ] Priority-based question queue (blocking → high → normal → low)
- [ ] Interrupt delivery for blocking questions
- [ ] Batch presentation at idle

---

## Phase 7: MCP Integration (Weeks 13-14)

### 7.1 MCP Client (`agentik-mcp`)
- [ ] Server discovery and connection
- [ ] stdio and HTTP transports
- [ ] Tool discovery and registration
- [ ] Capability caching

### 7.2 MCP Server Management
- [ ] Add/remove/enable/disable servers
- [ ] Server health monitoring
- [ ] Log viewing
- [ ] Configuration persistence

### 7.3 TypeScript MCP Servers
- [ ] Filesystem server (enhanced file ops)
- [ ] Git server (full git operations)
- [ ] Web server (fetch, search)

---

## Phase 8: Metrics & Rankings (Weeks 15-16)

### 8.1 Cost Tracking (`agentik-metrics`)
- [ ] Per-request token/cost recording
- [ ] Session, daily, monthly aggregation
- [ ] Budget limits with warnings
- [ ] Export to CSV/JSON

### 8.2 Model Benchmarking
- [ ] Embedded benchmark scores (MMLU, HumanEval, etc.)
- [ ] Provider pricing database
- [ ] Cost-effectiveness ranking
- [ ] User task-specific scoring (optional)

### 8.3 Usage Analytics
- [ ] SQLite-based usage database
- [ ] Dashboard command (`agentik stats`)
- [ ] Model comparison views

---

## Phase 9: Polish & Aider Parity (Weeks 17-18)

### 9.1 Aider Feature Parity Checklist
- [ ] Repository map with graph ranking
- [ ] Architect + Editor model separation
- [ ] Git auto-commit with intelligent messages
- [ ] `/undo` to revert last AI change
- [ ] Multi-file context management (`/add`, `/drop`)
- [ ] Voice input (optional, via Whisper API)
- [ ] Watch mode for file changes

### 9.2 CLI Polish
- [ ] Full slash command set
- [ ] Keyboard shortcuts
- [ ] Progress indicators and spinners
- [ ] Beautiful diff display
- [ ] Cost/token counter in status bar

### 9.3 Distribution
- [ ] Cross-platform binaries (Linux, macOS, Windows)
- [ ] Install scripts (curl | sh)
- [ ] Homebrew formula
- [ ] npm wrapper package (optional)

---

## Key Files to Create

### Rust Crates
| File | Purpose |
|------|---------|
| `crates/agentik-core/src/lib.rs` | Core types, re-exports |
| `crates/agentik-core/src/config.rs` | Figment-based config |
| `crates/agentik-core/src/message.rs` | Conversation primitives |
| `crates/agentik-providers/src/traits.rs` | Provider trait definitions |
| `crates/agentik-providers/src/anthropic.rs` | Claude implementation |
| `crates/agentik-providers/src/openai.rs` | GPT implementation |
| `crates/agentik-session/src/store.rs` | SQLite session storage |
| `crates/agentik-session/src/compaction.rs` | Context summarization |
| `crates/agentik-repomap/src/parser.rs` | Tree-sitter integration |
| `crates/agentik-repomap/src/ranking.rs` | PageRank implementation |
| `crates/agentik-agent/src/agent.rs` | Main agent loop |
| `crates/agentik-agent/src/planning.rs` | Planning mode logic |
| `crates/agentik-agent/src/questions.rs` | Anytime question system |
| `crates/agentik-cli/src/main.rs` | CLI entry point |
| `crates/agentik-cli/src/tui/mod.rs` | Ratatui interface |

### Configuration Files
| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace manifest |
| `CLAUDE.md` | Dev instructions |
| `data/prompts/system.md` | System prompt |
| `data/pricing.json` | Provider pricing |

---

## Verification Plan

### Unit Tests
- Provider abstraction with mock responses
- Session persistence round-trips
- Config parsing and merging
- Token counting accuracy

### Integration Tests
- End-to-end chat flow with local Ollama
- Session resume after restart
- MCP server connection
- Git operations in test repo

### Manual Testing Checklist
1. `agentik` - Start new session, send message, verify response
2. `agentik -c` - Continue previous session
3. `/add src/` - Add files to context, verify token count
4. `/plan` - Enter planning mode, verify read-only
5. `/model gpt-4o` - Switch models mid-session
6. `/cost` - View session costs
7. `/compact` - Trigger compaction, verify context reduction
8. Git auto-commit - Make changes, verify commit message
9. `/undo` - Revert last AI commit

### Benchmarks
- Response latency vs. direct API calls
- Memory usage during long sessions
- Compaction effectiveness (token reduction)

---

## Dependencies

### Rust Crates
```toml
[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
ratatui = "0.28"
crossterm = "0.28"
indicatif = "0.17"
syntect = "5"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
reqwest = { version = "0.12", features = ["json", "stream"] }
rusqlite = { version = "0.32", features = ["bundled"] }
figment = { version = "0.10", features = ["toml", "env"] }
tree-sitter = "0.24"
async-trait = "0.1"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
```

### TypeScript (MCP Servers)
```json
{
  "dependencies": {
    "@modelcontextprotocol/sdk": "^1.0.0",
    "zod": "^3.0.0"
  }
}
```

---

## Success Criteria

### MVP (Feature Parity with Aider)
- [ ] Multi-provider support (Anthropic, OpenAI, Ollama)
- [ ] Repository map with intelligent file ranking
- [ ] Git integration with auto-commit
- [ ] Session persistence and resume
- [ ] Context compaction
- [ ] Planning mode
- [ ] Cost tracking

### Post-MVP Enhancements
- [ ] Voice input
- [ ] MCP server ecosystem
- [ ] Model benchmarking dashboard
- [ ] Plugin architecture
- [ ] IDE integrations
