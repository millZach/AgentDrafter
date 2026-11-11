//! An object's ID in a DWG, in AgentDrafter's extended data.
//!
//! Part of the seam ([`super`]): only this module and its siblings name
//! acadrust. AgentDrafter's extended data on an object is one record under
//! its registered application, [`APPLICATION`], holding pairs of strings,
//! a key and its value. Every object a build writes on a sheet carries its
//! ID as two pairs, `sheet` and the sheet's name, then `id` and the ID the
//! object carries in its sheet description or template: a DWG's extended
//! data holds a string of at most 255 characters, and a sheet's name and an
//! `id` may each be nearly that long. The object's ID is the two joined by
//! `/`, such as `DEMO-001/border`. A named insert carries `name` and its
//! name after them. AutoCAD carries extended data through a save.

use acadrust::entities::EntityCommon;
use acadrust::xdata::{ExtendedData, ExtendedDataRecord, XDataValue};

use super::block::APPLICATION;
use crate::description::ids::ID_SEPARATOR;

/// The key of the name of an object's sheet in its extended data.
const SHEET_KEY: &str = "sheet";

/// The key of the ID an object carries in its file, in its extended data.
const ID_KEY: &str = "id";

/// Writes `id`, an object's ID, into the object's extended data, first in
/// AgentDrafter's record, ahead of anything the record holds already, such
/// as an insert's name. The ID is split at its first `/` into its sheet's
/// name, which holds none, and the ID the object carries in its file.
pub(super) fn write_id(common: &mut EntityCommon, id: &str) {
    let (sheet, own) = id.split_once(ID_SEPARATOR).unwrap_or(("", id));
    let data = &mut common.extended_data;
    let mut values = Vec::new();
    for (key, value) in [(SHEET_KEY, sheet), (ID_KEY, own)] {
        values.push(XDataValue::String(key.to_owned()));
        values.push(XDataValue::String(value.to_owned()));
    }
    if let Some(ours) = data.remove_record(APPLICATION) {
        values.extend(ours.values);
    }
    let mut record = ExtendedDataRecord::new(APPLICATION);
    for value in values {
        record.add_value(value);
    }
    data.add_record(record);
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

/// The value under `key` in AgentDrafter's record of `data`, which holds
/// pairs of strings, a key and its value.
pub(super) fn value_of(data: &ExtendedData, key: &str) -> Option<String> {
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

    #[test]
    fn an_id_reads_back_and_goes_ahead_of_a_name() {
        let mut line = EntityType::Line(Line::from_points(Vector3::ZERO, Vector3::UNIT_X));
        assert_eq!(read_id(line.common()), None);
        let mut named = ExtendedDataRecord::new(APPLICATION);
        named.add_value(XDataValue::String("name".to_owned()));
        named.add_value(XDataValue::String("jb".to_owned()));
        line.common_mut().extended_data.add_record(named);

        write_id(line.common_mut(), "DEMO-001/objects[0]");

        let common = line.common();
        assert_eq!(read_id(common).as_deref(), Some("DEMO-001/objects[0]"));
        assert_eq!(
            value_of(&common.extended_data, "name").as_deref(),
            Some("jb")
        );
        let records = common.extended_data.records();
        assert_eq!(
            records.len(),
            1,
            "one record under AgentDrafter's application"
        );
        assert_eq!(
            strings(&records[0].values),
            ["sheet", "DEMO-001", "id", "objects[0]", "name", "jb"]
        );
    }

    #[test]
    fn a_sheet_name_and_an_id_each_as_long_as_allowed_are_kept_apart() {
        let sheet = "S".repeat(251);
        let own = "i".repeat(255);
        let mut line = EntityType::Line(Line::from_points(Vector3::ZERO, Vector3::UNIT_X));
        write_id(line.common_mut(), &format!("{sheet}/{own}"));
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
