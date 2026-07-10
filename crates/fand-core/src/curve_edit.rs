//! Surgical curve edits to a config's raw TOML text.
//!
//! `get_config`/`set_config` on the wire round-trip the *entire* config
//! file as text (comments and formatting included). A client that wants to
//! change one curve must not do a parse → mutate → reserialize round trip
//! through `Config`/`toml`: that would silently discard every comment in
//! the file, including the hand-written hardware-safety rationale above
//! each channel. Everything here uses `toml_edit` to touch only the keys
//! being changed.
//!
//! Business rules (unknown references, cycles, ranges) are not re-checked
//! here — `Config::validate` is the single source of truth, applied by the
//! caller after this module hands back the edited text.

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
    #[error("curve `{0}` already exists")]
    DuplicateCurve(String),
    #[error("curve `{curve}` is not a `{expected}` curve")]
    WrongKind {
        curve: String,
        expected: &'static str,
    },
    #[error("curve `{curve}` already contains member `{member}`")]
    DuplicateMember { curve: String, member: String },
    #[error("curve `{curve}` has no member `{member}`")]
    UnknownMember { curve: String, member: String },
    #[error("curve `{0}` needs at least one member")]
    CannotRemoveLastMember(String),
}

fn curves_table(doc: &mut DocumentMut) -> Result<&mut Table, CurveEditError> {
    doc.entry("curves")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or(CurveEditError::CurvesNotATable)
}

/// The existing `[curves.<name>]` table, checked to be of `kind`.
fn curve_of_kind<'a>(
    doc: &'a mut DocumentMut,
    name: &str,
    kind: &'static str,
) -> Result<&'a mut Table, CurveEditError> {
    let curve = curves_table(doc)?
        .get_mut(name)
        .ok_or_else(|| CurveEditError::UnknownCurve(name.to_string()))?
        .as_table_mut()
        .ok_or_else(|| CurveEditError::CurveNotATable(name.to_string()))?;
    if curve.get("kind").and_then(Item::as_str) != Some(kind) {
        return Err(CurveEditError::WrongKind {
            curve: name.to_string(),
            expected: kind,
        });
    }
    Ok(curve)
}

fn points_array(points: &[(i32, u8)]) -> Array {
    let mut arr = Array::new();
    for &(temp, pwm) in points {
        let mut pair = Array::new();
        pair.push(i64::from(temp));
        pair.push(i64::from(pwm));
        arr.push(Value::Array(pair));
    }
    arr
}

/// Replace an existing graph curve's `points`, keeping every comment and
/// all formatting elsewhere byte-identical. Editing a curve that doesn't
/// exist is an error — creating one needs a sensor, see
/// [`create_graph_curve`].
pub fn replace_curve_points(
    toml_text: &str,
    name: &str,
    points: &[(i32, u8)],
) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let curve = curve_of_kind(&mut doc, name, "graph")?;
    curve["points"] = value(points_array(points));
    Ok(doc.to_string())
}

/// Create a new graph curve bound to `sensor`. The caller validates that
/// the sensor exists (via `Config::validate` on the result).
pub fn create_graph_curve(
    toml_text: &str,
    name: &str,
    sensor: &str,
    points: &[(i32, u8)],
) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let curves = curves_table(&mut doc)?;
    if curves.contains_key(name) {
        return Err(CurveEditError::DuplicateCurve(name.to_string()));
    }
    let mut curve = Table::new();
    curve["kind"] = value("graph");
    curve["sensor"] = value(sensor);
    curve["points"] = value(points_array(points));
    curves.insert(name, Item::Table(curve));
    Ok(doc.to_string())
}

/// Rebind which sensor drives an existing graph curve.
pub fn set_graph_sensor(
    toml_text: &str,
    name: &str,
    sensor: &str,
) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let curve = curve_of_kind(&mut doc, name, "graph")?;
    curve["sensor"] = value(sensor);
    Ok(doc.to_string())
}

/// Append `member` to a mix curve's `curves` list.
pub fn add_mix_member(toml_text: &str, name: &str, member: &str) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let curve = curve_of_kind(&mut doc, name, "mix")?;
    let members = curve
        .entry("curves")
        .or_insert(value(Array::new()))
        .as_array_mut()
        .ok_or_else(|| CurveEditError::CurveNotATable(name.to_string()))?;
    if members.iter().any(|v| v.as_str() == Some(member)) {
        return Err(CurveEditError::DuplicateMember {
            curve: name.to_string(),
            member: member.to_string(),
        });
    }
    members.push(member);
    Ok(doc.to_string())
}

/// Remove `member` from a mix curve's `curves` list. Refuses to drop the
/// last member — an empty mix is meaningless, and the daemon rejects it
/// anyway (`EmptyMix`).
pub fn remove_mix_member(
    toml_text: &str,
    name: &str,
    member: &str,
) -> Result<String, CurveEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let curve = curve_of_kind(&mut doc, name, "mix")?;
    let members = curve
        .get_mut("curves")
        .and_then(Item::as_array_mut)
        .ok_or_else(|| CurveEditError::UnknownMember {
            curve: name.to_string(),
            member: member.to_string(),
        })?;
    if members.len() <= 1 {
        return Err(CurveEditError::CannotRemoveLastMember(name.to_string()));
    }
    let index = members.iter().position(|v| v.as_str() == Some(member));
    match index {
        Some(i) => {
            members.remove(i);
            Ok(doc.to_string())
        }
        None => Err(CurveEditError::UnknownMember {
            curve: name.to_string(),
            member: member.to_string(),
        }),
    }
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

    const TOML: &str = "\
# top comment stays
[curves.cpu]
kind = \"graph\"
sensor = \"cpu\"
# curve comment stays
points = [[40, 80], [70, 200]]

