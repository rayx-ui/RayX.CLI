//! User PATH editing: Windows through an in-memory store, Unix through temporary profile files.

use std::fs;
use std::path::Path;

use rayx_cli::host::path_env::{
    BLOCK_END, BLOCK_START, MemoryPathStore, PathChange, PathValue, UserPath, add_to_profile,
    expand_variables, prepend_to_windows_path, profile_files,
};

const ENTRY: &str = r"C:\Users\dev\.cargo\bin";

#[test]
fn windows_prepend_preserves_reg_expand_sz_and_broadcasts_once() {
    let mut store = MemoryPathStore::new(Some(PathValue::Expandable(
        r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;C:\Tools".into(),
    )));
    let change = prepend_to_windows_path(&mut store, ENTRY, false).expect("edit");
    assert_eq!(change, PathChange::Added);
    assert_eq!(
        store.value(),
        Some(&PathValue::Expandable(format!(
            r"{ENTRY};%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;C:\Tools"
        )))
    );
    assert_eq!(store.broadcasts(), 1);
}

#[test]
fn windows_prepend_keeps_a_plain_value_plain_and_creates_an_expandable_one() {
    let mut plain = MemoryPathStore::new(Some(PathValue::Plain(r"C:\Tools".into())));
    prepend_to_windows_path(&mut plain, ENTRY, false).expect("edit");
    assert!(matches!(plain.value(), Some(PathValue::Plain(_))));

    let mut empty = MemoryPathStore::new(None);
    prepend_to_windows_path(&mut empty, ENTRY, false).expect("edit");
    assert_eq!(empty.value(), Some(&PathValue::Expandable(ENTRY.into())));
}

#[test]
fn windows_prepend_is_idempotent_and_matches_case_slashes_and_variables() {
    let mut store = MemoryPathStore::new(Some(PathValue::Expandable(
        r"C:\Tools;%USERPROFILE%\.cargo\bin\".into(),
    )))
    .with_variable("USERPROFILE", r"C:\Users\dev");
    // The same directory written with another case, slashes and the expanded variable.
    for spelling in [
        r"c:\users\DEV\.cargo\bin",
        r"C:/Users/dev/.cargo/bin",
        r"%USERPROFILE%\.cargo\bin",
    ] {
        let before = store.value().cloned();
        let change = prepend_to_windows_path(&mut store, spelling, false).expect("edit");
        assert_eq!(change, PathChange::AlreadyPresent, "{spelling}");
        assert_eq!(
            store.value().cloned(),
            before,
            "{spelling} left the value alone"
        );
    }
    assert_eq!(store.broadcasts(), 0, "nothing changed, nothing broadcast");
}

#[test]
fn windows_dry_run_changes_nothing() {
    let mut store = MemoryPathStore::new(Some(PathValue::Plain(r"C:\Tools".into())));
    let change = prepend_to_windows_path(&mut store, ENTRY, true).expect("dry run");
    assert_eq!(change, PathChange::Added, "reports what would happen");
    assert_eq!(store.value(), Some(&PathValue::Plain(r"C:\Tools".into())));
    assert_eq!(store.broadcasts(), 0);
}

#[test]
fn the_new_shell_notice_follows_only_a_real_change() {
    assert!(
        PathChange::Added
            .notice()
            .is_some_and(|n| n.contains("new shell"))
    );
    assert!(PathChange::AlreadyPresent.notice().is_none());
}

#[test]
fn variable_expansion_leaves_unknown_and_unbalanced_percents_alone() {
    let lookup = |name: &str| (name == "HOME").then(|| "/h".to_string());
    assert_eq!(expand_variables("%HOME%/bin", lookup), "/h/bin");
    assert_eq!(expand_variables("%NOPE%/bin", lookup), "%NOPE%/bin");
    assert_eq!(expand_variables("50% done", lookup), "50% done");
    assert_eq!(expand_variables("%%", lookup), "%%");
}

#[test]
fn shell_selects_the_login_profiles() {
    let home = Path::new("/home/dev");
    assert_eq!(profile_files("/bin/zsh", home), [home.join(".zprofile")]);
    assert_eq!(
        profile_files("/usr/bin/bash", home),
        [home.join(".profile"), home.join(".bashrc")]
    );
    assert_eq!(profile_files("/bin/sh", home), [home.join(".profile")]);
    assert_eq!(profile_files("", home), [home.join(".profile")]);
}

