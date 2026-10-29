# History and logs

[← README](../README.md)

Authenticity keeps a history of AI checks, so a past run can be compared with the current one, and three logs for
finding a fault. All of them are on this computer, outside the vault, in Authenticity's local data folder:
`%LOCALAPPDATA%\authenticity` on Windows (`$XDG_DATA_HOME/authenticity` or `~/.local/share/authenticity` elsewhere).

## The history of AI checks

Each AI check is recorded in `history/<vault id>.jsonl`: when it ran, the note's path, the detector (SlopTotal or
GPTZero), the device, and for each paragraph (or group of short paragraphs, or sentence) a hash of its text, its first
eight words, its score and every detector's result and weight.

- **The text itself** is kept only with *Keep the checked text in the history* on (off by default).
- **How long:** runs older than *Keep the history for* (30 days by default) are dropped, and the file is kept under
  5 MB, the oldest runs going first.
- **Where to see it:** *History* in the AI Detector pane lists the note's past checks; pick one to see its scores beside
  the current ones ([Using Authenticity](usage.md#the-ai-detector-pane)).

## The logs

| Log | Where | What it holds |
|---|---|---|
| Authenticity's log | In memory only, until Obsidian closes | What Authenticity told you (its notices), the checks it started, and checks that failed or were cancelled |
| The debug log | `logs/authenticity-debug.log`, only while *Debug log* is on (off by default) | Each request's size and hash, the grouping of short paragraphs, how many requests ran at once, the device, the answers' status, times and detector results, and errors. Moved to `.1` past 5 MB (one old file kept) |
| SlopTotal's log | `logs/sloptotal.log`, begun again each time Authenticity starts SlopTotal | SlopTotal's own output: its start-up, the models it loads, and its errors |

No log ever holds a key. A debug line holds the checked text only while *Keep the checked text in the history* is on.

## The log pane

Settings → Authenticity → **Logs** shows the logs live, as they grow:

- **Which log:** Authenticity, the debug log (listed while *Debug log* is on; reopen the settings after turning it on),
  or SlopTotal.
- **Follow new lines:** keeps the newest line in view.
- **Filter:** shows only the lines containing the text you type (case does not matter).
- **Wrap lines:** wraps long lines, or, off, scrolls sideways. The choice is kept for this device.
- **Copy:** copies the lines shown to the clipboard. Check them for paths before you share them.

The pane keeps the last 2,000 lines; drag its lower edge to make it taller. It reads the log files only (it never
changes them), from their end, and starts again from the end when a file is rotated or truncated. Before a line is
shown, the GPTZero key is taken out of it if it ever appeared there, and a debug line's checked text is hidden unless
the history keeps text. The log files are read on desktop Obsidian only.

## SlopTotal's errors

SlopTotal's log names the detector that failed and the error. What the common ones mean, and what to do:
[Troubleshooting](troubleshooting.md).
