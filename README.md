# HUDAGENTS

Most beautifuly and efficiently written AI Agents Framework centered around Computer Vision.

## Privacy first

In order to protect user's privacy this framework support local AI Agents as well as Cloud AI Agents.
For local AI agents documentation please see [Local Agents docs](https://github.com/andreivisan/hudagents/blob/main/crates/hudagents-local/README.md)

## Architectural notes

### Runtime behavior

1. `**LangGraph**` is using an explicit graph, `**AutoGen**` is using a conversation-driven model. We are using what is called a ***Super Optimal Suggested Graph***. We simply generate an optimal *default* `graph` of agents based on the user's initial query. The user has then the possibility to change the initial `graph`.

2. Currently the only type of memory supported is `**Short-Term Memory**` using a `**Context Queue**`.

### Agents characteristics

1. Each agent can maintain its own context queue of recent messages and use a shared memory if they work in a team. Future versions will use a vector database.

2. **Local vs Remote agents** - HudAgents will let use choose per agent whether to use a local model or a cloud service. That way the user has control over what data is shared and what stays local.

3. Each agend in HudAgents can be designed as a modular component with a defined role.

### Data model

- `protobuf` will be used for message passing and graph structure.
- `JSON` will be used for optional debug export or external logs.

### Tooling & Visualisation

- HudAgents will include introspection capabilities to trace and visualize the agent behaviors at different verbosity levels.
- In practice, this means implementing a logging or debug flag (e.g. a GRAPH and DEBUG level) that, when enabled, outputs the internal decision graph or conversation trace to the console.
- Besides logging, we might allow exporting the agent interaction graph (perhaps as a data structure or even a Graphviz diagram) at runtime when debugging.
- Future versions will include a GUI for the user to visualise the Graph flow.
- To make HudAgents user-friendly, a builder pattern for constructing agent graphs is preferred.

### Failure handling

- HudAgents will allow the user to configure how failures are handled, rather than baking in one policy.
- This means exposing settings like: max retries, fallback actions, or human intervention triggers.
- Built-in Error Recovery: On the framework side, we’ll incorporate some automatic error-handling capabilities so that common failures don’t always need human intervention. This will also include time-travel debugging, meaning the system can backtrack to a prior state if something goes wrong.