[curves.gpu]
kind = \"graph\"
sensor = \"gpu\"
points = [[35, 70], [80, 255]] # trailing comment stays

# mix comment stays
[curves.case_mix]
kind = \"mix\"
function = \"max\"
curves = [\"cpu\", \"gpu\"]
";

    #[test]
    fn replace_curve_points_keeps_comments() {
        let out = replace_curve_points(TOML, "cpu", &[(30, 70), (60, 140), (85, 255)]).unwrap();
        assert!(out.contains("# top comment stays"));
        assert!(out.contains("# curve comment stays"));
        assert!(out.contains("# trailing comment stays"));
        assert!(out.contains("points = [[30, 70], [60, 140], [85, 255]]"));
        assert!(
            out.contains("[[35, 70], [80, 255]]"),
            "other curve untouched"
        );
    }

    #[test]
    fn replace_curve_points_rejects_missing_curve() {
        assert!(matches!(
            replace_curve_points(TOML, "nope", &[(30, 70)]),
            Err(CurveEditError::UnknownCurve(name)) if name == "nope"
        ));
    }

    #[test]
    fn replace_curve_points_rejects_non_graph() {
        assert!(matches!(
            replace_curve_points(TOML, "case_mix", &[(30, 70)]),
            Err(CurveEditError::WrongKind {
                expected: "graph",
                ..
            })
        ));
    }

    #[test]
    fn create_graph_curve_writes_kind_sensor_points() {
        let out = create_graph_curve(TOML, "extra", "cpu", &[(30, 70), (80, 255)]).unwrap();
        assert!(out.contains("[curves.extra]"));
        assert!(out.contains("kind = \"graph\""));
        assert!(out.contains("sensor = \"cpu\""));
        assert!(out.contains("points = [[30, 70], [80, 255]]"));
        assert!(out.contains("# top comment stays"));
    }

    #[test]
    fn create_graph_curve_rejects_existing_name() {
        assert!(matches!(
            create_graph_curve(TOML, "cpu", "cpu", &[(30, 70)]),
            Err(CurveEditError::DuplicateCurve(name)) if name == "cpu"
        ));
    }

    #[test]
    fn set_graph_sensor_rebinds() {
        let out = set_graph_sensor(TOML, "gpu", "cpu").unwrap();
        assert!(out.contains("[curves.gpu]"));
        assert!(!out.contains("sensor = \"gpu\""));
        assert!(out.contains("# trailing comment stays"));
    }

    #[test]
    fn set_graph_sensor_rejects_mix() {
        assert!(matches!(
            set_graph_sensor(TOML, "case_mix", "cpu"),
            Err(CurveEditError::WrongKind { .. })
        ));
    }

    #[test]
    fn add_mix_member_appends() {
        let out = add_mix_member(TOML, "case_mix", "extra").unwrap();
        assert!(out.contains("curves = [\"cpu\", \"gpu\", \"extra\"]"));
        assert!(out.contains("# mix comment stays"));
    }

    #[test]
    fn add_mix_member_rejects_duplicate() {
        assert!(matches!(
            add_mix_member(TOML, "case_mix", "cpu"),
            Err(CurveEditError::DuplicateMember { member, .. }) if member == "cpu"
        ));
    }

    #[test]
    fn add_mix_member_rejects_graph_curve() {
        assert!(matches!(
            add_mix_member(TOML, "cpu", "gpu"),
            Err(CurveEditError::WrongKind {
                expected: "mix",
                ..
            })
        ));
    }

    #[test]
    fn remove_mix_member_removes() {
        let out = remove_mix_member(TOML, "case_mix", "gpu").unwrap();
        assert!(out.contains("curves = [\"cpu\"]"));
        assert!(
            out.contains("[curves.gpu]"),
            "the member curve itself stays"
        );
    }

    #[test]
    fn remove_mix_member_rejects_last_member() {
        let one = remove_mix_member(TOML, "case_mix", "gpu").unwrap();
        assert!(matches!(
            remove_mix_member(&one, "case_mix", "cpu"),
            Err(CurveEditError::CannotRemoveLastMember(c)) if c == "case_mix"
        ));
    }

    #[test]
    fn remove_mix_member_rejects_unknown_member() {
        assert!(matches!(
            remove_mix_member(TOML, "case_mix", "nope"),
            Err(CurveEditError::UnknownMember { member, .. }) if member == "nope"
        ));
    }

    #[test]
    fn remove_curve_keeps_comments() {
        let out = remove_curve(TOML, "gpu").unwrap();
        assert!(out.contains("# top comment stays"));
        assert!(out.contains("# curve comment stays"));
        assert!(out.contains("points = [[40, 80], [70, 200]]"));
        assert!(!out.contains("[curves.gpu]"));
        assert!(!out.contains("trailing comment stays"));
    }

    #[test]
    fn remove_curve_rejects_unknown_name() {
        assert!(matches!(
            remove_curve(TOML, "nope"),
            Err(CurveEditError::UnknownCurve(name)) if name == "nope"
        ));
    }
}
