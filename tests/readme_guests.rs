//! The README documents every guest command and flag, its links resolve, and the `--help` of
//! each new flag and subcommand says what it does.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;

fn rayx() -> Command {
    cargo_bin_cmd!("rayx")
}

fn readme() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).expect("README")
}

fn help(args: &[&str]) -> String {
    let output = rayx()
        .args(args)
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

/// The GitHub anchor of a heading.
fn slug(heading: &str) -> String {
    heading
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The text of the README section that starts at the heading containing `title`.
fn section(readme: &str, title: &str) -> String {
    let mut lines = readme
        .lines()
        .skip_while(|l| !(l.starts_with('#') && l.contains(title)));
    let first = lines.next().unwrap_or_else(|| panic!("no section {title}"));
    let level = first.chars().take_while(|c| *c == '#').count();
    let mut body = String::new();
    for line in lines {
        if line.starts_with('#') && line.chars().take_while(|c| *c == '#').count() <= level {
            break;
        }
        body.push_str(line);
        body.push('\n');
    }
    // Markdown joins wrapped lines, so a phrase may wrap anywhere.
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_readme_documents_lima_setup_with_its_sizes_prompt_flags_and_read_only_home() {
    let readme = readme();
    let lima = section(&readme, "`rayx setup --lima`");
    for needle in [
        "named `rayx`",
        "Ubuntu 24.04",
        "6 CPUs",
        "12 GiB",
        "100 GiB",
        "read-only",
        "`--clone [<dir>]`",
        "Customize? [y/N]",
        "`--vm-cpus <n>`",
        "`--vm-memory <GiB>`",
        "`--vm-disk <GiB>`",
        "`--vm-home-writable`",
        "`limactl edit`",
        "`--all` does not imply",
        "`--check`",
    ] {
        assert!(
            lima.contains(needle),
            "the Lima section says {needle}:\n{lima}"
        );
    }
}

#[test]
fn the_readme_documents_records_update_refresh_migration_and_removal() {
    let readme = readme();
    let records = section(&readme, "Records, `rayx setup --update`");
    for needle in [
        "`setup.json`",
        "`guests.json`",
        "`rayx setup --update [--yes] [--check]`",
        "Update lima guest rayx? [y/N]",
        "`sudo apt-get upgrade`",
        "never on the host",
        "`rayx self update`",
        "`rayx self update --yes`",
        "``run `rayx setup --update` to refresh the setup and guests``",
    ] {
        assert!(
            records.contains(needle),
            "the records section says {needle}:\n{records}"
        );
    }
    let migration = section(&readme, "Ubuntu versions and replacing a guest");
    for needle in [
        "never upgraded in place",
        "rayx-<old version>",
        "kept, retired",
        "Create <new> with Ubuntu 24.04 next to <old>? [y/N]",
    ] {
        assert!(
            migration.contains(needle),
            "the migration section says {needle}:\n{migration}"
        );
    }
    let removal = section(&readme, "`rayx lima` and removing guests");
    for needle in [
        "`rayx lima remove <name> [--yes]`",
        "`limactl delete --force <name>`",
        "`rayx wsl remove <name> [--yes]`",
        "`wsl --unregister <name>`",
        "Remove <name> and everything in it? [y/N]",
        "creates it again",
    ] {
        assert!(
            removal.contains(needle),
            "the removal section says {needle}:\n{removal}"
        );
    }
    let doctor = section(&readme, "`rayx doctor`");
    for needle in [
        "Lima",
        "current, outdated or retired",
        "`guests` array",
        "never starts a VM",
    ] {
        assert!(
            doctor.contains(needle),
            "the doctor section says {needle}:\n{doctor}"
        );
    }
}

#[test]
fn the_commands_and_flag_tables_and_the_pin_table_list_the_new_surface() {
    let readme = readme();
    for needle in [
        "| `rayx lima` |",
        "| `--lima` |",
        "| `--update` |",
        "| Guest Ubuntu (WSL and Lima) | 24.04 |",
    ] {
        assert!(readme.contains(needle), "{needle}");
    }
    assert!(readme.contains("`rayx wsl remove <name> [--yes]`"));
}

#[test]
fn every_internal_link_in_the_readme_points_at_a_heading() {
    let readme = readme();
    let anchors: Vec<String> = readme
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(slug)
        .collect();
    let mut checked = 0;
    for part in readme.split("](#").skip(1) {
        let anchor = part.split(')').next().expect("anchor");
        assert!(
            anchors.iter().any(|a| a == anchor),
            "no heading for #{anchor}; headings: {anchors:?}"
        );
        checked += 1;
    }
    assert!(checked >= 3, "the guest sections link to each other");
}

#[test]
fn the_help_of_setup_describes_every_new_flag() {
    let help = help(&["setup"]);
    for (flag, words) in [
        ("--lima", "Lima VM"),
        ("--update", "recorded"),
        ("--clone", "--wsl or --lima"),
        ("--vm-cpus", "all but two"),
        ("--vm-memory", "half of this Mac"),
        ("--vm-disk", "can only grow"),
        ("--vm-home-writable", "read-only"),
        ("--yes", "guests without asking"),
    ] {
        let line = help
            .lines()
            .find(|l| l.trim_start().starts_with(flag) || l.contains(&format!("      {flag}")))
            .unwrap_or_else(|| panic!("{flag} is documented:\n{help}"));
        assert!(line.contains(words), "{flag} says {words}: {line}");
    }
}

#[test]
fn the_help_of_the_new_subcommands_says_what_they_remove_or_do() {
    let update = help(&["self", "update"]);
    assert!(
        update.contains("--yes") && update.contains("refresh"),
        "{update}"
    );
    let lima = help(&["lima", "remove"]);
    assert!(
        lima.contains("<NAME>") && lima.contains("--yes") && lima.contains("Delete a Lima VM"),
        "{lima}"
    );
    let wsl = help(&["wsl", "remove"]);
    assert!(wsl.contains("<NAME>") && wsl.contains("--yes"), "{wsl}");
    let top = help(&[]);
    assert!(
        top.lines().any(|l| l.trim_start().starts_with("lima")),
        "{top}"
    );
}
