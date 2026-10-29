# Authenticity

**Writing checks for Obsidian that run on your own computer.**

Authenticity shows how far your paragraphs read like AI-generated text, which wording you share with the sources in
your library, and which hidden or look-alike characters a text carries. It marks and explains; it **never rewrites
your text** and never suggests a rewrite. The only changes it makes are character replacements you ask for, each
confirmed first.

## Disclosures (Obsidian's developer policies)

What Authenticity sends over the network, stores outside the vault and runs, as Obsidian's
[Developer policies](https://docs.obsidian.md/community-directory/developer-policies) ask, is set out item by item in
**[docs/disclosures.md](docs/disclosures.md)**. In short:

- **Network use:** none by default. Every check runs on your computer and talks only to `127.0.0.1`. The one exception
  is optional: **GPTZero**, an online AI detector, used only if you link your own API key and choose it when asked
  before a check; the checked text alone is then sent to GPTZero's servers
  ([privacy and security](https://gptzero.me/privacy-and-security), [terms](https://gptzero.me/terms-of-use.html)).
- **Payment and accounts:** none needed for any feature. GPTZero, if you choose to use it, needs your own GPTZero
  account and paid API plan.
- **Files outside the vault:** Authenticity runs its helper program and SlopTotal (the local AI detector you install
  yourself), reads SlopTotal's folder, and keeps its library index, history and logs in your local data folder.
- No telemetry, no ads, no self-updating: you download and install everything yourself.

## Contents

- [Features](#features)
- [Requirements](#requirements)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Documentation](#documentation)
- [Licence](#licence)

## Features

- **AI-writing check**: each paragraph scored by [SlopTotal](https://github.com/pablocaeg/sloptotal), a local
  AI-text detector (23 detectors combined into one score), on one scale from 0 (reads as written by a person) to 100
  (reads as AI), with the detectors' reasons in collapsed blocks and each sentence's own score. Optionally
  [GPTZero](https://gptzero.me) instead, chosen for each check.
- **Plagiarism check**: runs of six or more words you share with a library note (verbatim and near-verbatim,
  quotations left out), the field's fixed phrasing listed apart, and close paraphrases (a sentence that keeps a
  library sentence's particular words in its order), found by a search over the library's words with no model.
- **Reveal AI characters**: zero-width spaces and joiners, byte-order marks, soft hyphens, direction marks, unusual
  spaces, dashes, look-alike punctuation and letters, and styled Unicode letters, by fixed rules; replace them one by
  one or for a whole selection.
- **Two side panes**: the AI Detector (scores, reasons, sentences, history) and Plagiarism (each of your sentences
  beside the source's, with a link to the source note).
- **Tools for assistants**: a plugin API, an MCP server and a command-line check of a draft.

## Requirements

- **Desktop Obsidian 1.11.4 or later.** The helper program is built for Windows (x86-64); other systems need a build
  from source ([Building](docs/building.md)).
- **For the AI-writing check:** SlopTotal, installed once (about 11 GB of disk, about 9 GB of free memory while a check
  runs; an NVIDIA card with more than 9 GB makes it faster).
- **For the plagiarism check:** a folder of your source notes in the vault.

## Installation

1. Install the plugin from Obsidian's community plugins, or copy `main.js`, `manifest.json` and `styles.css` from a
   [release](https://github.com/Gameboard-dev/authenticity/releases) into `<vault>/.obsidian/plugins/authenticity/`.
2. Download the **helper program** `authenticity-helper.exe` from the
   [native release](https://github.com/Gameboard-dev/authenticity/releases/tag/native-0.3.0) and put it in the
   plugin's folder. It starts and stops SlopTotal; the other checks do not need it.
3. **Install SlopTotal** and set its folder under Settings → Authenticity → Writing checks.
4. Set **Library folder** under Settings → Authenticity → Plagiarism to the folder of your source notes.

Step by step: [Installation](docs/installation.md).

## Quick start

1. Select a paragraph or two, right-click, and choose **Check for AI writing**. The AI Detector pane opens with each
   paragraph's score; *Load reasons* shows why.
2. Right-click again and choose **Check for plagiarism**. The Plagiarism pane lists each sentence that shares wording
   with a library note, beside the source's sentence.
3. Choose **Reveal AI characters** to mark hidden and look-alike characters; click one in the AI Detector pane to step
   through them.

The commands of the same names check the whole note ([Using Authenticity](docs/usage.md)).

## Documentation

| Page | What it covers |
|---|---|
| [Disclosures and compliance](docs/disclosures.md) | Network use, what is kept and where, files outside the vault, payment, accounts, and each of Obsidian's rules |
| [Installation](docs/installation.md) | The plugin, the helper, SlopTotal, the library folder, the optional GPTZero key and embedding search, removing it all |
| [Using Authenticity](docs/usage.md) | The menus, commands, panes, marks and colours |
| [AI detection](docs/ai-detection.md) | How SlopTotal scores, the scale, reasons and writing patterns, short paragraphs, GPTZero, and how reliable the scores are |
| [Similarity](docs/similarity.md) | Copied wording, fixed phrasing, close paraphrase, the thresholds, the library index, the optional embedding search |
| [Characters](docs/characters.md) | What Reveal AI characters finds, and how replacing them works |
| [Settings](docs/settings.md) | Every setting and what it does |
| [History and logs](docs/history-and-logs.md) | The history of AI checks, the debug log and SlopTotal's log |
| [Assistant tools](docs/assistant-tools.md) | The plugin API, the status file, the MCP server, check-overlap and the helper's interface |
| [Privacy and security](docs/privacy-and-security.md) | What leaves the plugin, what GPTZero says it does with text, and how the helper is protected |
| [Troubleshooting](docs/troubleshooting.md) | Common problems, and the known limits |
| [Building from source](docs/building.md) | Building the plugin and the native programs, and the tests |
| [Licences](docs/licences.md) | Authenticity's licence, third-party licences, SlopTotal's models |

## Licence

MIT ([LICENSE](LICENSE)). SlopTotal and its models are separate projects under their own licences, and GPTZero is a
commercial service under its own terms: [Licences](docs/licences.md).
