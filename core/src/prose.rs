//! The prose of a note, as a detector should read it: Markdown and Obsidian markup removed (front matter, code and
//! TikZ blocks, display and inline maths, `%% %%` comments, tables, block ids, citations `[@key]`, footnotes,
//! evidence links, emphasis markers), links reduced to the text they show, one line per source paragraph. Every
//! character kept remembers where it came from in the note, as a UTF-16 range, which is what CodeMirror's positions
//! count, so a sentence the detector returns can be tinted where it sits.

/// The prose, and for each of its characters the [from, to) UTF-16 range in the note it came from (an inserted line
/// break has an empty range at the start of its line).
#[derive(Debug, Default, Clone)]
pub struct Prose {
    pub chars: Vec<char>,
    pub src: Vec<(u32, u32)>,
}

impl Prose {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn words(&self) -> usize {
        self.text().split_whitespace().count()
    }

    /// Only the characters that came from [from, to) of the note (a selection), with the line breaks between them.
    pub fn within(&self, from: u32, to: u32) -> Prose {
        let mut p = Prose::default();
        for (&c, &(a, b)) in self.chars.iter().zip(&self.src) {
            let inside = if c == '\n' { a > from && a < to } else { a >= from && b <= to };
            if inside && !(c == '\n' && (p.chars.is_empty() || p.chars.last() == Some(&'\n'))) {
                p.chars.push(c);
                p.src.push((a, b));
            }
        }
        while p.chars.last() == Some(&'\n') {
            p.chars.pop();
            p.src.pop();
        }
        p
    }

    /// The prose without text in double quotation marks (`"…"` or `“…”`, closed on the same line within 1,000
    /// characters), the marks included: what a plagiarism check sends with quotes left out, since quoting with a
    /// citation is not plagiarism. A space the removal doubles is dropped. An unclosed mark is kept, with what follows.
    pub fn without_quotes(&self) -> Prose {
        const LONGEST: usize = 1000;
        let mut p = Prose::default();
        let n = self.chars.len();
        let mut i = 0;
        while i < n {
            let c = self.chars[i];
            if c == '"' || c == '\u{201C}' {
                let closes = |d: char| d == '"' || (c == '\u{201C}' && d == '\u{201D}');
                let end = (i + 1..n.min(i + 1 + LONGEST)).take_while(|&j| self.chars[j] != '\n').find(|&j| closes(self.chars[j]));
                if let Some(j) = end {
                    i = j + 1;
                    if p.chars.last().is_none_or(|c| *c == ' ' || *c == '\n') {
                        while i < n && self.chars[i] == ' ' {
                            i += 1;
                        }
                    }
                    continue;
                }
            }
            p.chars.push(c);
            p.src.push(self.src[i]);
            i += 1;
        }
        p
    }
}

struct Note {
    cs: Vec<char>,
    o16: Vec<u32>, // o16[i]: the UTF-16 offset of cs[i]; one longer than cs
}

impl Note {
    fn at(&self, i: usize, s: &str) -> bool {
        s.chars().enumerate().all(|(k, c)| self.cs.get(i + k) == Some(&c))
    }

    /// The first index >= `from` and < `to` where `s` starts.
    fn find(&self, s: &str, from: usize, to: usize) -> Option<usize> {
        (from..to).find(|&i| self.at(i, s))
    }
}

struct Out<'a> {
    note: &'a Note,
    p: Prose,
    comment: bool, // inside %% %%, which may span lines
    html: bool,    // inside <!-- -->
}

