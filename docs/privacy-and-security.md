# Privacy and security

[← README](../README.md)

What leaves the plugin, what GPTZero says it does with text, and how the plugin and the helper are protected. The
policy disclosures (network hosts, files outside the vault, payment, telemetry) are on
[Disclosures and compliance](disclosures.md).

## Your notes

- The AI-writing check sends the paragraphs you check to SlopTotal through the helper, both on `127.0.0.1`, with the
  text alone. SlopTotal keeps a copy of each text it scores in its own database on your computer for one day, the
  shortest it allows, then deletes it.
- The plagiarism check reads your library notes and compares them on this computer. With the optional embedding
  search on, the sentences not already matched word for word go to that plugin's service on `127.0.0.1`.
- Results are kept in the plugin's folder in the vault; the history, the library index and the logs in Authenticity's
  local data folder ([History and logs](history-and-logs.md), [Disclosures](disclosures.md#what-is-kept-and-where)).
  Nothing is collected about you or your use.

## GPTZero

GPTZero is used only with your own key and only for a check where you chose it when asked
([AI detection](ai-detection.md#gptzero-optional)).

**What is sent.** For each paragraph (or group of short paragraphs), one HTTPS request to
`https://api.gptzero.me/v2/predict/text` with the text alone (`{"document": ...}`) and your key in the `x-api-key`
header: no note name, vault, path or other identifier. Before the question, and when you press *Check the key*, one
request to `https://api.gptzero.me/v3/usage-stats` with the key alone (no text, no words used) fetches the words left
on your plan.

**AI Patterns**, only when you press its button in a GPTZero row: one request to
`https://api.gptzero.me/v3/ai/patterns/stream` with that paragraph's text alone and the key.

**The key** is kept in Obsidian's keychain. Authenticity reads it only to send a request, and never writes it to its
files, history, logs or error messages; if GPTZero's answer ever quotes it, it is taken out before anything is shown.

**What GPTZero says it does with the text.** GPTZero's own pages, read on 29 October 2026, say:

- Its developer page (<https://gptzero.me/developers>), on the API: "No. We do not store or collect the documents
  passed into any calls to our API."
- Its help article "Are you storing data from API calls?": "By default, we do not store the documents passed into
  calls to our AI detection API." The article adds that inputs and results are saved when *Persist scan results* is
  turned on for the key on GPTZero's API key page, and that inputs on GPTZero's website and dashboard are stored and
  used in aggregate to improve the service.
- Its help article "Do I retain ownership of the work after passing it through GPTZero?": "While we retain a copy of
  submitted text, we do not reproduce the text or disclose it to third parties."
- Its [privacy and security page](https://gptzero.me/privacy-and-security) does not mention the API or submitted text
  in particular. It says sensitive information is "encrypted" and "only retained for as long as needed", and that
  GPTZero is SOC 2 Type II compliant.

**Reading them together.** The two statements about the API say that text sent through the API is not stored, unless
*Persist scan results* is turned on for the key; the statement that GPTZero keeps a copy is not specific to the API
and matches what the API article says about the website and dashboard. None of the pages says whether submitted text
is used to train GPTZero's models. Authenticity cannot check what GPTZero does: if this matters to you, keep *Persist
scan results* off for your key, read GPTZero's [privacy and security page](https://gptzero.me/privacy-and-security) and
[terms of use](https://gptzero.me/terms-of-use.html), or use SlopTotal, which keeps everything on your computer.

## The helper

- It listens on `127.0.0.1` only, so other computers cannot reach it, and refuses requests addressed to another host or
  sent from a web page, so a page open in your browser cannot use it.
- It sends SlopTotal the text alone: every request is rebuilt as `{"text": ...}`, so SlopTotal's URL scanning is never
  reached.
- It starts SlopTotal with `--host 127.0.0.1`, with model downloads switched off (`HF_HUB_OFFLINE=1`,
  `TRANSFORMERS_OFFLINE=1`, `HF_HUB_DISABLE_TELEMETRY=1`) and with the shortest retention SlopTotal allows.

## The plugin

- It starts one program, the helper, from a path set on this computer (never from the vault's settings), with an
  argument list, never through a shell.
- Text is set with `textContent` and the DOM API; no HTML is built from strings.
- The clipboard is written only when you press Copy in the log pane or copy with marks, and never read.
- It reads only its own files outside the vault: SlopTotal's folder (to find its Python and calibration), and its own
  local data folder.
