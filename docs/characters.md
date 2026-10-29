# Characters

[← README](../README.md)

Reveal AI characters marks characters that are invisible or look like ordinary ones: text pasted from a chat window,
a web page or a converted document often carries them. The rules are fixed; no model is used and nothing is sent.

## What it finds

| Kind | Examples |
|---|---|
| Invisible characters | Zero-width spaces and joiners, byte-order marks, soft hyphens, direction marks, variation selectors |
| Unusual spaces | No-break, thin, hair, ideographic and other non-standard spaces |
| Dashes | Unicode hyphens and dashes other than the keyboard's |
| Look-alike punctuation | Fullwidth, small, ideographic and Arabic commas, full stops, colons and semicolons; the Greek question mark; a modifier letter used as an apostrophe |
| Look-alike letters | A Cyrillic or Greek letter inside a Latin word; styled mathematical letters; fullwidth letters in Latin text |
| Typographic characters (optional) | Curly quotes and apostrophes, the ellipsis character, primes, the minus sign, bullets and arrows |

Typographic characters are marked while *Mark typographic characters* is on (the default): word processors produce them too, so
they are a weak sign on their own. How each kind is marked in a note: [What the marks mean](usage.md#what-the-marks-mean).

With *Mark unusual characters as you write* on, characters are marked in every note in *Only in these folders* (or every
note) as you type; otherwise only where you run Reveal AI characters ([Settings](settings.md#characters)).

## Replacing characters

**Replace unusual characters…** (right-click a selection) replaces them in the selection only, after a dialog that says
how many characters of which kinds will change; one undo puts them back.

- Invisible characters are removed. Joiners and variation selectors are removed only between Latin letters, since
  emoji and other scripts need them.
- Unusual spaces become spaces.
- Look-alike punctuation becomes the keyboard's, where the text around is Latin.
- Look-alike letters become Latin ones.
- Typographic characters (curly quotes, en and em dashes, the ellipsis character, primes, the minus sign, bullets and
  arrows) are replaced only with *Replace typographic characters too* on.
- Dashes: an em dash becomes `--` where it stands, with the spaces around it as they were ("a — b" becomes "a -- b",
  "a—b" becomes "a--b"); so do the horizontal bar, the two- and three-em dashes and the small and vertical em dashes.
  An en dash, a figure dash and the minus sign become `-`.
- Accented letters, ×, ° and other symbols are never replaced.

## Stepping through them

In the AI Detector pane, **AI characters** lists the kinds found, with how many. Click an entry to go through its
occurrences and those below it one at a time: each is selected, and a small prompt says what it is and what it would
become.

- **OK (Enter)** replaces it (one undo), **Skip (Esc)** skips it, **Stop** ends.
- Typographic characters are always offered their plain equivalent here (curly quotes become straight, the ellipsis
  three full stops, an em dash `--`, an en dash `-`), since you decide one at a time; *Replace typographic characters too* governs only
  the bulk command.
- Where there is no automatic replacement (a Cyrillic or Greek letter with no Latin twin, a styled Greek letter, the
  ʻokina), a box takes the one character to put there.
