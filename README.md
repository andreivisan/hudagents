# HUDAGENTS

Most beautifuly and efficiently written AI Agents Framework centered around Computer Vision.

## Principles

My development for hardware and software has the following MUSTS:
- minimal dependencies for memory efficiency (like we said above if it takes too much code we will use dependencies)
- elegant with as little as possible code to solve problems
- speed is vital
- security is vital
- 0 bugs is vital
- privacy is vital
- beauty is essential

## Privacy first

In order to protect user's privacy this framework support local AI Agents as well as Cloud AI Agents.
For local AI agents documentation please see [Local Agents docs](https://github.com/andreivisan/hudagents/blob/main/crates/hudagents-local/README.md)

## Architectural notes

### Runtime behavior

1. **LangGraph** is using an explicit graph, **AutoGen** is using a conversation-driven model. 
HudAgents builds a default task graph (suggested by an LLM ‘planner’ agent) from the user’s initial query. Users can then modify or extend this graph.

2. Currently the only type of memory supported is `**Short-Term Memory**` using a `**Context Queue**`.

### Agents characteristics

1. Each agent can maintain its own context queue of recent messages and use a shared memory if they work in a team. Future versions will use a vector database.

2. **Local vs Remote agents** - HudAgents will let use choose per agent whether to use a local model or a cloud service. That way the user has control over what data is shared and what stays local.

3. Each agent in HudAgents can be designed as a modular component with a defined role as one of the two:
    - **Stateless tools** (pure functions: “transcribe this”, “detect objects”).
    - **Stateful agents** (have memory, persona, internal goals).

### Data model

Starting with future versions:
    - `protobuf` will be used for message passing and graph structure.
    - `JSON` will be used for optional debug export or external logs.

Currently internal Rust structs will be used.

### Orchestration

- Using a DAG implementing Khan's algorithm to decided the order of execution and parallelism.
- Using a FSM to create cycles of execution in a DAG.
- Using a modified Actor Model for messaging between agents:
    1. Task-Handle split with Message enums to define the "protocol" of interaction.
    2. Bounded Mailboxes and Backpressure to support heavy load.
    3. Group chat and Manager Actors: Round-Robin for local models (to still have speed) and LLM-based selector for the cloud option.

### Tooling & Visualisation

- HudAgents will include introspection capabilities to trace and visualize the agent behaviors at different verbosity levels.
- In practice, this means implementing a logging or debug flag (e.g. a GRAPH and DEBUG level) that, when enabled, outputs the internal decision graph or conversation trace to the console.
- Besides logging, we might allow exporting the agent interaction graph (perhaps as a data structure or even a Graphviz diagram) at runtime when debugging.
- Future versions will include a GUI for the user to visualise the Graph flow.
- To make HudAgents user-friendly, a builder pattern for constructing agent graphs is preferred.

### Failure handling

- We will start with retry + configurable failure behavior.
- HudAgents will allow the user to configure how failures are handled, rather than baking in one policy.
- This means exposing settings like: max retries, fallback actions, or human intervention triggers.
- Built-in Error Recovery: On the framework side, we’ll incorporate some automatic error-handling capabilities so that common failures don’t always need human intervention. In the future this will also include time-travel debugging, meaning the system can backtrack to a prior state if something goes wrong.

## Future Versions

- Google's Pregel algorithm to support vertex parallel execution and support cycles. At the moment we are using Kahn's algorithm for parallel processing as no loops are supported at the moment. (Or maybe just use explicit loop constructs).
- Pre-processed flows based on functionality: for example an agentic flow optimised for coding, another one for studying.
- Pre-processed flows based on vendor: flows that are using the OpenAI ecosystem, or Anthropic, or just a gathering of the best open source AI agents. 
