# Similarity

[← README](../README.md)

The plagiarism check compares the text you check with the notes in your library folder, on this computer, and finds
three things: wording shared with a source, the field's fixed phrasing, and close paraphrases. The pane that lists them
is described on [Using Authenticity](usage.md#the-plagiarism-pane).

## The library

Set **Library folder** (Settings → Authenticity → Plagiarism) to the vault folder of your source notes; until it is
set, the check asks for it. **Your own writing is never a source:** notes in `Drafts/` and `LaTeX/`, the folders you
add under *Never compare with*, and the note being checked are never compared with.

### The library index

The first check reads every library note and builds an index of their word sequences and content words. It is kept
on this computer, outside the vault, and brought up to date note by note as the library changes: only the notes that
changed are read again. The command *Rebuild the library index from every library note* makes it again from scratch.
The index holds hashes and word stems, not the notes' text ([Disclosures](disclosures.md#what-is-kept-and-where)).

## Copied wording

Runs of six or more words that appear in a library note, ignoring case, punctuation, hyphenation, accents and
ligatures. With *Near-verbatim* on, stretches with a few words changed, inserted or left out are found too.

Left out of the comparison: text inside quotation marks (a quotation with its citation is meant to match its source),
code, maths, comments, tables, links and citations.

## Fixed phrasing

A run found in five library notes or more (*Fixed phrasing from*) is the field's fixed phrasing: listed apart in the
pane, not marked and not counted. So are runs made mostly of function words (the, of, and), runs with fewer content
words than *Fewest content words*, and the *Fixed terms* you list.

## Close paraphrase

A close paraphrase (patchwriting) is a sentence that keeps a library sentence's own words in its order with a few
changed. For each sentence of yours, the library passages that share the most of its rarer words are found in the
index and compared with it sentence by sentence; no model is used and nothing leaves the computer. A sentence is
flagged when both hold:

- it keeps at least the **paraphrase closeness** of its content words (the words other than the, of, and …) in the
  order of a library sentence (*Close paraphrase from*, 50% by default, never below 30%);
- the words it shares are that sentence's particular ones, not only common ground: each word is weighed by how rare
  it is in the library, and the shared words must carry 45% or more of the source sentence's weight, and enough of it
  in all.

Saying what a source says in your own structure is not flagged. On a library of about 800 notes the search for a
chapter takes under half a second.

## The thresholds

Two thresholds are set at the top of the Plagiarism pane and in Settings → Plagiarism; changing either place changes
both, and the ribbon menu offers the closeness too.

| Threshold | Default | What it is |
|---|---|---|
| Close paraphrase from | 50% (30% to 90%) | The paraphrase closeness above. Lower flags more. *Reset to 50%* in the pane restores the default |
| Verbatim from | 6 words | The shortest run of words shared word for word |

A change applies at once to the table, the count and the underlines, without searching again: a check keeps every run
from the shortest run it used, and each sentence's paraphrase candidates from 10 points below the closeness. Only a
value below what the last check fetched (a shorter run, or a closeness more than 10 points lower) needs a new check;
the pane says so and offers *Check the note again*.

## The optional embedding search

If the Quarry plugin is installed, Authenticity can optionally use its embedding model for the paraphrase check
(Settings → Plagiarism → *Embedding search*, off by default; *Check* tests it). The closeness means the same either
way; with the embedding search, the two sentences must also say the same by the model (a cosine of 0.75 or more). It
asks only a service on this computer. When that service does not answer, the check uses Authenticity's own search and
says so.

## What it covers, and what it does not

Only your library. It cannot see web pages, other people's work or anything you have not added; services such as
Turnitin search those. Wording copied without quotation marks is plagiarism even when it is cited: quote it exactly,
with the page, or say it in your own words and structure.

The same check runs from the command line on a draft file: [check-overlap](assistant-tools.md#check-overlap).
