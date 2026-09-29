// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small strict reader for the block-style YAML of GitHub Actions
//! workflows: mappings, sequences, plain and quoted scalars, block scalars
//! (`|`, `>`), and flow sequences of scalars. Comments are dropped by the
//! lexer, never by search. Anything else (tabs, anchors, aliases, tags,
//! complex keys, flow mappings, further documents, inconsistent indentation)
//! is refused, so a workflow the reader accepts means what the tree says.

/// A parsed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Yaml {
    /// A scalar; a block scalar keeps its newlines.
    Scalar(String),
    /// A sequence.
    Seq(Vec<Yaml>),
    /// A mapping, in document order.
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    /// The value of `key` in a mapping.
    pub(crate) fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Self::Map(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The text of a scalar.
    pub(crate) fn scalar(&self) -> Option<&str> {
        match self {
            Self::Scalar(text) => Some(text),
            _ => None,
        }
    }
}

struct Line {
    indent: usize,
    text: String,
    raw: String,
}

/// Parses one workflow document.
pub(crate) fn parse(source: &str) -> Result<Yaml, String> {
    let mut lines = Vec::new();
    let mut started = false;
    for raw in source.lines() {
        let trimmed = raw.trim_start_matches(' ');
        if trimmed.starts_with('\t') || raw[..raw.len() - trimmed.len()].contains('\t') {
            return Err("tab indentation".to_owned());
        }
        if trimmed == "---" && !started {
            started = true;
            continue;
        }
        if trimmed == "---" || trimmed == "..." {
            return Err("more than one document".to_owned());
        }
        let text = strip_comment(trimmed).trim_end().to_owned();
        if !text.is_empty() {
            started = true;
        }
        lines.push(Line {
            indent: raw.len() - trimmed.len(),
            text,
            raw: raw.to_owned(),
        });
    }
    let mut parser = Parser { lines, at: 0 };
    parser.skip_blank();
    let Some(first) = parser.lines.get(parser.at) else {
        return Err("empty document".to_owned());
    };
    let indent = first.indent;
    let value = parser.block(indent)?;
    parser.skip_blank();
    if parser.at < parser.lines.len() {
        return Err(format!("unexpected content at line {}", parser.at + 1));
    }
    Ok(value)
}

/// The line without a trailing comment: `#` starts one at the start of the
/// line or after whitespace, outside quotes.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (index, ch) in line.char_indices() {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => {
                if previous == ' ' || previous == '[' || previous == ',' || previous == ':' {
                    quote = Some(ch);
                }
            }
            None if ch == '#' && previous.is_whitespace() => return &line[..index],
            None => {}
        }
        previous = ch;
    }
    line
}

struct Parser {
    lines: Vec<Line>,
    at: usize,
}

impl Parser {
    fn skip_blank(&mut self) {
        while self
            .lines
            .get(self.at)
            .is_some_and(|line| line.text.is_empty())
        {
            self.at += 1;
        }
    }

    fn block(&mut self, indent: usize) -> Result<Yaml, String> {
        self.skip_blank();
        let line = self.lines.get(self.at).ok_or("missing value")?;
        if line.indent != indent {
            return Err(format!("indentation at line {}", self.at + 1));
        }
        if line.text == "-" || line.text.starts_with("- ") {
            self.sequence(indent)
        } else {
            self.mapping(indent)
        }
    }

