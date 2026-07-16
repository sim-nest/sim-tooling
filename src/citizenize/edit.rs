use std::collections::BTreeMap;

use super::Candidate;

pub(super) fn edit_file(text: &str, domain: &str, candidates: &[Candidate]) -> String {
    let mut before = BTreeMap::<usize, Vec<String>>::new();
    let needs_import = !text.contains("sim_citizen_derive::Citizen");
    if needs_import {
        before
            .entry(import_line(text))
            .or_default()
            .push("use sim_citizen_derive::Citizen;".to_owned());
    }
    for candidate in candidates {
        let derive = if candidate.has_derive {
            "#[derive(Citizen)]".to_owned()
        } else {
            "#[derive(Clone, Debug, Default, PartialEq, Citizen)]".to_owned()
        };
        before.entry(candidate.insert_line).or_default().extend([
            derive,
            format!(
                "#[citizen(symbol = \"{}/{}\", version = 1)]",
                domain, candidate.name
            ),
            format!(
                "// TODO: validate citizen example fixture for {}",
                candidate.name
            ),
        ]);
        for line in &candidate.field_list_lines {
            before
                .entry(*line)
                .or_default()
                .push("#[citizen(list)]".to_owned());
        }
    }

    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if let Some(insertions) = before.get(&line_no) {
            let indent = leading_indent(line);
            for insertion in insertions {
                if insertion.starts_with("use ") {
                    out.push_str(insertion);
                } else {
                    out.push_str(indent);
                    out.push_str(insertion);
                }
                out.push('\n');
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn import_line(text: &str) -> usize {
    let mut line_no = 1;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//!") || trimmed.starts_with("#![") {
            line_no += 1;
            continue;
        }
        break;
    }
    line_no
}

fn leading_indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}
