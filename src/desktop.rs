// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop entry files: read and edit keys while keeping every other line,
//! comment, translation and action group as it was.

pub const MAIN: &str = "Desktop Entry";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopFile {
    /// (group the line belongs to, raw line). Lines before any group have "".
    lines: Vec<(String, String)>,
}

impl DesktopFile {
    pub fn parse(text: &str) -> Self {
        let mut group = String::new();
        let lines = text
            .lines()
            .map(|line| {
                let t = line.trim();
                if t.starts_with('[') && t.ends_with(']') {
                    group = t[1..t.len() - 1].to_string();
                }
                (group.clone(), line.to_string())
            })
            .collect();
        Self { lines }
    }
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (_, line) in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
    pub fn has_group(&self, group: &str) -> bool {
        self.groups().iter().any(|g| g == group)
    }
    pub fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = Vec::new();
        for (g, line) in &self.lines {
            if line.trim().starts_with('[') && !groups.contains(g) {
                groups.push(g.clone());
            }
        }
        groups
    }
    fn find(&self, group: &str, key: &str) -> Option<usize> {
        self.lines.iter().position(|(g, line)| {
            g == group
                && line
                    .split_once('=')
                    .is_some_and(|(k, _)| k.trim() == key && !line.trim_start().starts_with('#'))
        })
    }
    /// The unescaped value of an untranslated key.
    pub fn get(&self, group: &str, key: &str) -> Option<String> {
        let i = self.find(group, key)?;
        let value = self.lines[i].1.split_once('=')?.1.trim_start();
        Some(unescape_value(value))
    }
    pub fn main(&self, key: &str) -> Option<String> {
        self.get(MAIN, key).filter(|v| !v.is_empty())
    }
    pub fn bool(&self, key: &str) -> bool {
        self.main(key).is_some_and(|v| v == "true")
    }
    /// Set a key, replacing it in place or appending it to the group.
    pub fn set(&mut self, group: &str, key: &str, value: &str) {
        let line = format!("{key}={}", escape_value(value));
        if let Some(i) = self.find(group, key) {
            self.lines[i].1 = line;
            return;
        }
        if !self.has_group(group) {
            if self.lines.last().is_some_and(|(_, l)| !l.trim().is_empty()) {
                self.lines.push((group.into(), String::new()));
            }
            self.lines.push((group.into(), format!("[{group}]")));
            self.lines.push((group.into(), line));
            return;
        }
        // After the group's last non-empty line.
        let last = self
            .lines
            .iter()
            .rposition(|(g, l)| g == group && !l.trim().is_empty())
            .unwrap_or(0);
        self.lines.insert(last + 1, (group.into(), line));
    }
    pub fn remove(&mut self, group: &str, key: &str) {
        while let Some(i) = self.find(group, key) {
            self.lines.remove(i);
        }
    }
    /// Desktop actions groups, e.g. "Desktop Action new-window".
    pub fn action_groups(&self) -> Vec<String> {
        self.groups()
            .into_iter()
            .filter(|g| g.starts_with("Desktop Action "))
            .collect()
    }
}

/// Value escapes from the Desktop Entry spec: \s \n \t \r \\.
pub fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn escape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    if out.starts_with(' ') {
        out.replace_range(0..1, "\\s");
    }
    out
}

