# Using Authenticity

[← README](../README.md)

Where to find the checks, what the two panes show, and what the marks in a note mean. What each check finds is
explained on its own page: [AI detection](ai-detection.md), [Similarity](similarity.md), [Characters](characters.md).

## Running a check

- **On a selection:** select text, right-click, and choose *Check for plagiarism*, *Check for AI writing*, *Reveal AI
  characters* or *Replace unusual characters…*.
- **On the whole note:** the commands *Check this note for AI writing*, *Check this note for plagiarism* and *Reveal AI
  characters in this note*. The selection has its own versions of each.

A check always checks again, even text that has not changed since its last check. An AI check of more words than
*Ask before checking more than this many words* (5,000 by default) asks first. With a GPTZero key linked, an AI check
first asks which detector to use ([GPTZero](ai-detection.md#gptzero-optional)).

**Cancel.** While a check runs, both panes and the ribbon menu offer **Cancel**. It stops the check at once, sends
nothing more to the detector, reads nothing more from the library, and keeps nothing from the check. SlopTotal itself
keeps running until it has been idle for a while; *Stop SlopTotal now* stops it at once.

## The ribbon icon and the status bar

The ribbon icon (the shield) and the status bar item open the same menu:

- the two panes;
- *Show AI marks* and *Show similarity marks*, two separate switches;
- *Start the server* or *Stop the server* (the helper and SlopTotal);
- *Strength of the AI marks*;
- *Close paraphrase from*, the same setting as the Plagiarism pane's ([Similarity](similarity.md#the-thresholds)).

The status bar shows whether the server runs, as a dot before *Authenticity*: grey when stopped, orange while
starting, green when running. Its tooltip says more, including the device SlopTotal runs on.

## Commands

| Command | What it does |
|---|---|
| Check this note for AI writing / Check the selection for AI writing | Scores each paragraph ([AI detection](ai-detection.md)) |
| Check this note for plagiarism / Check the selection for plagiarism | Compares with the library ([Similarity](similarity.md)) |
| Reveal AI characters in this note / in the selection | Marks hidden and look-alike characters ([Characters](characters.md)) |
| Replace unusual characters in the selection | Replaces them, after a dialog that says what will change ([Characters](characters.md#replacing-characters)) |
| Copy the selection with Authenticity's marks | Copies the selection's Markdown with each mark as a `~={#hex, Name}text=~` marker |
| Clear this note's results | Removes the note's scores and matches |
| Show or hide Authenticity's marks | Hides every mark, keeping the results |
| Open the AI Detector pane / Open the Plagiarism pane | Opens a pane |
| Stop SlopTotal now | Stops SlopTotal if Authenticity started it |
| Rebuild the library index from every library note | Makes the index again ([Similarity](similarity.md#the-library-index)) |

No command has a default hotkey.

## Going from a mark to its entry

Right-click checked text (a selection, or just the cursor in it): *Show in AI check* appears when the text was
AI-checked and has not changed since, *Show in similarity check* when it was checked for plagiarism, something there
was flagged, and it has not changed. Each opens its pane and scrolls to that entry, without checking anything.

There is no pop-up on hover: what a mark means is in the panes. An edited paragraph loses its colour until it is
checked again.

## The AI Detector pane

The pane follows the note last active.

- **AI characters**, at the top: the characters Reveal AI characters found, by kind, with how many. Click an entry to
  step through its occurrences ([Characters](characters.md#stepping-through-them)).
- **The table** of checked paragraphs, on one scale, AI-likeness 0 to 100, with its legend:

  | Column | What it shows |
  |---|---|
  | My paragraph | The paragraph, with the words of Authenticity's writing patterns highlighted |
  | AI-likeness | The score the tint shows, marked with the detector that gave it |
  | Verdict | The band, with at most one reliability note (scored with its neighbours, under 80 words, or the detector's low confidence) |
  | Why | The reasons of the detector that scored the row, named at the top of the cell (SlopTotal or GPTZero), each as a collapsed block; and *Writing patterns*, when Authenticity's own rules found any ([Reasons](ai-detection.md#reasons-and-writing-patterns)) |

  Every block in *Why* starts collapsed: its summary line gives the name and the result, and a click (or Enter or
  Space) opens it. *Sentences* under a paragraph lists its sentences, each with its own score. Sort by score or by
  position; click a row to go to the text.
- **History**: this note's past checks; pick one to see its scores beside the current ones
  ([History and logs](history-and-logs.md)).

Nothing in the pane suggests a rewrite.

## The Plagiarism pane

The pane follows the note last active.

- At the top, the two thresholds, *Close paraphrase from* and *Verbatim from*, and *Reset to 50%* when the closeness
  is not the default ([Similarity](similarity.md#the-thresholds)).
- A count line, such as "2 close paraphrases and 4 verbatim runs in 126 sentences".
- A table: *My sentence* | *Similar source sentence* | *Source* (citation key and title, page, and a link that opens
  the library note at the line, in a new tab) | *Kind* (verbatim, near-verbatim, close paraphrase) | *Closeness*. The
  words a sentence shares with the source are marked in both cells. Sort by closeness or by position; click a row to go
  to the sentence.
- The field's fixed phrasing, listed apart and not counted.

## What the marks mean

| Mark | Meaning |
|---|---|
| Green tint | The paragraph reads as written by a person (35 or below), stronger the lower |
| Orange tint | Mixed (between 35 and 65) |
| Red tint | The paragraph reads as AI (above 65), stronger the higher |
| Solid blue underline | The same or nearly the same wording as a library note |
| Dashed blue underline | A close paraphrase of a library sentence |
| Orange *INV* label | An invisible character |
| Orange box | An unusual space or look-alike punctuation |
| Red box | A dash |
| Wavy red line | A letter from another script inside a word |
| Dotted purple line | A typographic character (optional: word processors make these too) |

The tints come in five strengths each way; the colours and the strongest tint's opacity are in
[Settings](settings.md#colours). The field's fixed phrasing is not marked, and neither is restating a source in your
own structure.
