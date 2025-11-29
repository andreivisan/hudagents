# hudagents-tools

This crate includes tools and utilities to help the AI Agents or the user of this framework.

## 1. Whisper Download CLI

This CLI tool has 2 options:

### --sysinfo

Used with `--sysinfo` paramenter the CLI will return a detailed and relevant system info of the user's machine.

Along with the system info the tool will recommend the user which version of the Whisper model would suit best the configuration.

**Usage**

```bash
hudagents-tools sysinfo
```

### --download

If the user already knows which version of the Whisper model suits their configuration best then the CLI should be used with `--download` followed by the name of the Whisper model and an optional `--path` for where the user desiders to download the model.

If `--path` is not provided then by default the chosen model will be downloaded under `hudagents/.models`.

**Usage**

```bash
hudagents-tools download --model large --path /tmp
```