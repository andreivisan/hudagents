# Repository Guidelines

This repository is a Rust-based open-source AI Agents Framework, aiming to combine graph-oriented orchestration (LangGraph-style) with inter-agent 
communication patterns (AutoGen-style). Contributions should prioritize correctness, performance, security, privacy, and minimalism.

## Project Structure & Module Organization
- `crates/` — different modules part of the whole AI Agent Framework.
  - `crates/hudagents-capture/` — util code meant for capturing image and sound on the local machine for demo purposes.
  - `crates/hudagets-core/` — agent, context, graph API.
  - `crates/hudagents-local` - local agents defintions and tools.
  - `crates/hudagents-tools/` — CLI to check System Info and download the right Whisper local model.  

## Build, Test, and Development Commands
- `cargo fmt` — format code (required before PR).
- `cargo clippy --all-targets --all-features -D warnings` — lint; warnings are treated as errors.
- `cargo test` — run unit + integration tests.
- `cargo test -- --nocapture` — show logs for debugging.
- `cargo bench` — run benchmarks (when present).
- `cargo doc --no-deps --open` — generate and view docs locally.

## Coding Style & Naming Conventions
- Rust 2024 edition. Prefer idiomatic Rust with small, composable modules.
- Formatting via `rustfmt`; lint via `clippy`.
- Naming:
  - Types/traits: `PascalCase` (e.g., `AgentRuntime`)
  - Functions/vars/modules: `snake_case` (e.g., `route_message`)
  - Constants: `SCREAMING_SNAKE_CASE`
- Design rules (in priority order):
  1. **0 bugs**: correctness > cleverness.
  2. **Security & privacy**: least-privilege APIs, avoid leaking secrets/PII, validate inputs.
  3. **Speed & memory efficiency**: minimize allocations/copies; measure before optimizing.
  4. **Minimal dependencies**: prefer std; add crates only when they significantly reduce risk or complexity.
  5. **Beauty**: small, readable functions; clear types; expressive errors.

## Testing Guidelines
- Tests should be deterministic and runnable offline where possible.
- Prefer:
  - Unit tests near the code (`mod tests { ... }`)
  - Integration tests in `tests/` for cross-module behavior
- Name tests for intent: `it_routes_messages_by_topic()`, `graph_executes_in_topological_order()`.
- Add a regression test for every bug fix.

## Agent-Specific Instructions (Codex)
Codex should act as a **teacher**, not a copy/paste solution generator.
- Default response style:
  - Explain the concept and tradeoffs first.
  - Provide **small, focused Rust examples** (not a full finished feature) that illustrate the approach.
  - Ask “checkpoints” to confirm understanding and propose next steps.
- Assume the contributor is an enterprise full-stack engineer (Java/Python background). Prefer analogies when helpful (traits vs interfaces, 
ownership vs GC).
- If a request could be answered with a large dependency, first propose a minimal-std approach; suggest a crate only if it clearly reduces risk/bugs.
