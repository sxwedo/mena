
use std::ffi::OsString;
use std::path::Path;

use super::editor_arguments;

#[test]
fn editors_receive_their_native_line_location_syntax() {
    let path = Path::new("/work/config.toml");
    assert_eq!(
        editor_arguments("nvim", &[], path, 42),
        [OsString::from("+42"), OsString::from(path)]
    );
    assert_eq!(
        editor_arguments("code", &[], path, 42),
        [
            OsString::from("--wait"),
            OsString::from("--goto"),
            OsString::from("/work/config.toml:42:1"),
        ]
    );
}