impl Out<'_> {
    fn push(&mut self, i: usize) {
        self.put(self.note.cs[i], (self.note.o16[i], self.note.o16[i + 1]));
    }

    /// Text that stands for markup in [a, b) of the note (a citation's author and year, a formula's placeholder).
    fn stand_in(&mut self, text: &str, a: usize, b: usize) {
        let range = (self.note.o16[a], self.note.o16[b]);
        text.chars().for_each(|c| self.put(c, range));
    }

    fn put(&mut self, c: char, range: (u32, u32)) {
        if c.is_whitespace() {
            if matches!(self.p.chars.last(), None | Some(' ') | Some('\n')) {
                return;
            }
            self.p.chars.push(' ');
        } else {
            if matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')') && self.p.chars.last() == Some(&' ') {
                self.p.chars.pop(); // the space left where a citation or link was removed
                self.p.src.pop();
            }
            self.p.chars.push(c);
        }
        self.p.src.push(range);
    }

    fn end_line(&mut self, line_start: usize) {
        while self.p.chars.last() == Some(&' ') {
            self.p.chars.pop();
            self.p.src.pop();
        }
        if !matches!(self.p.chars.last(), None | Some('\n')) {
            let at = self.note.o16[line_start];
            self.p.chars.push('\n');
            self.p.src.push((at, at));
        }
    }

    /// The characters of [a, b) of one line, inline markup removed.
    fn inline(&mut self, a: usize, b: usize) {
        let n = self.note;
        let cs = &n.cs;
        let alnum = |i: usize| i < cs.len() && cs[i].is_alphanumeric();
        let mut i = a;
        while i < b {
            if self.comment {
                match n.find("%%", i, b) {
                    Some(j) => (self.comment, i) = (false, j + 2),
                    None => i = b,
                }
                continue;
            }
            if self.html {
                match n.find("-->", i, b) {
                    Some(j) => (self.html, i) = (false, j + 3),
                    None => i = b,
                }
                continue;
            }
            let c = cs[i];
            if n.at(i, "%%") {
                (self.comment, i) = (true, i + 2);
            } else if n.at(i, "<!--") {
                (self.html, i) = (true, i + 4);
            } else if c == '\\' && i + 1 < b {
                self.push(i + 1); // \% → %
                i += 2;
            } else if n.at(i, "$$") {
                i = n.find("$$", i + 2, b).map_or(b, |j| j + 2);
            } else if c == '$' {
                match n.find("$", i + 1, b) {
                    Some(j) if j > i + 1 && !cs[i + 1].is_whitespace() => {
                        let maths: String = cs[i + 1..j].iter().collect();
                        self.stand_in(&math_text(&maths), i, j + 1); // inline maths, as plain text, keeps the sentence whole
                        i = j + 1;
                    }
                    _ => {
                        self.push(i);
                        i += 1;
                    }
                }
            } else if c == '`' {
                match n.find("`", i + 1, b) {
                    Some(j) => {
                        (i + 1..j).for_each(|k| self.push(k));
                        i = j + 1;
                    }
                    None => i += 1,
                }
            } else if n.at(i, "![[") {
                i = n.find("]]", i, b).map_or(b, |j| j + 2);
            } else if n.at(i, "[[") {
                let Some(j) = n.find("]]", i, b) else {
                    i += 2;
                    continue;
                };
                let (s, e) = (i + 2, j);
                match n.find("|", s, e) {
                    Some(bar) if is_label(&cs[bar + 1..e]) => self.stand_in("1", s, e), // a cross-reference prints a number
                    Some(bar) => self.inline(bar + 1, e),
                    None if n.at(s, "EV-") => {
                        // evidence links: the converter leaves them out, with the commas that join a run of them
                        let mut k = j + 2;
                        loop {
                            let mut m = k;
                            while m < b && (cs[m] == ',' || cs[m] == ' ') {
                                m += 1;
                            }
                            match n.at(m, "[[EV-").then(|| n.find("]]", m, b)).flatten() {
                                Some(e) => k = e + 2,
                                None => break,
                            }
                        }
                        i = k;
                        continue;
                    }
                    None => {
                        let t = n.find("#", s, e).map_or(s, |h| h + 1);
                        let t = (t..e).rev().find(|&k| cs[k] == '/').map_or(t, |k| k + 1);
                        self.inline(t, e);
                    }
                }
                i = j + 2;
            } else if n.at(i, "^[") {
                i = close_bracket(cs, i + 1, b).map_or(b, |j| j + 1); // a footnote
            } else if n.at(i, "((") {
                i = n.find("))", i, b).map_or(i + 2, |j| j + 2); // a Tags for Markdown caption tag
            } else if c == '!' && n.at(i + 1, "[") {
                let j = close_bracket(cs, i + 1, b);
                i = match j {
                    Some(j) if n.at(j + 1, "(") => n.find(")", j, b).map_or(b, |k| k + 1),
                    _ => i + 1,
                };
            } else if c == '[' {
                match close_bracket(cs, i, b) {
                    Some(j) if n.at(j + 1, "(") => {
                        self.inline(i + 1, j); // [text](url)
                        i = n.find(")", j, b).map_or(b, |k| k + 1);
                    }
                    Some(j) if (i + 1..j).any(|k| cs[k] == '@' && alnum(k + 1)) => i = j + 1, // [@key, p. 3]
                    _ => i += 1,
                }
            } else if c == '@' && alnum(i + 1) && (i == 0 || !alnum(i - 1)) {
                // @key, narrative, which prints the authors: "Surname (year)" from the key stands in for them (its
                // characters, but not a full stop that ends the sentence)
                let start = i;
                i += 1;
                while i < b && (alnum(i) || matches!(cs[i], '_' | ':' | '-') || (cs[i] == '.' && alnum(i + 1))) {
                    i += 1;
                }
                let key: String = cs[start + 1..i].iter().collect();
                self.stand_in(&narrative(&key), start, i);
            } else if n.at(i, "**") || n.at(i, "__") || n.at(i, "==") || n.at(i, "~~") {
                i += 2;
            } else if c == '*' || (c == '_' && !(i > 0 && alnum(i - 1) && alnum(i + 1))) {
                i += 1;
            } else if c == '<' && i + 1 < b && (cs[i + 1].is_ascii_alphabetic() || cs[i + 1] == '/') {
                match n.find(">", i, b) {
                    Some(j) => i = j + 1, // an HTML tag
                    None => {
                        self.push(i);
                        i += 1;
                    }
                }
            } else if c == '#' && alnum(i + 1) && (i == 0 || cs[i - 1].is_whitespace()) {
                i += 1; // a tag
                while i < b && (alnum(i) || matches!(cs[i], '/' | '-' | '_')) {
                    i += 1;
                }
            } else {
                self.push(i);
                i += 1;
            }
        }
    }
}

