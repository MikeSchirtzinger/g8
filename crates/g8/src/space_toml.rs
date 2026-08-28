//! `.g8/space.toml` read-modify-write — SPEC Decision 3 / ARCHITECTURE §10.2.
//!
//! The TOML file at the space root is the durable ConvergenceSpace
//! declaration ("ConvergenceSpace is declared by `.g8/space.toml`", SPEC
//! locked text); the store's `project` rows are the queryable mirror.
//! `g8 space add` therefore writes BOTH — the store row and the member
//! entry here.
//!
//! Round-trips through `toml::Value` so sections this module doesn't model
//! (`[substrate_budgets]`, `[links]`) survive a rewrite untouched. Comments
//! and formatting are not preserved (v0.1 limitation of the value-level
//! round-trip).

use std::path::Path;

use anyhow::{Context, Result};
use toml::value::{Table, Value};

/// Identity of the space, written to `[space]` on first creation only —
/// existing `[space]` values are never clobbered on subsequent adds.
pub struct SpaceIdentity<'a> {
    pub id: &'a str,
    pub name: &'a str,
}

/// Add (or update, keyed by member name) one `[[members]]` entry.
///
/// `member_root` is recorded exactly as the user gave it — SPEC Decision 3
/// allows "relative or absolute paths to project directories", and its own
/// example annotates a relative root with "relative to space.toml", so a
/// relative argument must stay relative rather than being canonicalized.
pub fn upsert_member(
    g8_dir: &Path,
    space: &SpaceIdentity<'_>,
    member_name: &str,
    member_root: &str,
) -> Result<()> {
    let path = g8_dir.join("space.toml");
    let mut root = load_or_empty(&path)?;

    let table = root
        .as_table_mut()
        .context("space.toml top level is not a TOML table")?;

    let space_table = table
        .entry("space")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .context("space.toml [space] is not a table")?;
    space_table
        .entry("id")
        .or_insert_with(|| Value::String(space.id.to_string()));
    space_table
        .entry("name")
        .or_insert_with(|| Value::String(space.name.to_string()));
    space_table
        .entry("description")
        .or_insert_with(|| Value::String(String::new()));

    let members = table
        .entry("members")
        .or_insert_with(|| Value::Array(vec![]))
        .as_array_mut()
        .context("space.toml [[members]] is not an array")?;

    let mut entry = Table::new();
    entry.insert("name".into(), Value::String(member_name.to_string()));
    entry.insert("root".into(), Value::String(member_root.to_string()));

    match members
        .iter_mut()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(member_name))
    {
        Some(existing) => *existing = Value::Table(entry),
        None => members.push(Value::Table(entry)),
    }

    write(&path, &root)
}

/// Remove the `[[members]]` entry with the given name, if the file and the
/// entry exist. A missing file or member is not an error — `space remove`
/// stays usable against stores predating the space.toml write.
pub fn remove_member(g8_dir: &Path, member_name: &str) -> Result<()> {
    let path = g8_dir.join("space.toml");
    if !path.exists() {
        return Ok(());
    }
    let mut root = load_or_empty(&path)?;
    let Some(members) = root
        .as_table_mut()
        .and_then(|t| t.get_mut("members"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    members.retain(|m| m.get("name").and_then(Value::as_str) != Some(member_name));
    write(&path, &root)
}

fn load_or_empty(path: &Path) -> Result<Value> {
    if path.exists() {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        content
            .parse::<Value>()
            .with_context(|| format!("parsing {}", path.display()))
    } else {
        Ok(Value::Table(Table::new()))
    }
}

fn write(path: &Path, root: &Value) -> Result<()> {
    let content = toml::to_string_pretty(root).context("serializing space.toml")?;
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> SpaceIdentity<'static> {
        SpaceIdentity {
            id: "space-test-id",
            name: "default",
        }
    }

    #[test]
    fn creates_file_with_space_header_and_member() {
        let dir = tempfile::tempdir().unwrap();
        upsert_member(dir.path(), &identity(), "member", "../member").unwrap();

        let content = std::fs::read_to_string(dir.path().join("space.toml")).unwrap();
        let parsed: Value = content.parse().unwrap();
        assert_eq!(
            parsed["space"]["id"].as_str(),
            Some("space-test-id"),
            "content: {content}"
        );
        assert_eq!(parsed["members"][0]["name"].as_str(), Some("member"));
        assert_eq!(parsed["members"][0]["root"].as_str(), Some("../member"));
    }

    #[test]
    fn second_member_appends_without_clobbering_first() {
        let dir = tempfile::tempdir().unwrap();
        upsert_member(dir.path(), &identity(), "a", "../a").unwrap();
        upsert_member(dir.path(), &identity(), "b", "/abs/b").unwrap();

        let content = std::fs::read_to_string(dir.path().join("space.toml")).unwrap();
        let parsed: Value = content.parse().unwrap();
        let members = parsed["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[1]["root"].as_str(), Some("/abs/b"));
    }

    #[test]
    fn re_adding_same_name_updates_root_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        upsert_member(dir.path(), &identity(), "m", "../old").unwrap();
        upsert_member(dir.path(), &identity(), "m", "../new").unwrap();

        let parsed: Value = std::fs::read_to_string(dir.path().join("space.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let members = parsed["members"].as_array().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0]["root"].as_str(), Some("../new"));
    }

    #[test]
    fn unmodeled_sections_survive_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("space.toml"),
            r#"
[space]
id = "keep-this-id"
name = "platform"

[substrate_budgets]
"export-pipeline" = { wip_cap = 5 }
"#,
        )
        .unwrap();

        upsert_member(dir.path(), &identity(), "m", "../m").unwrap();

        let parsed: Value = std::fs::read_to_string(dir.path().join("space.toml"))
            .unwrap()
            .parse()
            .unwrap();
        // Existing [space] identity is never clobbered.
        assert_eq!(parsed["space"]["id"].as_str(), Some("keep-this-id"));
        // Unmodeled section survives.
        assert_eq!(
            parsed["substrate_budgets"]["export-pipeline"]["wip_cap"].as_integer(),
            Some(5)
        );
        assert_eq!(parsed["members"][0]["name"].as_str(), Some("m"));
    }

    #[test]
    fn remove_member_drops_entry_and_tolerates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        // Missing file: no error.
        remove_member(dir.path(), "ghost").unwrap();

        upsert_member(dir.path(), &identity(), "a", "../a").unwrap();
        upsert_member(dir.path(), &identity(), "b", "../b").unwrap();
        remove_member(dir.path(), "a").unwrap();

        let parsed: Value = std::fs::read_to_string(dir.path().join("space.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let members = parsed["members"].as_array().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0]["name"].as_str(), Some("b"));
    }
}