    fn sequence(&mut self, indent: usize) -> Result<Yaml, String> {
        let mut items = Vec::new();
        loop {
            self.skip_blank();
            let Some(line) = self.lines.get(self.at) else {
                break;
            };
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(format!("indentation at line {}", self.at + 1));
            }
            if !(line.text == "-" || line.text.starts_with("- ")) {
                break;
            }
            let rest = line.text[1..].trim_start().to_owned();
            if rest.is_empty() {
                self.at += 1;
                self.skip_blank();
                let next = self.lines.get(self.at).ok_or("missing sequence item")?;
                if next.indent <= indent {
                    return Err("empty sequence item".to_owned());
                }
                let inner = next.indent;
                items.push(self.block(inner)?);
                continue;
            }
            // `- key: v` opens a mapping whose keys sit at the column of the
            // first key; `- scalar` is a scalar.
            let column = line.indent + (line.text.len() - rest.len());
            if looks_like_entry(&rest) {
                let line = &mut self.lines[self.at];
                line.indent = column;
                line.text = rest;
                items.push(self.mapping(column)?);
            } else {
                let value = scalar(&rest)?;
                self.at += 1;
                items.push(value);
            }
        }
        Ok(Yaml::Seq(items))
    }

    fn mapping(&mut self, indent: usize) -> Result<Yaml, String> {
        let mut entries: Vec<(String, Yaml)> = Vec::new();
        loop {
            self.skip_blank();
            let Some(line) = self.lines.get(self.at) else {
                break;
            };
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(format!("indentation at line {}", self.at + 1));
            }
            if line.text == "-" || line.text.starts_with("- ") {
                break;
            }
            let text = line.text.clone();
            let (key, value) = split_entry(&text)
                .ok_or_else(|| format!("not a mapping entry at line {}", self.at + 1))?;
            if entries.iter().any(|(name, _)| *name == key) {
                return Err(format!("duplicate key {key:?}"));
            }
            self.at += 1;
            let parsed = if value.is_empty() {
                self.skip_blank();
                match self.lines.get(self.at) {
                    Some(next) if next.indent > indent => {
                        let inner = next.indent;
                        self.block(inner)?
                    }
                    // A sequence may sit at the key's own indent.
                    Some(next)
                        if next.indent == indent
                            && (next.text == "-" || next.text.starts_with("- ")) =>
                    {
                        self.sequence(indent)?
                    }
                    _ => Yaml::Scalar(String::new()),
                }
            } else if value.starts_with('|') || value.starts_with('>') {
                self.block_scalar(indent, value)?
            } else {
                scalar(value)?
            };
            entries.push((key, parsed));
        }
        Ok(Yaml::Map(entries))
    }

    /// A block scalar: the following lines indented deeper than `indent`.
    fn block_scalar(&mut self, indent: usize, header: &str) -> Result<Yaml, String> {
        if !header[1..].chars().all(|ch| "+-0123456789".contains(ch)) {
            return Err("unsupported block scalar header".to_owned());
        }
        let mut body: Vec<&str> = Vec::new();
        let mut content_indent = None;
        while let Some(line) = self.lines.get(self.at) {
            let raw = line.raw.as_str();
            let blank = raw.trim().is_empty();
            let leading = raw.len() - raw.trim_start_matches(' ').len();
            if !blank && leading <= indent {
                break;
            }
            let base = *content_indent.get_or_insert(leading);
            body.push(if blank || raw.len() < base {
                ""
            } else {
                &raw[base..]
            });
            self.at += 1;
        }
        Ok(Yaml::Scalar(
            body.join("\n").trim_end_matches('\n').to_owned(),
        ))
    }
}

fn looks_like_entry(text: &str) -> bool {
    split_entry(text).is_some()
}

/// `key: value` (value possibly empty); quoted keys are unquoted.
fn split_entry(text: &str) -> Option<(String, &str)> {
    if text.starts_with(['"', '\'']) {
        let quote = text.chars().next()?;
        let close = text[1..].find(quote)? + 1;
        let key = text[1..close].to_owned();
        let rest = text[close + 1..].strip_prefix(':')?;
        return (rest.is_empty() || rest.starts_with(' ')).then(|| (key, rest.trim_start()));
    }
    if text.starts_with(['[', '{', '&', '*', '!', '?', '|', '>', '%', '@', '`']) {
        return None;
    }
    let mut from = 0;
    while let Some(found) = text[from..].find(':') {
        let at = from + found;
        let after = &text[at + 1..];
        if after.is_empty() || after.starts_with(' ') {
            return Some((text[..at].to_owned(), after.trim_start()));
        }
        from = at + 1;
    }
    None
}

fn scalar(text: &str) -> Result<Yaml, String> {
    let text = text.trim();
    if text.starts_with(['&', '*', '!', '{']) {
        return Err(format!("unsupported construct in {text:?}"));
    }
    if let Some(inner) = text.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or("unterminated flow sequence")?;
        let mut items = Vec::new();
        if !inner.trim().is_empty() {
            for part in inner.split(',') {
                items.push(scalar(part)?);
            }
        }
        return Ok(Yaml::Seq(items));
    }
    for quote in ['"', '\''] {
        if let Some(rest) = text.strip_prefix(quote) {
            let inner = rest
                .strip_suffix(quote)
                .ok_or_else(|| format!("unterminated quote in {text:?}"))?;
            return Ok(Yaml::Scalar(inner.to_owned()));
        }
    }
    Ok(Yaml::Scalar(text.to_owned()))
}

#[cfg(test)]
#[path = "workflow_yaml_tests.rs"]
mod tests;
