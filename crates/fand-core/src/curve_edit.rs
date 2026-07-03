//! Surgical curve-point edits to a config's raw TOML text.
//!
//! `get_config`/`set_config` on the wire round-trip the *entire* config
//! file as text (comments and formatting included). A client that wants to
//! change one curve's points must not do a parse → mutate → reserialize
//! round trip through `Config`/`toml`: that would silently discard every
//! comment in the file, including the hand-written hardware-safety
//! rationale above each channel. `replace_curve_points` uses `toml_edit`
//! to touch only the one `points` line, leaving everything else untouched.

use thiserror::Error;
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

#[derive(Debug, Error)]
pub enum CurveEditError {
    #[error("parsing current config: {0}")]
    Parse(#[from] toml_edit::TomlError),
    #[error("`[curves]` is not a table")]
    CurvesNotATable,
    #[error("`[curves.{0}]` is not a table")]
    CurveNotATable(String),
    #[error("curve `{0}` does not exist")]
    UnknownCurve(String),
}

/// Replace `[curves.<name>].points` in `toml_text`, keeping every comment
/// and all formatting elsewhere byte-identical. Creates the curve's table
/// if it doesn't exist yet (the caller decides whether that's worth
/// announcing — this function has no I/O).
pub fn replace_curve_points(
    toml_text: &str,
    name: &str,
    points: &[(i32, u8)],
) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let curves = doc
        .entry("curves")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or(CurveEditError::CurvesNotATable)?;
    let curve = curves
        .entry(name)
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| CurveEditError::CurveNotATable(name.to_string()))?;

    let mut arr = Array::new();
    for &(temp, pwm) in points {
        let mut pair = Array::new();
        pair.push(i64::from(temp));
        pair.push(i64::from(pwm));
        arr.push(Value::Array(pair));
    }
    curve["points"] = value(arr);
    Ok(doc.to_string())
}

/// Remove `[curves.<name>]` entirely, keeping every comment and all
/// formatting elsewhere byte-identical. The caller is responsible for
/// checking nothing still references this curve (or letting the daemon's
/// `Config::validate` reject the result with `UnknownCurve`).
pub fn remove_curve(toml_text: &str, name: &str) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let removed = doc
        .get_mut("curves")
        .and_then(Item::as_table_mut)
        .and_then(|curves| curves.remove(name));
    if removed.is_none() {
        return Err(CurveEditError::UnknownCurve(name.to_string()));
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_curve_points_keeps_comments() {
        let toml = "\
# top comment stays
[curves.cpu]
# curve comment stays
points = [[40, 80], [70, 200]]

[curves.gpu]
points = [[35, 70], [80, 255]] # trailing comment stays
";
        let out = replace_curve_points(toml, "cpu", &[(30, 70), (60, 140), (85, 255)]).unwrap();
        assert!(out.contains("# top comment stays"));
        assert!(out.contains("# curve comment stays"));
        assert!(out.contains("# trailing comment stays"));
        assert!(out.contains("points = [[30, 70], [60, 140], [85, 255]]"));
        assert!(out.contains("[[35, 70], [80, 255]]"), "other curve untouched");
    }

    #[test]
    fn replace_curve_points_can_create_a_curve() {
        let toml = "[curves.cpu]\npoints = [[40, 80], [70, 200]]\n";
        let out = replace_curve_points(toml, "case", &[(30, 70), (80, 255)]).unwrap();
        assert!(out.contains("[curves.case]"));
        assert!(out.contains("points = [[30, 70], [80, 255]]"));
    }

    #[test]
    fn remove_curve_keeps_comments() {
        let toml = "\
# top comment stays
[curves.cpu]
# curve comment stays
points = [[40, 80], [70, 200]]

[curves.gpu]
points = [[35, 70], [80, 255]] # trailing comment stays
";
        let out = remove_curve(toml, "gpu").unwrap();
        assert!(out.contains("# top comment stays"));
        assert!(out.contains("# curve comment stays"));
        assert!(out.contains("points = [[40, 80], [70, 200]]"));
        assert!(!out.contains("[curves.gpu]"));
        assert!(!out.contains("trailing comment stays"));
    }

    #[test]
    fn remove_curve_rejects_unknown_name() {
        let toml = "[curves.cpu]\npoints = [[40, 80], [70, 200]]\n";
        assert!(matches!(
            remove_curve(toml, "nope"),
            Err(CurveEditError::UnknownCurve(name)) if name == "nope"
        ));
    }
}
