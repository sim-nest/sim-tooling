//! Parsing of the split-cut configuration that groups packages for repo-contract.

use std::collections::{BTreeMap, BTreeSet};

pub(crate) const CONTRACT_CUT_PATH: &str = "repo-contract-cut.toml";

#[derive(Debug, Clone)]
pub(crate) struct SplitCut {
    pub group_order: Vec<String>,
    pub groups: BTreeMap<String, Vec<String>>,
    pub package_groups: BTreeMap<String, String>,
}

pub(crate) fn parse_split_cut(text: &str) -> Result<SplitCut, String> {
    let mut group_order = Vec::new();
    let mut groups = BTreeMap::new();
    let mut section = String::new();
    let mut active_array: Option<String> = None;
    let mut active_items = Vec::new();

    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name) = active_array.clone() {
            active_items.extend(quoted_strings(line));
            if line.contains(']') {
                if name == "group_order" {
                    group_order = std::mem::take(&mut active_items);
                } else {
                    groups.insert(name, std::mem::take(&mut active_items));
                }
                active_array = None;
            }
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']).to_owned();
            continue;
        }

        if let Some((key, rest)) = line.split_once('=') {
            let key = key.trim().to_owned();
            let rest = rest.trim();
            if rest.starts_with('[') {
                active_items = quoted_strings(rest);
                if rest.contains(']') {
                    if key == "group_order" {
                        group_order = std::mem::take(&mut active_items);
                    } else if section == "groups" {
                        groups.insert(key, std::mem::take(&mut active_items));
                    }
                } else if key == "group_order" || section == "groups" {
                    active_array = Some(key);
                }
            }
        }
    }

    if let Some(name) = active_array {
        return Err(format!("unterminated array in split cut: {name}"));
    }
    if group_order.is_empty() {
        return Err("split cut missing group_order".to_owned());
    }
    if groups.is_empty() {
        return Err("split cut missing [groups] entries".to_owned());
    }

    let mut package_groups = BTreeMap::new();
    for (group, packages) in &groups {
        if !group_order.iter().any(|known| known == group) {
            return Err(format!("group {group} is not listed in group_order"));
        }
        for package in packages {
            if let Some(previous) = package_groups.insert(package.clone(), group.clone()) {
                return Err(format!(
                    "package {package} appears in both {previous} and {group}"
                ));
            }
        }
    }

    let ordered = group_order.iter().cloned().collect::<BTreeSet<_>>();
    let configured = groups.keys().cloned().collect::<BTreeSet<_>>();
    if let Some(group) = ordered.difference(&configured).next() {
        return Err(format!("group {group} has no package list"));
    }

    Ok(SplitCut {
        group_order,
        groups,
        package_groups,
    })
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(head, _)| head)
}

fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('"') else {
            break;
        };
        out.push(after_start[..end].to_owned());
        rest = &after_start[end + 1..];
    }
    out
}
