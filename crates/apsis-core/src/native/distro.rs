// SPDX-License-Identifier: GPL-3.0-only

//! `sys-distro` in `info.json`: Timeshift's `LinuxDistro.get_dist_info("/").full_name()`
//! (linuxmint/timeshift 24.01.1, `LinuxDistro.vala:46-153`; called at `Main.vala:241`).

use std::fs;
use std::path::Path;

use super::info::vala_strip;

/// `ID RELEASE (CODENAME)`. From `<root>/etc/lsb-release` if it exists, else from
/// `<root>/etc/os-release`. Empty when neither names a distribution.
#[must_use]
pub fn full_name(root: &Path) -> String {
    // An existing lsb-release wins even when it names nothing; one that can't be read counts
    // as empty (`file_read`).
    let read = |file: &str| fs::read_to_string(root.join(file)).unwrap_or_default();
    if root.join("etc/lsb-release").exists() {
        from_lsb_release(&read("etc/lsb-release"))
    } else if root.join("etc/os-release").exists() {
        from_os_release(&read("etc/os-release"))
    } else {
        String::new()
    }
}

/// The name from lsb-release's text: `DISTRIB_*` keys, one leading and one trailing `"` off.
#[must_use]
pub fn from_lsb_release(text: &str) -> String {
    let (mut id, mut release, mut codename) = ("", "", "");
    for (key, value) in pairs(text) {
        let value = value.strip_prefix('"').unwrap_or(value);
        let value = value.strip_suffix('"').unwrap_or(value);
        match key {
            "DISTRIB_ID" => id = value,
            "DISTRIB_RELEASE" => release = value,
            "DISTRIB_CODENAME" => codename = value,
            _ => {}
        }
    }
    name(id, release, codename)
}

/// The name from os-release's text: `ID` and `VERSION_ID`, quotes kept, no codename. 24.01.1
/// reads it this way; 26.09.0 strips the quotes and reads `VERSION_CODENAME` too.
#[must_use]
pub fn from_os_release(text: &str) -> String {
    let (mut id, mut release) = ("", "");
    for (key, value) in pairs(text) {
        match key {
            "ID" => id = value,
            "VERSION_ID" => release = value,
            _ => {}
        }
    }
    name(id, release, "")
}

/// Lines with exactly one `=`, as stripped (key, value).
fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.split('\n').filter_map(|line| {
        let mut parts = line.split('=');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(key), Some(value), None) => Some((vala_strip(key), vala_strip(value))),
            _ => None,
        }
    })
}

/// `full_name` (`LinuxDistro.vala:46-58`).
fn name(id: &str, release: &str, codename: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    let mut name = id.to_owned();
    if !release.is_empty() {
        name.push(' ');
        name.push_str(release);
    }
    if !codename.is_empty() {
        name.push_str(" (");
        name.push_str(codename);
        name.push(')');
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsb_release() {
        let text = "DISTRIB_ID=Pop\nDISTRIB_RELEASE=24.04\nDISTRIB_CODENAME=noble\n\
            DISTRIB_DESCRIPTION=\"Pop!_OS 24.04 LTS\"\n";
        assert_eq!(from_lsb_release(text), "Pop 24.04 (noble)");
        // os-release keys don't count in lsb-release.
        assert_eq!(from_lsb_release("ID=pop\nVERSION_ID=24.04\n"), "");
    }

    #[test]
    fn os_release_keeps_quotes_and_has_no_codename() {
        let text = "NAME=\"Fedora Linux\"\nVERSION_ID=\"42\"\nID=fedora\n\
            VERSION_CODENAME=\"\"\nFOO=a=b\n";
        assert_eq!(from_os_release(text), "fedora \"42\"");
    }

    #[test]
    fn an_existing_lsb_release_wins_even_empty() {
        let root = std::env::temp_dir().join(format!("apsis-distro-{}", std::process::id()));
        fs::create_dir_all(root.join("etc")).unwrap();
        fs::write(
            root.join("etc/os-release"),
            "ID=pop\nVERSION_ID=\"24.04\"\n",
        )
        .unwrap();
        assert_eq!(full_name(&root), "pop \"24.04\"");
        fs::write(root.join("etc/lsb-release"), "").unwrap();
        let name = full_name(&root);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(name, "");
        assert_eq!(full_name(Path::new("/nonexistent-apsis-root")), "");
    }
}
