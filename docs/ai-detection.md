# AI detection

[← README](../README.md)

How the AI-writing check scores your text, what the numbers and reasons mean, and how far to trust them. Installing
SlopTotal is on [Installation](installation.md#3-sloptotal); the pane is described on
[Using Authenticity](usage.md#the-ai-detector-pane).

## The scale

Every score is on one scale, **AI-likeness 0 to 100**: 0 reads as written by a person, 100 as AI. Up to 35 reads as
human (green), above 65 as AI (red), and between is mixed (orange). A score is a resemblance to the output of
language models, never proof of who wrote a text.

## SlopTotal

[SlopTotal](https://github.com/pablocaeg/sloptotal) runs 23 detectors (neural classifiers and statistical tests) and
combines them with a logistic model whose weights its author fitted on their own benchmark, SlopBench: one set of
weights for English and one for other languages (`app/calibration.json` in SlopTotal's folder, which Authenticity
reads but never changes).

**The quick score.** The tint and the *AI-likeness* column come from SlopTotal's quick check: each paragraph is sent on
its own (two at a time on the processor, fewer when the computer is busy; on the graphics card two, and one when its
memory runs low).

**Short paragraphs.** A paragraph of 80 words or more is scored on its own. A shorter one is scored together with the
paragraphs after it in its section (headings start sections) until the group reaches 80 words; at a section's end,
with those before it as well; only when its section has nothing more, across the next heading. Each short paragraph
shows the group's score, marked *scored together with the next paragraph (too short alone)*, and clicking it selects
the whole group. Editing any paragraph of a group drops the group's score until the next check. A selection or note too
short in all is scored as one and said to be less reliable.

**When a detector does not run.** SlopTotal's own overall score counts a detector that failed on the text as 0 (as
human as possible), which would pull the score down, so Authenticity does not show that number. It leaves the failed
detectors out: the score is the weighted average of the detectors that ran, by SlopTotal's own weights renormalised
over them. The pane says which detectors did not run, with SlopTotal's reason. If those that ran carry less than half
of SlopTotal's weight for the text's language, no score is shown, only the message. If Authenticity cannot first work
SlopTotal's own number out again from the same inputs (another calibration file, or text in a script written without
spaces between words), it shows no score rather than a guess.

## Reasons and hints

**Reasons.** *Load reasons* runs SlopTotal's full analysis of a paragraph. Each detector is then its own block: its
name in plain words, its result 0 to 100 with a bar, the share of the score it counts for, and one sentence saying what
it measures and what it found, in SlopTotal's own words. The trained classifiers give a probability only, and the pane
says so rather than inventing a reason. Detectors are ordered by how far each moved the score; those of weight 0 for
the text's language are folded away as *not counted for this language*.

A weight reflects both how accurate a detector was on SlopBench and how much it overlaps with the others; it is not a
measure of how far to trust that detector alone. Single detectors disagree strongly on short text: on a 248-character
text written by a person, SlopTotal's overall score was 24 (human) while three of its classifiers scored it 87 to 93.
Read the paragraph's overall score in its context, never one detector.

**Hints** are model-free and found in the text itself: stock phrases, signposting openers, vague wording, stacked
hedges, em dashes and even sentence lengths. They point at features common in model output; none is evidence on its
own.

**Sentences.** *Score sentences* scores each sentence of a paragraph with its neighbours, so you can see which ones
carry the paragraph's score.

## How reliable it is

SlopTotal's own published figures, measured on SlopBench (1,626 human texts written before ChatGPT and 1,626 AI texts
on the same topics from 14 models, across 15 kinds of writing):

- **It is tuned to rarely call human text AI:** 1.7% of human texts were called "Likely AI" (SlopTotal's score of 55
  or more), and 67% of AI texts were. So it **misses about a third of AI text** (SlopTotal's `README.md`, the table
  under its SlopBench results).
- **It misses more on academic writing:** after calibration, 60% of the AI-written arXiv abstracts were flagged, down
  from 79% before (`tests/eval/slopbench/FINDINGS.md`, section "The calibration").
- **Its benchmark includes no Anthropic (Claude) model** beyond three test samples: "frontier models are thinly
  represented, and no Anthropic model beyond three test samples" (`tests/eval/slopbench/README.md`).
- It is more often wrong on essays by non-native English writers, and under about 80 words a score is a weak signal
  (SlopTotal's `README.md`, "What does not work").

**What follows:** a high score is meaningful, since few human texts score that high; a low ("human") score is weak
evidence and does not show that a text was written by a person. These are SlopTotal's figures for its own score
(October 2026); Authenticity's bands (35 and 65) are its own and have not been measured separately. Follow your
institution's rules on AI use and disclosure.

## GPTZero (optional)

[GPTZero](https://gptzero.me) is an online AI detector. With its API key linked
([Installation](installation.md#5-gptzero-optional)), each AI check in Obsidian first asks: **GPTZero** (sends the
checked text to GPTZero's servers), **SlopTotal (offline)** or **Cancel**, saying about how many words GPTZero would use
and how many your plan has left. Tick *Remember for this session* to stop the question until Obsidian restarts; the
choice is never kept longer. Without a key there is no question and nothing is sent. Cancel sends nothing.

**What is shown.** GPTZero's probability that the text was written by AI, on the same scale (AI-likeness = 100 times
that probability) and marked *GPTZero* on every row; its class (human, mixed, AI) and its message; and, under
*Sentences*, each sentence's own probability, with the sentences GPTZero highlights as AI. GPTZero gives its reasons
(AI Patterns) only to keys it has allow-listed, so the pane shows its message instead. Results are kept apart from
SlopTotal's, and the history records which detector gave each score.

**When a request fails** (a bad or expired key, a plan out of words, the hourly rate limit), Authenticity says so
plainly, keeps nothing from GPTZero, and offers to check with SlopTotal instead.

**What is sent, and what GPTZero says it does with it:** [Privacy and security](privacy-and-security.md#gptzero). The
[assistant tools](assistant-tools.md) never use GPTZero: an assistant cannot answer the question asked before each
check.

## What SlopTotal does that Authenticity does not use

Authenticity uses only SlopTotal's local scoring API (`/health`, `/api/quick-score`, `/api/analyze`, its queue tickets
and its list of detectors), always with the text alone. These parts of SlopTotal are **not used**, and Authenticity
never starts them or sends what they need: URL scanning (the `url` field is never sent: the helper rebuilds every
request as `{"text": ...}`), its web page, its Chrome extension, its Site check, its Docker image, its feedback
endpoint (which records who wrote a text), and any model download after installation.