/// A LaTeX label used as a link's text (`tab:lit:studies`), which the converter turns into `\ref`.
fn is_label(cs: &[char]) -> bool {
    let t: String = cs.iter().collect();
    ["sec:", "fig:", "tab:", "eq:", "lst:", "app:"].iter().any(|p| t.starts_with(p)) && !t.contains(' ')
}

/// Inline maths as a reader would say it, near enough: TeX's markup removed (`\mathrm{round}(r/S)` → `round(r/S)`,
/// `lpha` → `alpha`), so a detector reads the sentence's real symbols, not a placeholder; a long formula becomes
/// "the formula".
fn math_text(src: &str) -> String {
    let mut out = String::new();
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => {
                let start = i + 1;
                let mut j = start;
                while j < cs.len() && cs[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let name: String = cs[start..j].iter().collect();
                match name.as_str() {
                    "" => j = (j + 1).min(cs.len()), // \, \; \{ …: spacing or an escaped brace
                    "mathrm" | "text" | "mathbf" | "mathit" | "operatorname" | "mathcal" | "boldsymbol" | "left" | "right" | "big" | "Big" => {}
                    "cdot" | "times" => out.push_str(" × "),
                    "le" | "leq" => out.push_str(" ≤ "),
                    "ge" | "geq" => out.push_str(" ≥ "),
                    "approx" => out.push_str(" ≈ "),
                    _ => out.push_str(&name),
                }
                i = j;
            }
            '{' | '}' => i += 1,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    let out: Vec<&str> = out.split_whitespace().collect();
    let out = out.join(" ");
    if out.chars().count() > 40 { "the formula".into() } else { out }
}

/// What a narrative citation prints, near enough, from a Better BibTeX key: `smith2020` → "Smith (2020)".
fn narrative(key: &str) -> String {
    let name: String = key.chars().take_while(|c| c.is_alphabetic()).collect();
    let year: String = key.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    let mut cs = name.chars();
    let name = match cs.next() {
        Some(f) => f.to_uppercase().chain(cs).collect::<String>(),
        None => "The authors".to_string(),
    };
    if year.len() == 4 { format!("{name} ({year})") } else { name }
}

/// The index of the `]` that closes the `[` at `open`, counting nested brackets, before `end`.
fn close_bracket(cs: &[char], open: usize, end: usize) -> Option<usize> {
    let mut depth = 0;
    for (k, &c) in cs.iter().enumerate().take(end).skip(open) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the text of a line starts once its block markers are removed: quote `>`, list `-`/`*`/`+`/`1.`, task
/// `[ ]`, heading `#`.
fn content_start(cs: &[char], mut i: usize, end: usize) -> usize {
    let skip_ws = |mut i: usize| {
        while i < end && (cs[i] == ' ' || cs[i] == '\t') {
            i += 1;
        }
        i
    };
    i = skip_ws(i);
    while i < end && cs[i] == '>' {
        i = skip_ws(i + 1);
    }
    if i + 1 < end && matches!(cs[i], '-' | '*' | '+') && cs[i + 1] == ' ' {
        i = skip_ws(i + 1);
    } else {
        let mut j = i;
        while j < end && cs[j].is_ascii_digit() {
            j += 1;
        }
        if j > i && j + 1 < end && matches!(cs[j], '.' | ')') && cs[j + 1] == ' ' {
            i = skip_ws(j + 1);
        }
    }
    if i + 3 < end && cs[i] == '[' && cs[i + 2] == ']' && cs[i + 3] == ' ' {
        i = skip_ws(i + 3);
    }
    let mut j = i;
    while j < end && cs[j] == '#' {
        j += 1;
    }
    if j > i && (j == end || cs[j] == ' ') {
        i = skip_ws(j);
    }
    i
}

/// Where the text of a line ends: before a trailing block id ` ^name`.
fn content_end(cs: &[char], start: usize, mut end: usize) -> usize {
    while end > start && cs[end - 1].is_whitespace() {
        end -= 1;
    }
    let mut j = end;
    while j > start && !cs[j - 1].is_whitespace() {
        j -= 1;
    }
    if j + 1 < end && cs[j] == '^' && cs[j + 1] != '[' {
        let id_ok = cs[j + 1..end].iter().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'));
        if id_ok {
            end = j;
        }
    }
    end
}

pub fn extract(note: &str) -> Prose {
    let cs: Vec<char> = note.chars().collect();
    let mut o16 = Vec::with_capacity(cs.len() + 1);
    let mut pos = 0u32;
    for &c in &cs {
        o16.push(pos);
        pos += c.len_utf16() as u32;
    }
    o16.push(pos);
    let n = Note { cs, o16 };
    let mut out = Out { note: &n, p: Prose::default(), comment: false, html: false };
    let mut lines = Vec::new();
    let mut s = 0;
    for (i, &c) in n.cs.iter().enumerate() {
        if c == '\n' {
            lines.push((s, i));
            s = i + 1;
        }
    }
    lines.push((s, n.cs.len()));
    let trimmed = |(a, b): (usize, usize)| -> String { n.cs[a..b].iter().collect::<String>().trim().to_string() };
    let mut fence: Option<(char, usize)> = None;
    let (mut math, mut front) = (false, trimmed(lines[0]) == "---");
    for (k, &(a, b)) in lines.iter().enumerate() {
        let t = trimmed((a, b));
        if front {
            if k > 0 && (t == "---" || t == "...") {
                front = false;
            }
            continue;
        }
        if let Some((m, len)) = fence {
            if t.starts_with(&m.to_string().repeat(len)) && t.chars().all(|c| c == m) {
                fence = None;
            }
            continue;
        }
        if math {
            math = !t.contains("$$");
            continue;
        }
        if out.comment || out.html {
            out.inline(a, b); // finds the end of the comment; any text after it is kept
            out.end_line(a);
            continue;
        }
        let first = t.chars().next();
        if let Some(m @ ('`' | '~')) = first {
            let len = t.chars().take_while(|&c| c == m).count();
            if len >= 3 {
                fence = Some((m, len));
                continue;
            }
        }
        if let Some(rest) = t.strip_prefix("$$") {
            math = !rest.contains("$$");
            continue;
        }
        let marks: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        let rule = marks.len() >= 3 && ["-", "_", "*"].iter().any(|m| marks.chars().all(|c| c.to_string() == *m));
        let block_id = t.starts_with('^') && !t.starts_with("^[") && !t.contains(' ');
        if t.is_empty() || t.starts_with('|') || rule || block_id || t.starts_with("> [!") {
            out.end_line(a);
            continue;
        }
        let start = content_start(&n.cs, a, b);
        let end = content_end(&n.cs, start, b);
        out.inline(start, end);
        out.end_line(a);
    }
    while out.p.chars.last() == Some(&'\n') {
        out.p.chars.pop();
        out.p.src.pop();
    }
    out.p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> String {
        extract(s).text()
    }

    #[test]
    fn markup_is_removed() {
        let note = "---\ntitle: x\n---\n# Literature Review\n^sec--lit\n\nThe **model** is *small* [@smith2020, p. 3] and @jones2021 agrees.^[A footnote.] See [[glossary#ROUGE|ROUGE]] and [[Some Note]] (Table [[#^tab--x|tab:x]]).\n%%[flag] check this%%\n\n```tikz\n\\draw (0,0);\n```\n\n$$\nx = 1\n$$\n^eq--one\n\n| a | b |\n|---|---|\n\n- **H2a** Caching raises $\\alpha$ error. ^hyp2a\n((<tag|Figure)) The pipeline [[EV-0001]], [[EV-0002]].";
        assert_eq!(text(note), "Literature Review\nThe model is small and Jones (2021) agrees. See ROUGE and Some Note (Table 1).\nH2a Caching raises alpha error.\nThe pipeline.");
    }

    #[test]
    fn inline_maths_is_sent_as_its_symbols() {
        assert_eq!(text(r"A value $r$ maps to $q = \mathrm{round}(r/S) + Z$ here."), "A value r maps to q = round(r/S) + Z here.");
        assert_eq!(text(r"Rate $\alpha \le 0.05$ and $\frac{a+b+c+d}{e+f+g+h} + \sum_{i=1}^{n} x_i y_i z_i w_i$."), "Rate alpha ≤ 0.05 and the formula.");
    }

    #[test]
    fn sources_are_utf16_ranges() {
        let note = "😀 A **b**.";
        let p = extract(note);
        assert_eq!(p.text(), "😀 A b.");
        assert_eq!(p.src[0], (0, 2)); // the emoji is two UTF-16 units
        assert_eq!(p.src[4], (7, 8)); // b, after "😀 A **"
    }

    #[test]
    fn comments_span_lines_and_percent_is_unescaped() {
        assert_eq!(text("One 5\\% rise %% a\nb %% two.\n\nThree."), "One 5% rise\ntwo.\nThree.");
    }

    #[test]
    fn links_and_selection() {
        let note = "First [shown text](https://x.y) here.\n\nSecond line.";
        let p = extract(note);
        assert_eq!(p.text(), "First shown text here.\nSecond line.");
        let second = note.find("Second").unwrap() as u32;
        assert_eq!(p.within(second, note.len() as u32).text(), "Second line.");
    }
}

#[cfg(test)]
mod real {
    /// `AUTH_NOTE=<path> cargo test -p authenticity-core real -- --ignored --nocapture`: a note's prose, to read.
    #[test]
    #[ignore]
    fn print_prose() {
        let path = std::env::var("AUTH_NOTE").expect("AUTH_NOTE");
        let p = super::extract(&std::fs::read_to_string(path).unwrap());
        println!("{}\n\n{} words", p.text(), p.words());
    }
}
