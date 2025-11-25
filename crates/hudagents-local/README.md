# Local Agents

## Agents description

- We need to have 2 local agents to begin with:
    1. Whisper - transcribe audio to text. 
    2. Gemma - Process image and besed on the transcription received from Whisper returns a reply.

These are local agents to give the user the option for full privacy.

## Features

- The user downloads local models on their own servers.
- The user configures which models they will use in the mobile app by providing.
    1. Model name
    2. Model URL
- Based on these configurations the Rust backend will connect to those URL's and use user's models.
- User has the option to clone the repo and run the application locally.
    - For this case the user can config un the defaults the model they will use.
    - For this case the hudagents-local crate will provide an option to download Whisper model for the user as it is not part of the Ollama project.
    - Everything else will be configured through Ollama.