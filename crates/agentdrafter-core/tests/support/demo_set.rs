//! Copies the demo plant's drawing set for a test. The core's tests and the
//! command line's both include this file, so they copy the same files and
//! no list of them is kept anywhere.

use std::fs;
use std::path::Path;

/// Copies every YAML file of `examples/demo-set` into `to`, keeping its
/// folders. DWGs a local build left in the example are not copied.
pub fn copy_demo_set(to: &Path) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-set");
    copy_yaml(&from, to);
}

fn copy_yaml(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_yaml(&path, &target);
        } else if path.extension().is_some_and(|e| e == "yaml") {
            fs::create_dir_all(to).unwrap();
            fs::copy(&path, &target).unwrap();
        }
    }
}
