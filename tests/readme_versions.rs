//! The README documents choosing a release (`self update --release`, `setup --release`), named VMs
//! (`--vm`) and the update target, and the `--help` of every new flag says what it does.

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
    let text = String::from_utf8(output.stdout).expect("help is UTF-8");
    // The help wraps lines; a phrase may wrap anywhere.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// The text of the README section that starts at the heading containing `title`, wrapped lines
/// joined.
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
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_says(text: &str, what: &str, needles: &[&str]) {
    for needle in needles {
        assert!(text.contains(needle), "{what} says {needle}:\n{text}");
    }
}

#[test]
fn the_readme_documents_choosing_a_release_in_both_directions() {
    let readme = readme();
    let releases = section(&readme, "Choosing a release and going back");
    assert_says(
        &releases,
        "the release section",
        &[
            "`rayx self update --release 1.0.1 [--yes] [--check]`",
            "`--release`, not `--version`",
            "the install receipt",
            "release X was not found",
            "`upgrade`, `downgrade` or `same`",
            "`rayx setup --lima|--wsl --release 1.0.1`",
            "`rayx setup --update --release 1.0.1`",
            "that tag's own installer",
            "nothing changes if it is missing",
            "`RAYX_LIMA_BINARY` or `RAYX_WSL_BINARY`",
            "Install rayx 1.0.1 over 1.2.0, an older release? [y/N]",
            "Install rayx 1.0.1 in <guest> over 1.2.0, an older release? [y/N]",
            "`--yes` skips the question",
            "no terminal and no `--yes` refuses",
            "Nothing is remembered",
            "a later plain `rayx self update` installs the latest release",
            "brings the guests back to the running version",
            "`1.0.0` is the oldest release that can be named",
            "`1.1.0`",
            "predates the guest refresh",
        ],
    );
}

#[test]
fn the_readme_documents_named_vms_and_the_update_target() {
    let readme = readme();
    let lima = section(&readme, "`rayx setup --lima`");
    assert_says(
        &lima,
        "the Lima section",
        &[
            "`--vm <name>`",
            "one VM per project repository",
            "`rayx setup --lima --vm app`",
            "default `rayx`",
            "1 to 40 lowercase letters, digits and hyphens",
            "not `default`",
            "own record",
            "`app-<old version>`",
            "never adopted",
            "`rayx` included",
            "`limactl`",
            "choose another `--vm` name",
            "while `guests.json` cannot be read",
        ],
    );
    let update = section(&readme, "Records, `rayx setup --update`");
    assert_says(
        &update,
        "the update section",
        &[
            "`rayx setup --update --vm <name>`",
            "repeat the flag",
            "Lima VMs and WSL distributions alike",
            "skips the host part",
            "recorded names, before any command runs",
            "`--check` then inspects only the named running guests",
        ],
    );
    let migration = section(&readme, "Ubuntu versions and replacing a guest");
    assert_says(
        &migration,
        "the migration section",
        &["`<name>-<old version>` for a VM chosen with `--vm`"],
    );
    let removal = section(&readme, "`rayx lima` and removing guests");
    assert_says(
        &removal,
        "the removal section",
        &["`--vm <name>` for a named VM"],
    );
}

#[test]
fn the_commands_table_and_the_install_section_point_at_the_release_choice() {
    let readme = readme();
    let commands = section(&readme, "Commands");
    assert_says(
        &commands,
        "the commands table",
        &["installs the release `--release` names"],
    );
    let install = section(&readme, "Install");
    assert_says(
        &install,
        "the install section",
        &["`rayx self update --release 1.0.1`"],
    );
    let setup = section(&readme, "`rayx setup`");
    assert_says(
        &setup,
        "the setup flag table",
        &["| `--release <X.Y.Z>` |", "| `--vm <name>` |"],
    );
}

#[test]
fn every_link_to_a_new_section_resolves() {
    let readme = readme();
    let anchors: Vec<String> = readme
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(slug)
        .collect();
    let anchor = "choosing-a-release-and-going-back";
    assert!(
        readme.contains(&format!("(#{anchor})")),
        "the README links to #{anchor}"
    );
    assert!(
        anchors.iter().any(|a| a == anchor),
        "#{anchor} is a heading: {anchors:?}"
    );
}

#[test]
fn the_help_of_setup_says_what_release_and_vm_do() {
    let setup = help(&["setup"]);
    assert_says(
        &setup,
        "`rayx setup --help`",
        &[
            "--release <RELEASE>",
            "With --lima, --wsl or --update: put exactly this release",
            "of the Linux rayx in the guest instead of this rayx's version; it is not remembered",
            "--vm <NAME>",
            "With --lima or --update: the Lima VM by name (default `rayx`; one VM per project)",
            "repeat it to refresh only these recorded guests and skip this machine's own update",
            "Set up a Lima VM (`rayx`, or the name --vm gives)",
        ],
    );
}

#[test]
fn the_help_of_self_update_says_what_release_and_yes_do() {
    let update = help(&["self", "update"]);
    assert_says(
        &update,
        "`rayx self update --help`",
        &[
            "--release <RELEASE>",
            "Install exactly this release (X.Y.Z, for example 1.0.1), also an older one after asking; it is not remembered",
            "Skip the question before installing an older release",
            "refresh this machine's setup and guests",
        ],
    );
}
