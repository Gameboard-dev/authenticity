# Settings

[← README](../README.md)

Settings → Authenticity. A column of contents beside the settings (above them in a narrow window) links to each
section. Most settings are kept in the plugin's `data.json` in the vault, so devices sharing the vault share them;
those marked *this computer* are kept in this computer's Obsidian only, so a program's path never travels with your
notes.

## Writing checks

| Setting | What it does |
|---|---|
| SlopTotal folder (this computer) | The folder SlopTotal is installed in ([Installation](installation.md#3-sloptotal)). Environment variables and a leading `~` are expanded; *Status* shows the folder used |
| Device (this computer) | *Automatic* (default): the graphics card when SlopTotal's Python has PyTorch for CUDA and the card has room for its models (about 8.3 GB with a margin), asking your other local GPU services to free theirs first if you agree; otherwise the processor. *Graphics card* asks whenever the card is short; *Processor* always uses the processor |
| SlopTotal port (this computer) | Where SlopTotal answers on `127.0.0.1` (8000). A SlopTotal you started yourself on this port is used as it is and never stopped |
| Stop SlopTotal after this many idle minutes (this computer) | SlopTotal holds about 9 GB of memory while it runs; it is stopped this long after the last check (10) |
| Helper program (this computer) | The full path of `authenticity-helper`; blank: the one in the plugin's folder |
| Status | Whether the helper and SlopTotal run, and on which device; *Check* asks again |
| Ask before checking more than this many words | An AI check of more words asks first (5,000; 0: always ask) |

## Plagiarism

What these find: [Similarity](similarity.md).

| Setting | What it does |
|---|---|
| Library folder | The vault folder of your source notes. Required for the plagiarism check |
| Never compare with | More folders of your own writing, comma-separated. `Drafts/`, `LaTeX/` and the note being checked never count |
| Leave out quoted text | Text in quotation marks is not compared (on) |
| Near-verbatim | Also find stretches with a few words changed, inserted or left out (on) |
| Fewest content words | A run with fewer words that are not function words or fixed terms is ordinary language, not copied wording (4) |
| Fixed phrasing from | A run found in this many library notes or more is the field's fixed phrasing, listed apart (5; 0: never) |
| Fixed terms | Technical terms that never make a run, one per line or comma-separated |
| Close paraphrase | Find close paraphrases (on) |
| Copied wording: shortest run | The shortest run shared word for word (6). Also at the top of the Plagiarism pane |
| Close paraphrase from | The paraphrase closeness (50%, 30% to 90%). Also in the Plagiarism pane, with *Reset to 50%*, and in the ribbon menu |
| Embedding search (optional) | Use the Quarry plugin's embedding model for the paraphrase check, if it is installed (off). *Check* tests it ([Similarity](similarity.md#the-optional-embedding-search)) |
| Library index | How many notes the index holds; *Rebuild* makes it again from every library note |

## GPTZero (optional)

| Setting | What it does |
|---|---|
| GPTZero API key | Links a key kept in Obsidian's keychain (Settings → Keychain), never in Authenticity's files. Without a key nothing is sent ([AI detection](ai-detection.md#gptzero-optional)) |
| Check the key | Asks GPTZero for the plan and the words left this billing cycle; uses no words |

## History

| Setting | What it does |
|---|---|
| Keep the history for (days) | How long each AI check is kept in the history (30) ([History and logs](history-and-logs.md)) |
| Keep the checked text in the history | Off (default): the history keeps each paragraph's hash and first eight words only. On: also the text |

## Logs

| Setting | What it does |
|---|---|
| Debug log | Writes a detailed log of each check, for finding a fault (off) |
| Log | The live log pane ([History and logs](history-and-logs.md#the-log-pane)) |

## Colours

| Setting | What it does |
|---|---|
| AI colour | The tint of paragraphs that read as AI, stronger the higher the score |
| Human colour | The tint of paragraphs that read as written by a person, stronger the lower the score. Scores in between are orange |
| Strength | The opacity of the strongest tint (60%). Also in the ribbon menu |
| Show marks in notes | Off hides every tint, underline and character mark, keeping the results. The command *Show or hide Authenticity's marks* does the same |
| Show AI marks | The paragraph tints. Also in the ribbon menu |
| Show similarity marks | The plagiarism check's underlines. Also in the ribbon menu |

## Characters

What these find: [Characters](characters.md).

| Setting | What it does |
|---|---|
| Mark unusual characters as you write | Marks invisible and look-alike characters in every note in the folders below as you type (on). Off: only where you use Reveal AI characters |
| Mark typographic characters | Also mark curly quotes, the ellipsis character, bullets and arrows (on) |
| Replace typographic characters too | *Replace unusual characters* also replaces typographic characters (off) |
| Only in these folders | Comma-separated folders whose notes are marked as you write; blank for every note |
