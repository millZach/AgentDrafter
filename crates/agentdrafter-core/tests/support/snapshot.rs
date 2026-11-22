//! Every file and folder of a drawing set, for tests that check an undo
//! puts each back byte for byte. The core's journal tests and the command
//! line's both include this file, so they compare the same things.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    GUIDES_FOLDER, JOURNAL_FILE, LOCK_FILE, LOOKING_AT_FILE, PROPOSALS_FOLDER, UNDO_STATES_FOLDER,
};

/// Every file and folder of the set, each file with its bytes, but the
/// journal's own: its file, its lock, the undo states and the folder they
/// are in, the proposals beside it, and what the drafter is looking at,
/// which a change set and an undo never write; each guide's record is
/// compared without the time it was given.
pub fn snapshot(set: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let own = set.join(".agentdrafter");
    let journal = set.join(JOURNAL_FILE);
    let lock = set.join(LOCK_FILE);
    let states = set.join(UNDO_STATES_FOLDER);
    let proposals = set.join(PROPOSALS_FOLDER);
    let looking_at = set.join(LOOKING_AT_FILE);
    let guides = set.join(GUIDES_FOLDER);
    let mut found = BTreeMap::new();
    let mut stack = vec![set.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path == journal
                || path == lock
                || path == looking_at
                || path.starts_with(&states)
                || path.starts_with(&proposals)
            {
                continue;
            }
            if path.is_dir() {
                if path != own {
                    found.insert(path.clone(), None);
                }
                stack.push(path);
            } else if path.starts_with(&guides) && path.extension().is_some_and(|e| e == "json") {
                // A guide's record, without the time it was given, its
                // top-level `at`: the one thing two interfaces may record
                // differently.
                let mut record: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert!(record.as_object_mut().unwrap().remove("at").is_some());
                found.insert(path.clone(), Some(record.to_string().into_bytes()));
            } else {
                found.insert(path.clone(), Some(fs::read(&path).unwrap()));
            }
        }
    }
    found
}
