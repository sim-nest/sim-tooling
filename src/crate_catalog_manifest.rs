//! Cargo manifest editing helpers for the crate-catalog task.

pub(crate) fn ensure_package_metadata(text: &str, description: &str) -> String {
    let mut lines = text.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let Some((start, end)) = package_range(&lines) else {
        return text.to_owned();
    };
    let mut insertions = Vec::new();
    if !has_package_key(&lines, start, end, "description") {
        insertions.push(format!("description = {}", toml_string(description)));
    }
    if !has_package_key(&lines, start, end, "readme") {
        insertions.push("readme = \"README.md\"".to_owned());
    }
    if !has_package_key(&lines, start, end, "publish") {
        // Cargo permits publication when `package.publish` is absent. Make
        // that effective value explicit without changing release semantics as
        // a side effect of regenerating documentation metadata.
        insertions.push("publish = true".to_owned());
    }
    if insertions.is_empty() {
        return text.to_owned();
    }
    let insert_at = package_insert_index(&lines, start, end);
    for (offset, line) in insertions.into_iter().enumerate() {
        lines.insert(insert_at + offset, line);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn package_range(lines: &[String]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|line| line.trim() == "[package]")?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| line.trim_start().starts_with('['))
        .map(|(index, _)| index)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn has_package_key(lines: &[String], start: usize, end: usize, key: &str) -> bool {
    lines[start + 1..end].iter().any(|line| {
        let trimmed = line.trim_start();
        trimmed
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    })
}

fn package_insert_index(lines: &[String], start: usize, end: usize) -> usize {
    for key in ["license", "license.workspace", "edition", "version", "name"] {
        if let Some(index) = lines[start + 1..end].iter().position(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        }) {
            return start + 1 + index + 1;
        }
    }
    start + 1
}

fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::ensure_package_metadata;

    #[test]
    fn missing_publish_preserves_cargo_default() {
        let manifest = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

        let updated = ensure_package_metadata(manifest, "Demo package");

        assert!(updated.contains("publish = true\n"));
        assert!(!updated.contains("publish = false\n"));
    }

    #[test]
    fn explicit_publish_policy_is_unchanged() {
        let manifest = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n";

        let updated = ensure_package_metadata(manifest, "Demo package");

        assert_eq!(updated.matches("publish = false").count(), 1);
        assert!(!updated.contains("publish = true\n"));
    }
}
