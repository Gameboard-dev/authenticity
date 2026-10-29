# Disclosures and compliance

[← README](../README.md)

Obsidian's [Developer policies](https://docs.obsidian.md/community-directory/developer-policies) ask a plugin to say
clearly whether it needs payment or an account, which remote services it uses and why, whether it reaches files outside
the vault, whether it shows ads, and how any data it sends is handled. This page goes through each. How the data is
protected is on [Privacy and security](privacy-and-security.md).

## In short

**Authenticity is offline by default.** All three checks run on your computer: the plagiarism check compares your text
with your own library notes; the AI-writing check uses SlopTotal, a detector you install yourself, which Authenticity
runs on `127.0.0.1` with downloads switched off; Reveal AI characters is fixed rules. Out of the box Authenticity makes
no request to any address but `127.0.0.1`, needs no account, has no payment, and collects nothing about you.

**The one exception is optional and off by default: GPTZero**, an online AI detector. Only if you link your own GPTZero
API key does Authenticity ask, before each AI check, whether to use it; only when you choose GPTZero is the checked text
sent to GPTZero's servers, and it counts against your paid GPTZero plan. Without a key nothing is ever sent.

## Network use

| Connection | When | What is sent |
|---|---|---|
| Authenticity's helper, `127.0.0.1` (port 8791 by default) | When you run an AI check, or ask the AI Detector pane for reasons or sentence scores | The paragraph or sentence being scored |
| SlopTotal, `127.0.0.1` (port 8000 by default), through the helper | The same | `{"text": ...}` only |
| `api.gptzero.me` (HTTPS), **optional** | Only with a GPTZero key linked: for a check where you chose GPTZero when asked; *Check the key*; and the question before a check | The checked paragraphs' text, one request per paragraph or group, with your key in a header; a paragraph's text again when you press *AI Patterns* in its row; *Check the key* and the question send the key alone (no text, no words used) |
| The optional embedding search, `127.0.0.1` (port 8766 by default), **optional** | Only with *Embedding search* on, when you check for plagiarism | The sentences not already matched word for word |
| huggingface.co | **Once, at installation, started by you** ([Installation](installation.md#3-sloptotal)) | SlopTotal's requests for its models (about 9 GB). Authenticity never starts this |

What GPTZero says it does with the text it receives: [Privacy and security](privacy-and-security.md#gptzero).

## What is kept, and where

| What | Where | Synchronised with the vault? |
|---|---|---|
| Settings (colours, plagiarism options, the library folder, the name of the keychain entry) | `.obsidian/plugins/authenticity/data.json` | Yes |
| This computer's settings (SlopTotal folder, device, ports, idle time, helper program), and the log pane's *Wrap lines* | Obsidian's local storage | No |
| AI scores of the paragraphs you checked, the detectors' reasons, sentence scores, and GPTZero's results when you chose it, kept apart (by a hash of the text; no other text) | `.obsidian/plugins/authenticity/ai.json` | Yes |
| Plagiarism results: what the last check of each note found (the matched words and sentences of your note and of the source, paraphrase candidates), and what passes the thresholds now | `.obsidian/plugins/authenticity/found.json`, `matches.json` | Yes |
| What the optional embedding search said about each sentence asked about (by a hash of the sentence; the source sentences found), used only by the plugin API | `.obsidian/plugins/authenticity/paraphrase.json` | Yes |
| Your GPTZero API key | Obsidian's keychain (Settings → Keychain); never in Authenticity's files, history or logs | As Obsidian keeps its keychain |
| The library index (word-sequence hashes and content-word stems of the library notes' passages, their paths and modification times; no text) | `<local data>/authenticity/<vault id>/library.idx` | No |
| The history of AI checks ([History and logs](history-and-logs.md)) | `<local data>/authenticity/history/<vault id>.jsonl` | No |
| The debug log, only while *Debug log* is on | `<local data>/authenticity/logs/authenticity-debug.log` | No |
| SlopTotal's output log | `<local data>/authenticity/logs/sloptotal.log` | No |
| The GPU claim while SlopTotal holds the graphics card | `<local data>/obsidian-kit/gpu/authenticity.json` | No |
| The status file other local tools can read (whether Authenticity is enabled, its version, features, MCP tool names; no text) | `<local data>/obsidian-kit/companions/authenticity.json` | No |
| SlopTotal's own copy of each text it scores, for one day, then deleted by SlopTotal | SlopTotal's folder (`data`) | No |

`<local data>` is `%LOCALAPPDATA%` on Windows (`$XDG_DATA_HOME` or `~/.local/share`, and for the index
`$XDG_CACHE_HOME` or `~/.cache`, elsewhere). *Clear this note's results* removes a note's scores and matches.

## Files and programs outside the vault

Authenticity is desktop-only. Besides the files above, it:

- **reads SlopTotal's folder**, to find its Python and PyTorch build and its calibration (never changed);
- **reads its own log files** in its local data folder for the log pane in the settings, read-only
  ([History and logs](history-and-logs.md#the-log-pane));
- **runs two programs**, each from a path set on this computer, never from the vault's settings: the helper
  (`authenticity-helper`, in the plugin's folder unless you set another) and, through the helper, SlopTotal's own
  Python. Both are started with an argument list, never through a shell;
- reads the graphics card's free memory through NVIDIA's driver library when there is one.

These are needed because SlopTotal's models are too large to run inside Obsidian, and the history and index are kept
out of the vault so they are not synchronised.

## SlopTotal

SlopTotal is a separate open-source program you install yourself ([Installation](installation.md#3-sloptotal)).
Authenticity uses only its local scoring API, starts it with `--host 127.0.0.1`, `HF_HUB_OFFLINE=1`,
`TRANSFORMERS_OFFLINE=1`, `HF_HUB_DISABLE_TELEMETRY=1` and `SLOPTOTAL_RETENTION_DAYS=1`, and stops it after the idle
time you set, when Obsidian closes, or when another of your plugins needs the graphics card it holds. A SlopTotal you
started yourself is used as it is and never stopped. The parts of SlopTotal that are never used:
[AI detection](ai-detection.md#what-sloptotal-does-that-authenticity-does-not-use).

## The optional embedding search

If the Quarry plugin is installed, Authenticity can optionally use its embedding model for the paraphrase check
(*Embedding search*, off by default). It then sends sentences to that plugin's service on `127.0.0.1`; nothing leaves
the computer.

## The assistant tools

`authenticity-mcp` (for research assistants) and `check-overlap` (the command line) are separate programs you run or
register yourself ([Assistant tools](assistant-tools.md)). They read the vault's notes and Authenticity's results from
disk, use the same helper and SlopTotal settings, and never use GPTZero.

## Payment, accounts, ads, telemetry

Authenticity is free, needs no account and no key, shows no ads, and sends no telemetry. The one optional exception is
GPTZero: using it needs a GPTZero account and a paid API plan, bought from GPTZero, not through Authenticity; without
it every feature works.

## No self-installation or self-updating

Authenticity never downloads or installs anything: not the helper, not SlopTotal, not its models. The installation guide
says what to install and where; you do it.

## Source code and licences

Authenticity is open source under the MIT licence. SlopTotal and its models are separate projects under their own
licences, and GPTZero is a commercial service under its own terms ([Licences](licences.md)).
