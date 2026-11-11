//! An object's ID and name in a DWG, in AgentDrafter's extended data.
//!
//! Part of the seam ([`super`]): only this module and its siblings name
//! acadrust. AgentDrafter's extended data on an object is one record under
//! its registered application, [`APPLICATION`], holding pairs of strings,
//! a key and its value. Every object a build writes on a sheet carries its
//! ID as two pairs, `sheet` and the sheet's name, then `id` and the ID the
//! object carries in its sheet description or template: a DWG's extended
//! data holds a string of at most 255 characters, and a sheet's name and an
//! `id` may each be nearly that long. The object's ID is the two joined by
//! `/`, such as `DEMO-001/border`. An object with a name carries `name` and
//! its name after them. AutoCAD carries extended data through a save.

use acadrust::entities::EntityCommon;
use acadrust::xdata::{ExtendedData, ExtendedDataRecord, XDataValue};

use super::block::APPLICATION;
use crate::description::ids::ID_SEPARATOR;

/// The key of the name of an object's sheet in its extended data.
const SHEET_KEY: &str = "sheet";

/// The key of the ID an object carries in its file, in its extended data.
const ID_KEY: &str = "id";

/// The key of an object's name in its extended data.
pub(super) const NAME_KEY: &str = "name";

/// Writes an object's ID, the name of its `sheet` and the `id` it carries
/// in its file, and its `name`, if it has one, as AgentDrafter's record in
/// the object's extended data, in place of any record of AgentDrafter's it
/// holds.
pub(super) fn write_id_and_name(
    common: &mut EntityCommon,
    sheet: &str,
    id: &str,
    name: Option<&str>,
) {
    let mut record = ExtendedDataRecord::new(APPLICATION);
    let pairs = [
        (SHEET_KEY, Some(sheet)),
        (ID_KEY, Some(id)),
        (NAME_KEY, name),
    ];
    for (key, value) in pairs {
        if let Some(value) = value {
            record.add_value(XDataValue::String(key.to_owned()));
            record.add_value(XDataValue::String(value.to_owned()));
        }
    }
    common.extended_data.upsert_record(record);
}

/// The object's ID, from AgentDrafter's extended data on it: its sheet's
/// name and its own ID, joined by `/`.
pub(super) fn read_id(common: &EntityCommon) -> Option<String> {
    let data = &common.extended_data;
    let own = value_of(data, ID_KEY)?;
    match value_of(data, SHEET_KEY) {
        Some(sheet) => Some(format!("{sheet}{ID_SEPARATOR}{own}")),
        None => Some(own),
    }
}

/// The object's name, from AgentDrafter's extended data on it.
pub(super) fn read_name(common: &EntityCommon) -> Option<String> {
    value_of(&common.extended_data, NAME_KEY)
}

/// The value under `key` in AgentDrafter's record of `data`, which holds
/// pairs of strings, a key and its value.
fn value_of(data: &ExtendedData, key: &str) -> Option<String> {
    let record = data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case(APPLICATION))?;
    record.values.chunks(2).find_map(|pair| match pair {
        [XDataValue::String(k), XDataValue::String(value)] if k == key => Some(value.clone()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use acadrust::entities::{EntityType, Line};
    use acadrust::types::Vector3;

    use super::*;

    fn strings(values: &[XDataValue]) -> Vec<&str> {
        values
            .iter()
            .map(|v| match v {
                XDataValue::String(s) => s.as_str(),
                other => panic!("a string, not {other:?}"),
            })
            .collect()
    }

    fn line() -> EntityType {
        EntityType::Line(Line::from_points(Vector3::ZERO, Vector3::UNIT_X))
    }

    #[test]
    fn an_id_and_a_name_read_back_in_one_record() {
        let mut line = line();
        assert_eq!(read_id(line.common()), None);
        let mut older = ExtendedDataRecord::new(APPLICATION);
        older.add_value(XDataValue::String("name".to_owned()));
        older.add_value(XDataValue::String("old".to_owned()));
        line.common_mut().extended_data.add_record(older);

        write_id_and_name(line.common_mut(), "DEMO-001", "objects[0]", Some("frame"));

        let common = line.common();
        assert_eq!(read_id(common).as_deref(), Some("DEMO-001/objects[0]"));
        assert_eq!(read_name(common).as_deref(), Some("frame"));
        let records = common.extended_data.records();
        assert_eq!(
            records.len(),
            1,
            "one record under AgentDrafter's application"
        );
        assert_eq!(
            strings(&records[0].values),
            ["sheet", "DEMO-001", "id", "objects[0]", "name", "frame"]
        );

        let mut unnamed = self::line();
        write_id_and_name(unnamed.common_mut(), "DEMO-001", "border", None);
        assert_eq!(
            strings(&unnamed.common().extended_data.records()[0].values),
            ["sheet", "DEMO-001", "id", "border"]
        );
        assert_eq!(read_name(unnamed.common()), None);
    }

    #[test]
    fn a_sheet_name_and_an_id_each_as_long_as_allowed_are_kept_apart() {
        let sheet = "S".repeat(251);
        let own = "i".repeat(255);
        let mut line = line();
        write_id_and_name(line.common_mut(), &sheet, &own, None);
        let values = strings(&line.common().extended_data.records()[0].values)
            .into_iter()
            .map(str::len)
            .collect::<Vec<_>>();
        assert_eq!(values, [5, 251, 2, 255]);
        assert_eq!(read_id(line.common()), Some(format!("{sheet}/{own}")));
    }

    /// `CONTEXT.md` says drawing and object. acadrust's own names, which
    /// say document and entity, are allowed; ours are not.
    #[test]
    fn our_names_here_say_drawing_and_object() {
        let source = include_str!("ids.rs");
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        let ours = code
            .replace("EntityCommon", "")
            .replace("acadrust::entities", "");
        for word in ["document", "entit"] {
            let found: Vec<&str> = ours
                .lines()
                .filter(|line| line.to_lowercase().contains(word))
                .collect();
            assert!(found.is_empty(), "`{word}` in our own names: {found:#?}");
        }
    }
}
