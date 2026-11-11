//! Every file and folder of a drawing set, for tests that check an undo
//! puts each back byte for byte. The core's journal tests and the command
//! line's both include this file, so they compare the same things.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{JOURNAL_FILE, UNDO_STATES_FOLDER};

/// Every file and folder of the set, each file with its bytes, but the
/// journal's own: its file, the undo states and the folder they are in.
pub fn snapshot(set: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let own = set.join(".agentdrafter");
    let journal = set.join(JOURNAL_FILE);
    let states = set.join(UNDO_STATES_FOLDER);
    let mut found = BTreeMap::new();
    let mut stack = vec![set.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path == journal || path.starts_with(&states) {
                continue;
            }
            if path.is_dir() {
                if path != own {
                    found.insert(path.clone(), None);
                }
                stack.push(path);
            } else {
                found.insert(path.clone(), Some(fs::read(&path).unwrap()));
            }
        }
    }
    found
}
