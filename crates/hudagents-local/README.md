# hudagents-local

`hudagents-local` provides **local AI backends** for the HudAgents system.

It is responsible for wiring together **local speech-to-text** and **local vision LLMs** so that the rest of the system can talk to a single `AgentBackend` implementation from `hudagents-core`.

> High-level: this crate gives you a “local-only brain” for Hudward — audio + image in, answer out — without depending on any cloud model.

## 1. Local agents (concept)

To begin with, we support **two local components**:

1. **Whisper (speech-to-text)**  
   - Transcribes audio from the user into text.
   - Implemented via `whisper.cpp` (or compatible HTTP wrapper).

2. **Gemma (vision LLM)**  
   - Takes:
     - the transcription from Whisper,
     - the current image (if any),
     - sliding-window conversation context (from `hudagents-core`),
   - and returns a textual reply.
   - Implemented via **Ollama**, using a Gemma model (e.g. `gemma:2b` or similar).

These are **local agents** to give users the option of *full privacy* when they run everything on their own machines or servers.

Internally, this crate will expose a **single orchestrator type** (e.g. `LocalAgent`) that implements `AgentBackend` and calls Whisper + Gemma under the hood.  
That API will be documented once the implementation stabilizes.

## 2. Deployment modes

There are two main ways `hudagents-local` is intended to be used.

### 2.1 Local development mode (clone & run)

This is the “developer convenience” scenario:

- The user clones the HudAgents repo and runs the backend locally.
- They have **Ollama** installed and running on their machine.
- They choose to use **local models** for maximum privacy / offline capability.

In this mode:

- The user is responsible for:
  - Installing and running **Ollama**.
  - Pulling the desired Gemma models via Ollama (`ollama pull gemma:...`).
- The backend is responsible for:
  - Detecting whether a suitable **Whisper model** is available.
  - If not, offering to **download the appropriate Whisper model** based on configuration (e.g. model size, language).
  - Reporting clear errors if local services are unreachable.

The Whisper model download helper will likely be exposed via **CLI tools** belonging to this crate (or workspace), *not* hard-wired into the core library logic.  
Example idea (not final API):

```bash
hudagents-local whisper download --model base.en --dest ~/.hudagents/models
```
