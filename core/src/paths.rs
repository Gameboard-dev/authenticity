//! A folder as the user writes it in a setting, with environment variables expanded: `%LOCALAPPDATA%\Programs\X`
//! (Windows), `$HOME/x` or `${HOME}/x` (elsewhere), and a leading `~` (the home folder, everywhere). The setting's own
//! description suggests the first form, so it must work as written.
//!
//! One pass, left to right: a variable's value is used as it is, never expanded again. A `%` that does not open a
//! variable name closed by another `%` (a folder called `100%`) is kept as written. A variable that is not set is an
//! error that names it, never an empty string (which would turn `%X%\SlopTotal` into the relative `\SlopTotal`).
//! What comes out is still only a path: the caller checks it (absolute, the program there) and never runs it through a
//! shell.

/// `text` with its variables expanded by `var` (the environment, or a stand-in in tests); `windows` chooses `%NAME%`
/// over `$NAME`.
pub fn expand(text: &str, var: impl Fn(&str) -> Option<String>, windows: bool) -> Result<String, String> {
    let text = text.trim();
    if text.contains(['\0', '\n', '\r']) {
        return Err("the folder holds a line break or a NUL character".into());
    }
    let unset = |name: &str| format!("the folder names the environment variable {name}, which is not set on this computer: write the full path instead");
    let name_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || (windows && matches!(c, '(' | ')'));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // a leading ~ (alone, or before a separator): the home folder
    if rest == "~" || rest.starts_with("~/") || (windows && rest.starts_with("~\\")) {
        let home = if windows { var("USERPROFILE").or_else(|| var("HOME")) } else { var("HOME") };
        out.push_str(&home.ok_or_else(|| unset(if windows { "%USERPROFILE%" } else { "$HOME" }))?);
        rest = &rest[1..];
    }
    while let Some(c) = rest.chars().next() {
        if windows && c == '%' {
            let body = &rest[1..];
            if let Some(end) = body.find('%') {
                let name = &body[..end];
                if !name.is_empty() && name.chars().all(name_char) {
                    out.push_str(&var(name).ok_or_else(|| unset(&format!("%{name}%")))?);
                    rest = &body[end + 1..];
                    continue;
                }
            }
        } else if !windows && c == '$' {
            let body = &rest[1..];
            let (name, used) = match body.strip_prefix('{') {
                Some(b) => match b.find('}') {
                    Some(end) => (&b[..end], end + 2),
                    None => ("", 0),
                },
                None => {
                    let n = body.find(|c: char| !name_char(c)).unwrap_or(body.len());
                    (&body[..n], n)
                }
            };
            if !name.is_empty() && name.chars().all(name_char) && !name.starts_with(|c: char| c.is_ascii_digit()) {
                out.push_str(&var(name).ok_or_else(|| unset(&format!("${name}")))?);
                rest = &body[used..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    Ok(out)
}

/// Whether `text` names anything [`expand`] would replace (to show the expanded folder beside the setting).
pub fn has_variables(text: &str, windows: bool) -> bool {
    let t = text.trim();
    t.starts_with('~') || if windows { t.matches('%').count() >= 2 } else { t.contains('$') }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<String> {
        match k {
            "LOCALAPPDATA" => Some("C:\\Users\\me\\AppData\\Local".into()),
            "USERPROFILE" => Some("C:\\Users\\me".into()),
            "ProgramFiles(x86)" => Some("C:\\Program Files (x86)".into()),
            "HOME" => Some("/home/me".into()),
            "XDG_DATA_HOME" => Some("/home/me/.local/share".into()),
            // a value that looks like a variable or a command stays as it is: one pass, and only ever a path
            "ODD" => Some("%LOCALAPPDATA% & calc".into()),
            _ => None,
        }
    }

    #[test]
    fn windows_variables_and_home() {
        let w = |t: &str| expand(t, env, true);
        assert_eq!(w("%LOCALAPPDATA%\\Programs\\SlopTotal").unwrap(), "C:\\Users\\me\\AppData\\Local\\Programs\\SlopTotal");
        assert_eq!(w("  %LOCALAPPDATA%\\Programs\\SlopTotal  ").unwrap(), "C:\\Users\\me\\AppData\\Local\\Programs\\SlopTotal");
        assert_eq!(w("%ProgramFiles(x86)%\\X").unwrap(), "C:\\Program Files (x86)\\X");
        assert_eq!(w("~\\SlopTotal").unwrap(), "C:\\Users\\me\\SlopTotal");
        assert_eq!(w("C:\\Tools\\SlopTotal").unwrap(), "C:\\Tools\\SlopTotal", "nothing to expand");
        assert_eq!(w("C:\\100%\\SlopTotal").unwrap(), "C:\\100%\\SlopTotal", "a lone % is kept");
        assert_eq!(w("C:\\a%b c%d").unwrap(), "C:\\a%b c%d", "not a variable name: kept");
        assert_eq!(w("C:\\x~y").unwrap(), "C:\\x~y", "~ only at the start");
        assert_eq!(w("$HOME\\x").unwrap(), "$HOME\\x", "no $ variables on Windows");
        assert_eq!(w("%ODD%\\x").unwrap(), "%LOCALAPPDATA% & calc\\x", "a value is not expanded again");
    }

    #[test]
    fn unix_variables_and_home() {
        let u = |t: &str| expand(t, env, false);
        assert_eq!(u("$HOME/sloptotal").unwrap(), "/home/me/sloptotal");
        assert_eq!(u("${XDG_DATA_HOME}/sloptotal").unwrap(), "/home/me/.local/share/sloptotal");
        assert_eq!(u("~/sloptotal").unwrap(), "/home/me/sloptotal");
        assert_eq!(u("~").unwrap(), "/home/me");
        assert_eq!(u("~other/x").unwrap(), "~other/x", "another user's home is not looked up");
        assert_eq!(u("/opt/$5/x").unwrap(), "/opt/$5/x");
        assert_eq!(u("/opt/${unclosed/x").unwrap(), "/opt/${unclosed/x");
        assert_eq!(u("%LOCALAPPDATA%/x").unwrap(), "%LOCALAPPDATA%/x", "no % variables elsewhere");
    }

    #[test]
    fn an_unset_variable_or_a_hostile_value_is_an_error() {
        let e = expand("%NOPE%\\SlopTotal", env, true).unwrap_err();
        assert!(e.contains("%NOPE%") && e.contains("not set"), "{e}");
        assert!(expand("$NOPE/x", env, false).unwrap_err().contains("$NOPE"));
        assert!(expand("~/x", |_| None, false).unwrap_err().contains("$HOME"));
        assert!(expand("C:\\x\nC:\\y", env, true).is_err());
        assert!(expand("C:\\x\0", env, true).is_err());
        // quotes and shell characters are only characters of a path
        assert_eq!(expand("C:\\x\" & calc & \"", env, true).unwrap(), "C:\\x\" & calc & \"");
    }

    #[test]
    fn variables_are_noticed() {
        assert!(has_variables("%LOCALAPPDATA%\\x", true));
        assert!(has_variables("~/x", false));
        assert!(!has_variables("C:\\100%\\x", true));
        assert!(!has_variables("C:\\Tools", true));
    }
}
