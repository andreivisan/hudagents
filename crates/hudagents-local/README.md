# hudagents-local

`hudagents-local` provides **local AI backends** for the HudAgents system.

It is responsible for wiring together **local speech-to-text** and **local vision LLMs** so that the rest of the system can talk to a single `AgentBackend` implementation from `hudagents-core`.

> High-level: this crate gives you a “local-only brain” for Hudward — audio + image in, answer out — without depending on any cloud model.

## 1. Local agents (concept)

To begin with, we support **two local components**:

1. **Whisper (speech-to-text)**  
   - Transcribes audio from the user into text.
   - Implemented via `whisper.cpp` (or compatible HTTP wrapper).

2. **Qwen3-VL (vision LLM)**  
   - Takes:
     - the transcription from Whisper,
     - the current image (if any),
     - sliding-window conversation context (from `hudagents-core`),
   - and returns a textual reply.
   - Implemented via **Ollama**, using a Qwen3-VL model.

These are **local agents** to give users the option of *full privacy* when they run everything on their own machines or servers.

Internally, this crate will expose a **single orchestrator type** (e.g. `LocalAgent`) that implements `AgentBackend` and calls Whisper + Qwen3-VL under the hood.  
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
  - Pulling the desired Qwen3-VL models via Ollama (`ollama pull ...`).
- The backend is responsible for:
  - Detecting whether a suitable **Whisper model** is available.
  - If not, using hudagents-tools' CLI to download the **Whisper model** that suits best the user's system configuration.
  - Reporting clear errors if local services are unreachable.
