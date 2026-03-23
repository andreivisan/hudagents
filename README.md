# HudAgents

Rust AI agent framework for privacy-aware, computer-vision-first workflows, combining DAG orchestration, FSM control flow, 
actor-style execution, and local/cloud model routing.

> Status: experimental / pre-alpha. The core graph primitives and local-model building blocks exist, but APIs and docs are 
still evolving.

## Why HudAgents?

- Privacy-first local and cloud agents, with routing decisions made per agent.
- Structured orchestration built around DAG execution, FSM-style control, and actor-like messaging.
- Clear separation between stateless tools and stateful agents.
- Built for computer vision, speech-to-text, and wearable AI workflows.
- Correctness, reliability, privacy, and minimal dependencies are first-class goals.

## Workspace Overview

- `hudagents-core`: agent traits, context primitives, graph building blocks, and runtime foundations.
- `hudagents-local`: local speech-to-text and vision backends.
- `hudagents-tools`: CLI utilities for system inspection and Whisper model downloads.
- `hudagents-capture`: local image and audio capture helpers for demos.

## Quick Start

Prerequisites:

- Rust stable toolchain
- `ffmpeg` for speech-to-text workflows
- `Ollama` if you want to experiment with the local vision stack from `hudagents-local`

```bash
git clone https://github.com/hudward/hudagents
cd hudagents
cargo build
cargo test
cargo run -p hudagents-tools -- sysinfo
```

To download a Whisper model after checking your system:

```bash
cargo run -p hudagents-tools -- download --model tiny.en
```

## Minimal Example

The graph API is still evolving, but the current core already supports building layered DAG execution plans:

```rust
use hudagents_core::agent::{Agent, AgentInput, AgentOutput, HAAgentError};
use hudagents_core::graph::{GraphBuilder, HAGraphError};
use std::sync::Arc;

struct EchoAgent;

impl Agent for EchoAgent {
    fn id(&self) -> &str {
        "echo"
    }

    fn call(&self, input: AgentInput) -> Result<AgentOutput, HAAgentError> {
        Ok(match input {
            AgentInput::Text(text) => AgentOutput::FinalAnswer(text),
            _ => AgentOutput::FinalAnswer("unsupported input".to_string()),
        })
    }
}

fn main() -> Result<(), HAGraphError> {
    let worker = Arc::new(EchoAgent);

    let mut builder = GraphBuilder::new();
    let ingest = builder.add_node("ingest", worker.clone());
    let respond = builder.add_node("respond", worker);

    builder.add_edge(ingest, respond)?;

    let graph = builder.build()?;
    assert_eq!(graph.layers.len(), 2);

    Ok(())
}
```

## Core Concepts

### DAG + FSM orchestration

HudAgents uses a graph-oriented execution model for dependency ordering and parallelism, with FSM-style control for retries,
skips, and other non-linear runtime decisions.

### Tools vs stateful agents

HudAgents distinguishes between narrow stateless tools and longer-lived stateful agents that carry recent context and 
role-specific behavior.

### Local vs cloud privacy model

HudAgents is designed for per-agent local/cloud routing so privacy decisions stay explicit and sensitive workflows can stay
local when needed.

### Failure handling and observability

Failure handling and introspection are part of the framework design, not afterthoughts. Please refer to the architecture docs.

## Project Status

HudAgents is still early. The repository already contains graph primitives, local-agent building blocks, and a 
model-management CLI, but it is not yet a polished end-user framework release.

Current state:

- Core graph construction and cycle detection exist in `hudagents-core`.
- Local Whisper and local-model integration work is underway in `hudagents-local`.
- The first practical user entry point today is `hudagents-tools`.

## Documentation

- [hudagents-core](crates/hudagents-core/README.md)
- [hudagents-local](crates/hudagents-local/README.md)
- [hudagents-tools](crates/hudagents-tools/README.md)

Planned docs:

- Architecture
- Roadmap
- Examples

## Contributing

Contributions are not yet opened.

## License

Licensed under `MIT OR Apache-2.0`.