/// Split a command line into arguments. Accepts the Desktop Entry spec's
/// double quotes and, like GIO, POSIX single quotes (Gear Lever writes those).
pub fn split_command(command: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            '"' => {
                in_arg = true;
                loop {
                    match chars.next() {
                        None => return Err("Unclosed double quote".into()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '`' | '$' | '\\')) => current.push(e),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err("Unclosed double quote".into()),
                        },
                        Some(other) => current.push(other),
                    }
                }
            }
            '\'' => {
                in_arg = true;
                loop {
                    match chars.next() {
                        None => return Err("Unclosed single quote".into()),
                        Some('\'') => break,
                        Some(other) => current.push(other),
                    }
                }
            }
            '\\' => {
                in_arg = true;
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            other => {
                in_arg = true;
                current.push(other);
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}

const RESERVED: &[char] = &[
    ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')',
    '`',
];

/// Quote one argument per the Desktop Entry spec (double quotes when needed).
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(RESERVED) {
        return arg.into();
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn join_command(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An integrated app's Exec line: `[env K=V ...] PROGRAM ARGS...`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exec {
    pub env: Vec<(String, String)>,
    pub program: String,
    pub args: Vec<String>,
}

pub const FIELD_CODES: [&str; 9] = ["%f", "%F", "%u", "%U", "%i", "%c", "%k", "%d", "%D"];

impl Exec {
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut parts = split_command(value)?.into_iter().peekable();
        let mut env = Vec::new();
        if parts.peek().map(String::as_str) == Some("env") {
            parts.next();
            while let Some(p) = parts.peek() {
                match p.split_once('=') {
                    Some((k, v)) if !k.is_empty() && !k.starts_with('-') && valid_env_key(k) => {
                        env.push((k.to_string(), v.to_string()));
                        parts.next();
                    }
                    _ => break,
                }
            }
        }
        let program = parts.next().ok_or("Exec has no program")?;
        Ok(Self {
            env,
            program,
            args: parts.collect(),
        })
    }
    pub fn render(&self) -> String {
        let mut all = Vec::new();
        if !self.env.is_empty() {
            all.push("env".to_string());
            all.extend(self.env.iter().map(|(k, v)| format!("{k}={v}")));
        }
        all.push(self.program.clone());
        all.extend(self.args.iter().cloned());
        join_command(&all)
    }
    /// Arguments as the user edits them: one line, shell-style quoting.
    pub fn arguments_text(&self) -> String {
        join_command(&self.args)
    }
}

pub fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    const GEARLEVER: &str = "[Desktop Entry]\nType=Application\nName=Claude\nName[de]=Claude DE\nIcon=/home/u/AppImages/.icons/claude\nTryExec=/home/u/AppImages/claude.appimage\nExec=env DESKTOPINTEGRATION=1 FOO='a b' /home/u/AppImages/claude.appimage --no-sandbox %u\nX-AppImage-Version=1.0\n\n[Desktop Action new]\nName=New\nExec=/home/u/AppImages/claude.appimage --new\n";
    #[test]
    fn edits_keep_other_lines_and_groups() {
        let mut f = DesktopFile::parse(GEARLEVER);
        assert_eq!(f.main("Name").as_deref(), Some("Claude"));
        f.set(MAIN, "X-AppImage-Version", "2.0");
        f.set(MAIN, "X-New", "v");
        f.set("Desktop Action new", "Icon", "/x.png");
        let text = f.text();
        assert!(text.contains("Name[de]=Claude DE\n"));
        assert!(text.contains("X-AppImage-Version=2.0\nX-New=v\n\n[Desktop Action new]"));
        assert!(text.ends_with("--new\nIcon=/x.png\n"));
        assert_eq!(f.action_groups(), ["Desktop Action new"]);
    }
    #[test]
    fn exec_lines_from_gear_lever_parse_and_render_to_the_spec() {
        let f = DesktopFile::parse(GEARLEVER);
        let exec = Exec::parse(&f.main("Exec").unwrap()).unwrap();
        assert_eq!(
            exec.env,
            [
                ("DESKTOPINTEGRATION".into(), "1".into()),
                ("FOO".into(), "a b".into())
            ]
        );
        assert_eq!(exec.program, "/home/u/AppImages/claude.appimage");
        assert_eq!(exec.args, ["--no-sandbox", "%u"]);
        assert_eq!(
            exec.render(),
            "env DESKTOPINTEGRATION=1 \"FOO=a b\" /home/u/AppImages/claude.appimage --no-sandbox %u"
        );
        assert_eq!(Exec::parse(&exec.render()).unwrap(), exec);
    }
    #[test]
    fn quoting_round_trips_awkward_paths() {
        let args: Vec<String> = ["/home/u/My Apps/a$b\"c\\d.AppImage", "--x=1", "", "%F"]
            .map(String::from)
            .to_vec();
        let line = join_command(&args);
        let stored = escape_value(&line);
        assert_eq!(split_command(&unescape_value(&stored)).unwrap(), args);
        assert!(split_command("\"open").is_err());
    }
}
