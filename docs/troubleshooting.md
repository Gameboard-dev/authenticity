# Troubleshooting

[← README](../README.md)

Most problems are explained in Authenticity's own notices and in the log pane (Settings → Authenticity → **Logs**,
[History and logs](history-and-logs.md#the-log-pane)). Start there.

## Common problems

| Problem | What to do |
|---|---|
| "no helper program at …" | The helper is not in the plugin's folder. Download it from the [native release](https://github.com/Gameboard-dev/authenticity/releases/tag/native-0.3.0) ([Installation](installation.md#2-the-helper)), or set *Helper program* |
| "the helper speaks protocol …: update the helper program" | The helper is from another version: download the one that matches the plugin |
| The AI check says SlopTotal's folder is not set or not found | Set *SlopTotal folder*; *Status* under it shows the folder it expands to ([Installation](installation.md#step-5-tell-authenticity-where-it-is)) |
| SlopTotal does not start, or a detector is missing | Read SlopTotal's log in the log pane. Check that `scripts\smoke_test.py` passes in SlopTotal's folder ([Installation](installation.md#step-4-first-start-which-downloads-the-models-about-9-gb-once)) |
| The port is in use | Another program uses port 8000 or 8791. Change *SlopTotal port*; a SlopTotal you started yourself on that port is used as it is |
| Asked what to do because the graphics card is full | Another program holds the card's memory. Choose the processor (slower), or let your other local GPU services free their memory |
| The check is slow | On the processor, SlopTotal takes a few seconds a paragraph, and the first check of a session loads the models (tens of seconds). A card with more than 9 GB and PyTorch's CUDA build is faster |
| A paragraph shows no score, only a message | The detectors that ran carry too little of SlopTotal's weight for the text's language, or the text is in a script Authenticity cannot score ([AI detection](ai-detection.md#sloptotal)) |
| *Engine binoculars failed: math range error* (or *cross_perplexity*) in SlopTotal's log | The model returned a value too large for the calculation: that run's copy of the model was bad, not your text. Stop SlopTotal (*Stop SlopTotal now*) so the next check starts it with fresh models; if it happens again, turn on *Debug log* and report it to SlopTotal |
| The plagiarism check asks for a library folder | Set *Library folder* (Settings → Plagiarism) |
| A source you expected is not found | It is outside the library folder, in a folder never compared with, or its note changed after the index was made; *Rebuild* the library index |
| *Embedding search* says the service does not answer | The Quarry plugin's service is not running. The check uses Authenticity's own search instead; nothing else is needed |
| *Embedding search* says the service asks for its token, or refused it | The Quarry plugin is older than its service's token, or has not reached its service yet: update Quarry, or start its service. In a browser (Ignis), paste the token again in Quarry's settings (*Service token*) |
| GPTZero: "the GPTZero key is missing or has expired" (401) or "does not recognise the key" (403) | Link a valid key and press *Check the key* |
| GPTZero: "the word limit or hourly rate limit is reached" (429) | Wait, or check with SlopTotal (Authenticity offers it) |
| Marks are not shown | Check *Show marks in notes*, *Show AI marks* and *Show similarity marks*; an edited paragraph loses its colour until it is checked again |

## Known limits

- AI-detection scores are a resemblance to model output, never proof of authorship. SlopTotal misses about a third of
  AI text and is less reliable on short text and non-native English ([AI detection](ai-detection.md#how-reliable-it-is)).
- The plagiarism check knows only your library ([Similarity](similarity.md#what-it-covers-and-what-it-does-not)).
- The paraphrase search and the hints are for English text.
- Desktop only: the plugin starts the helper as a process. The helper is built for Windows (x86-64); other systems need
  a build from source.
