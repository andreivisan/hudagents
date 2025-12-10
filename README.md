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