#[test]
fn profile_gets_one_marked_block_and_keeps_existing_content() {
    let dir = tempfile::tempdir().expect("temp dir");
    let profile = dir.path().join(".profile");
    fs::write(&profile, "export EDITOR=vim\nalias ll='ls -l'").expect("seed profile");

    let change = add_to_profile(&profile, "$HOME/.cargo/bin", false).expect("edit");
    assert_eq!(change, PathChange::Added);
    let text = fs::read_to_string(&profile).expect("read");
    assert_eq!(
        text,
        format!(
            "export EDITOR=vim\nalias ll='ls -l'\n\n{BLOCK_START}\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n{BLOCK_END}\n"
        )
    );

    // A second entry joins the same block instead of adding another.
    add_to_profile(&profile, "$HOME/.local/bin", false).expect("edit");
    let text = fs::read_to_string(&profile).expect("read");
    assert_eq!(text.matches(BLOCK_START).count(), 1);
    assert_eq!(text.matches(BLOCK_END).count(), 1);
    assert!(text.contains("export PATH=\"$HOME/.cargo/bin:$PATH\""));
    assert!(text.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
}

#[test]
fn profile_edit_is_idempotent_byte_for_byte() {
    let dir = tempfile::tempdir().expect("temp dir");
    let profile = dir.path().join(".zprofile");
    add_to_profile(&profile, "$HOME/.cargo/bin", false).expect("first");
    let first = fs::read(&profile).expect("read");
    for _ in 0..3 {
        let change = add_to_profile(&profile, "$HOME/.cargo/bin", false).expect("again");
        assert_eq!(change, PathChange::AlreadyPresent);
    }
    assert_eq!(fs::read(&profile).expect("read"), first);
}

#[test]
fn profile_dry_run_writes_nothing_and_creates_no_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let profile = dir.path().join(".profile");
    let change = add_to_profile(&profile, "$HOME/.cargo/bin", true).expect("dry run");
    assert_eq!(change, PathChange::Added);
    assert!(!profile.exists());
}

#[test]
fn a_damaged_block_is_closed_not_duplicated() {
    let dir = tempfile::tempdir().expect("temp dir");
    let profile = dir.path().join(".profile");
    fs::write(&profile, format!("{BLOCK_START}\nexport A=1\n")).expect("seed");
    add_to_profile(&profile, "$HOME/bin", false).expect("edit");
    let text = fs::read_to_string(&profile).expect("read");
    assert_eq!(text.matches(BLOCK_START).count(), 1);
    assert_eq!(text.matches(BLOCK_END).count(), 1);
    assert!(text.contains("export A=1"));
}

#[test]
fn profile_entries_that_could_break_the_shell_are_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let profile = dir.path().join(".profile");
    for bad in ["/a\"b", "/a`b`", "/a\\b", "/a\nb", "/a/$(rm -rf x)"] {
        let error = add_to_profile(&profile, bad, false).expect_err(bad);
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{bad:?}");
    }
    assert!(!profile.exists());
}

#[test]
fn user_path_writes_home_relative_entries_to_every_profile_of_the_shell() {
    let dir = tempfile::tempdir().expect("temp dir");
    let home = dir.path().to_path_buf();
    let files = profile_files("/bin/bash", &home);
    let mut path = UserPath::Profiles {
        files: files.clone(),
        home: home.clone(),
    };
    let entry = home.join(".cargo").join("bin");
    assert_eq!(path.add(&entry, false).expect("add"), PathChange::Added);
    for file in &files {
        let text = fs::read_to_string(file).expect("profile written");
        assert!(
            text.contains("export PATH=\"$HOME/.cargo/bin:$PATH\""),
            "{text}"
        );
    }
    assert_eq!(
        path.add(&entry, false).expect("again"),
        PathChange::AlreadyPresent
    );
}

#[test]
fn user_path_on_windows_goes_through_the_store() {
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    assert_eq!(
        path.add(Path::new(ENTRY), false).expect("add"),
        PathChange::Added
    );
    assert_eq!(
        path.add(Path::new(ENTRY), false).expect("again"),
        PathChange::AlreadyPresent
    );
}

#[cfg(windows)]
#[test]
fn the_registry_store_reads_the_real_user_path_without_changing_it() {
    use rayx_cli::host::Os;
    use rayx_cli::host::path_env::{PathStore, RegistryPathStore};

    // Read-only: the real store's write path is covered through the in-memory store.
    RegistryPathStore
        .read()
        .expect("HKCU\\Environment is readable");
    assert!(UserPath::system(Os::Windows).is_ok());
}

#[test]
fn a_failed_broadcast_does_not_fail_a_persisted_edit() {
    let mut store = MemoryPathStore::new(None).with_failing_broadcast();
    let change = prepend_to_windows_path(&mut store, ENTRY, false).expect("edit survives");
    assert_eq!(change, PathChange::Added);
    assert_eq!(store.value(), Some(&PathValue::Expandable(ENTRY.into())));
    assert_eq!(store.broadcasts(), 1, "the broadcast was attempted");
    assert!(
        change.notice().is_some(),
        "the user is still told to open a new shell"
    );
}
