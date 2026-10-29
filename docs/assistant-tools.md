# Assistant tools

[← README](../README.md)

Authenticity offers its checks to other plugins (a plugin API), to research assistants such as Claude (an MCP server),
and to the command line (check-overlap). All of them score AI writing with SlopTotal only, never GPTZero: GPTZero is
used only for a check started in Obsidian, after you choose it when asked ([AI detection](ai-detection.md#gptzero-optional)).
None of them changes a note.

## The plugin API

`app.plugins.getPlugin("authenticity")?.api`, set once the plugin has loaded (and removed when it unloads), each time
followed by `app.workspace.trigger("authenticity:api-changed")`.

| Member | Type | What it is |
|---|---|---|
| `version` | number | `1`. Raised only when a member changes meaning or goes |
| `available()` | `() => boolean` | Whether its checks can run now: synchronous, no request, no model load |
| `features` | string array | What it offers now: `"ai_score"` (once this computer has a SlopTotal folder), `"similarity"`, `"invisible_characters"` |
| `status()` | `() => object` | What can run and why not: `{version, ai: {ok, why?, sloptotal?}, similarity: {ok}, paraphrase: {ok, why?}, characters: {ok}}` |
| `checkAi(paragraphs)` | `(string[]) => Promise` | `{paragraphs: [{score, verdict, confidence, class, describe} or {skipped}]}`. Starts SlopTotal if needed, asking the GPU question in Obsidian if the card is short. At most 500 paragraphs |
| `explainAi(text)` | `(string) => Promise` | SlopTotal's analysis: `{overall, verdict, flagged, total, language, languageSupport, reasons: [{engine, score, verdict, details, description}]}` |
| `checkSimilarity(text, {mode, note})` | `(string, object) => Promise` | `{matches: [...], paraphrase_not_checked}` ([below](#checksimilarity)) |
| `revealCharacters(text, {typographic})` | `(string, object) => object[]` | `[{from, to, code, name, kind}]`, synchronous |
| `hints(sentence, paragraphSentences)` | `(string, string[]) => object[]` | The model-free hints: `[{id, label, found, why, spans}]` (byte ranges of the sentence) |

A Promise rejects with an `Error` whose message says what to do.

### checkSimilarity

`mode` is `"verbatim"`, `"paraphrase"` or `"both"` (default); `note` is the vault path of the note the text is from
(never its own source). Each match is `{kind, text, from, to, score, cosine, source, title, sourceText, also, sources,
page, line}`, with UTF-16 places in `text`:

- for a close paraphrase, `score` is the share of the sentence's content words kept in the source sentence's order,
  `cosine` the two sentences' similarity when the optional embedding search was used (else absent), and `sourceText`
  the source sentence;
- a verbatim or near-verbatim match also has `matched`, `sentence`, `sentenceAt`, `sourceSentence` and `sourceAt`: the
  words matched exactly, and the sentences of the text and of the source holding the run.

The matches are filtered by the user's thresholds, as the Plagiarism pane shows them ([Similarity](similarity.md#the-thresholds)).

## The status file

`<local data>/obsidian-kit/companions/authenticity.json` (`%LOCALAPPDATA%` on Windows, `$XDG_DATA_HOME` or
`~/.local/share` elsewhere), written when the plugin loads and every 60 seconds while it is enabled, and deleted when it
unloads. It lets local tools see whether Authenticity is enabled, and holds no text:

```json
{ "name": "authenticity", "version": "0.3.0", "api": 1, "enabled": true, "updated": 1793241580,
  "features": ["ai_score", "similarity", "invisible_characters"],
  "mcp": { "server": "authenticity", "tools": ["check_ai", "explain_ai", "check_similarity", "reveal_characters", "check_overlap", "status", "list_notes", "ai_results", "similarity_results", "unusual_characters"] },
  "port": 8791 }
```

`port` is the helper's while it runs (0 otherwise).

## The MCP server

`authenticity-mcp` (from the [native release](https://github.com/Gameboard-dev/authenticity/releases/tag/native-0.3.0),
or `native/target/release/` after a build) speaks JSON-RPC 2.0 over standard input and output. Register it with your
assistant as a stdio server:

```text
authenticity-mcp --vault <vault folder> [--config-dir .obsidian] [--sloptotal <SlopTotal folder>]
                 [--sloptotal-port 8000] [--helper-port 8791] [--device auto|gpu|cpu] [--idle-minutes 10]
```

Without `--sloptotal`, `%LOCALAPPDATA%\Programs\SlopTotal` is used when SlopTotal is there. The server uses the helper
Obsidian started if one answers on the helper port; otherwise it runs one in its own process and stops SlopTotal when
it stops.

| Tool | What it does |
|---|---|
| `status` | The helper and SlopTotal (state, device), the library folder and its notes, the paraphrase search |
| `check_ai(paragraphs, gpu?)` | Each paragraph's score (0 to 100), verdict, colour class and description, and its sentences' hints with UTF-16 spans. Paragraphs under 50 characters are skipped. More words than the user's limit answers `needs_confirmation` |
| `explain_ai(text, gpu?)` | SlopTotal's overall score and the detectors' reasons: `counted` (by weight, each with its weight) and `not_counted_for_this_language` |
| `check_similarity(text, mode?, note?, gpu?)` | `matches` (verbatim, near-verbatim, paraphrase: kind, span, score, source note, title, page, the source's words), `fixed_phrasing`, and how `paraphrase` was checked |
| `check_overlap(draft, library?, words?, terms?)` | check-overlap's result for a draft file, as JSON |
| `reveal_characters(text, typographic?)` | The characters found, with UTF-16 spans |
| `list_notes`, `ai_results(note)`, `similarity_results(note)`, `unusual_characters(note, typographic?)` | What the plugin keeps for notes, with lines |

When the graphics card has no room for SlopTotal, a check answers `gpu_shortage` with what is needed and free: ask the
user, then call again with `gpu: "free"` (their other GPU services let go) or `"cpu"`.

## check-overlap

The close-copying check of a draft file from the command line, for example on a chapter before handing it in.

```text
check-overlap DRAFT [--library] [--words N (6)] [--common N (5)] [--content N (4)] [--term PHRASE]...
              [--vault DIR] [--library-folder FOLDER] [--exclude FOLDER]... [--json]
```

- By default the draft is compared with the notes of the works it cites (`[@key]`, `@key`): the library notes in the
  `<key>/` folder of the library folder, or whose `citekey` property is the key. `--library` compares it with every
  library note. Cited keys with no library note are listed.
- Quotations, citations, code, maths, comments, callouts, tables, links and LaTeX commands are left out. Runs of
  `--words` words or more are reported, near-verbatim stretches with the words that differ; runs in `--common` notes or
  more are the field's fixed phrasing (listed apart); `--term` (repeatable) and Authenticity's *Fixed terms* never make
  a run.
- The vault is `--vault`, else the nearest folder above the draft holding `.obsidian`. The library folder is
  `--library-folder`, else Authenticity's *Library folder* setting; one of the two is required. Notes in `Drafts/`,
  `LaTeX/`, Authenticity's *Never compare with* folders, `--exclude` folders and the draft itself are never sources.
- It reads and keeps the same library index as the plugin (only notes that changed are read again).
- Exit status: 0 no run, 1 runs found, 2 the check could not run. `--json` prints the result as JSON.

## The helper's interface

`authenticity-helper --sloptotal <folder> [--port 8791] [--sloptotal-port 8000] [--device auto|gpu|cpu]
[--idle-minutes 10] [--exit-with-stdin]` serves on `127.0.0.1` only:

| Request | What it does |
|---|---|
| `GET /health` | `service: "authenticity-helper"`, `protocol: 1`, SlopTotal's `state`, `device`, `why`, and whether the helper started it |
| `POST /sloptotal/start {gpu}` (or `{gpu_shortage}`) | Starts SlopTotal |
| `POST /sloptotal/stop` | Stops a SlopTotal it started |
| `/sloptotal/...` | SlopTotal's scoring API, with the text alone |
| `GET /weights` | SlopTotal's detector weights |
| `POST /gpu/release` | The release path of its GPU claim: stops a SlopTotal it started on the card |

The GPU claim (`<local data>/obsidian-kit/gpu/authenticity.json`) lets other local GPU services ask for the card; its
name is "Authenticity". How the helper is protected: [Privacy and security](privacy-and-security.md#the-helper).
