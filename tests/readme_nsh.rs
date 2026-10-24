//! The README documents the CLI as it is (DOCS-1, d-docs): every `rayx …` command line it shows
//! names commands and flags the CLI has, every command and flag of the CLI is documented, the nsh
//! sections say how the pin is chosen and installed, and the pins tables of the README and
//! `docs/projects.md` equal the built-in defaults, with every documented project key read.

use std::path::{Path, PathBuf};

use clap::CommandFactory;
use rayx_cli::cli::Cli;
use rayx_cli::project::pins::{DEFAULTS, PinSource, Pins};
use rayx_cli::project::{Discovery, Project};
use serde_json::json;

fn read(relative: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|_| panic!("{relative}"))
}

/// The text of the section that starts at the heading containing `title`, up to the next heading
/// of the same or a higher level.
fn section(text: &str, title: &str) -> String {
    let mut lines = text
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
    body
}

/// The joined text, so a phrase may wrap anywhere.
fn joined(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every inline code span of `text` (wrapped lines joined) that is a `rayx` command line.
fn command_lines(text: &str) -> Vec<String> {
    let text = joined(text);
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("rayx ") || *span == "rayx")
        .map(str::to_string)
        .collect()
}

/// The nsh commands `rayx app` lists in its help.
fn nsh_commands() -> Vec<String> {
    let cli = Cli::command();
    let app = cli.find_subcommand("app").expect("app");
    let help = app.get_after_help().expect("after help").to_string();
    let list = help
        .split("nsh commands: ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .expect("the nsh command list");
    list.split(", ").map(str::to_string).collect()
}

fn flags_of(command: &clap::Command) -> Vec<String> {
    command
        .get_arguments()
        .flat_map(|arg| {
            let mut names = Vec::new();
            if let Some(long) = arg.get_long() {
                names.push(format!("--{long}"));
            }
            if let Some(short) = arg.get_short() {
                names.push(format!("-{short}"));
            }
            names
        })
        .chain(["--help".to_string(), "-h".to_string()])
        .collect()
}

/// Checks one documented command line against the CLI; returns what is wrong.
fn check_line(line: &str, nsh: &[String]) -> Option<String> {
    let cli = Cli::command();
    let mut command = &cli;
    let mut path = vec!["rayx".to_string()];
    let words: Vec<&str> = line.split_whitespace().skip(1).collect();
    let mut index = 0;
    // Subcommands first.
    while let Some(word) = words.get(index) {
        let word = word.trim_matches(|c| c == '[' || c == ']');
        if word.starts_with('-') || word.starts_with('<') {
            break;
        }
        match command.find_subcommand(word) {
            Some(sub) => {
                command = sub;
                path.push(word.to_string());
                index += 1;
            }
            None if command.get_subcommands().next().is_some() => {
                return Some(format!(
                    "`{line}`: no command `{word}` under `{}`",
                    path.join(" ")
                ));
            }
            None => break,
        }
    }
    let flags = flags_of(command);
    // A command that passes its arguments on (`rayx fmt [--check]`) takes any flag.
    if command
        .get_arguments()
        .any(|arg| arg.is_positional() && arg.is_allow_hyphen_values_set())
    {
        return None;
    }
    let app = path == ["rayx", "app"];
    for word in &words[index..] {
        let word = word.trim_matches(|c| c == '[' || c == ']' || c == ',' || c == '.');
        if word == "--" {
            break;
        }
        if app && !word.starts_with('-') {
            // The nsh command: everything after it is nsh's.
            if word.starts_with('<') || nsh.iter().any(|known| known == word) {
                break;
            }
            return Some(format!(
                "`{line}`: `{word}` is not an nsh command rayx lists"
            ));
        }
        for flag in word.split('|').filter(|part| part.starts_with('-')) {
            let flag = flag.split('=').next().unwrap_or(flag);
            if !flags.iter().any(|known| known == flag) {
                return Some(format!(
                    "`{line}`: `{}` has no flag `{flag}`",
                    path.join(" ")
                ));
            }
        }
    }
    None
}

#[test]
fn every_documented_command_line_names_commands_and_flags_the_cli_has() {
    let readme = read("README.md");
    let nsh = nsh_commands();
    let lines = command_lines(&readme);
    assert!(
        lines.len() > 40,
        "the README shows the command lines: {lines:?}"
    );
    let wrong: Vec<String> = lines
        .iter()
        .filter_map(|line| check_line(line, &nsh))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_wrong_flag_or_command_in_the_readme_would_be_caught() {
    let nsh = nsh_commands();
    assert!(check_line("rayx setup --gpu", &nsh).is_some());
    assert!(check_line("rayx nsh remove", &nsh).is_some());
    assert!(check_line("rayx app pack --target web", &nsh).is_some());
    assert!(check_line("rayx app -C <dir> run --target web --port 3000", &nsh).is_none());
    assert!(check_line("rayx setup --lima|--wsl --release 1.0.1", &nsh).is_none());
    assert!(check_line("rayx setup --lima|--vsl", &nsh).is_some());
}

#[test]
fn every_command_and_flag_of_the_cli_is_documented() {
    let readme = read("README.md");
    let text = joined(&readme);
    fn walk(command: &clap::Command, path: &[String], out: &mut Vec<(String, Vec<String>)>) {
        let flags = command
            .get_arguments()
            .filter(|arg| !arg.is_positional())
            .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
            .filter_map(|arg| {
                arg.get_long()
                    .map(|long| format!("--{long}"))
                    .or_else(|| arg.get_short().map(|short| format!("-{short}")))
            })
            .collect();
        out.push((path.join(" "), flags));
        for sub in command.get_subcommands() {
            let mut next = path.to_vec();
            next.push(sub.get_name().to_string());
            walk(sub, &next, out);
        }
    }
    let mut commands = Vec::new();
    walk(&Cli::command(), &["rayx".to_string()], &mut commands);
    let mut missing = Vec::new();
    for (command, flags) in &commands {
        if !text.contains(&format!("`{command}")) {
            missing.push(command.clone());
        }
        for flag in flags {
            let shown = ["`", " ", "["]
                .iter()
                .any(|before| text.contains(&format!("{before}{flag}")));
            if !shown {
                missing.push(format!("{command} {flag}"));
            }
        }
    }
    assert!(missing.is_empty(), "undocumented: {missing:?}");
    for command in commands
        .iter()
        .map(|(c, _)| c)
        .filter(|c| c.matches(' ').count() == 1)
    {
        assert!(
            // `rayx self` is listed as `rayx self update`, its only command.
            readme.contains(&format!("| `{command}` |"))
                || readme.contains(&format!("| `{command} ")),
            "the commands table lists {command}"
        );
    }
}

#[test]
fn the_readme_says_how_rayx_chooses_and_installs_nsh() {
    let readme = read("README.md");
    let nsh = joined(&section(&readme, "How `rayx` chooses and installs nsh"));
    for needle in [
        "never runs the `nsh` on your `PATH`",
        "`Cargo.lock`",
        "`rayx.toml`",
        "`[nsh]`",
        "`fork-0.2.0-4` at `327bb9e`",
        "`$RAYX_HOME/nsh/<id>` (`~/.rayx/nsh`)",
        "`327bb9ea65d0`",
        "`v0.2.0`",
        "`nsh-<platform>.tar.gz`",
        "`.sha256`",
        "`cargo build --release --locked -p nsh-cli --target-dir target`",
        "`receipt.json`",
        "waits for the first",
        "`rayx nsh prune [--yes]`",
        "`RAYX_NSH=<path>`",
        "`rayx: RAYX_NSH=<path> replaces the project's nsh 327bb9ea65d0`",
        "A path dependency on an NSH checkout",
        "`cargo build --release -p nsh-cli --target-dir target`",
        "copying nothing into the store",
    ] {
        assert!(
            nsh.contains(needle),
            "the nsh section says {needle}:\n{nsh}"
        );
    }
    let old = joined(&section(&readme, "A new CLI, and the old one"));
    for needle in [
        "`rayx-ui/RayX.CLI.Old`",
        "GPUI apps stay on the old CLI",
        "The installer replaces the old binary",
        "Records and guests carry over",
        "`setup.json` and `guests.json`",
    ] {
        assert!(
            old.contains(needle),
            "the old CLI section says {needle}:\n{old}"
        );
    }
    let app = joined(&section(&readme, "`rayx app`"));
    for needle in [
        "`rayx app [-C <dir>] [--no-install] <command> [nsh arguments] [-- app arguments]`",
        "without `CARGO`",
        "`CC_wasm32_unknown_unknown`",
        "`ANDROID_NDK_HOME`",
        "replaces itself with nsh",
        "exits with nsh's code",
    ] {
        assert!(
            app.contains(needle),
            "the app section says {needle}:\n{app}"
        );
    }
}

/// The backticked spans of `text`.
fn spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// What each row of the sets-per-target table claims, set by set, that `required_sets` does not
/// give: `(command, target)` pairs read from the first cell, sets from the second (`--web`, …, or
/// `none`).
fn sets_table_errors(table: &str) -> (Vec<(String, Option<String>)>, Vec<String>) {
    use rayx_cli::app::prereqs::required_sets;
    let mut checked = Vec::new();
    let mut wrong = Vec::new();
    for line in table.lines().filter(|l| l.starts_with('|')).skip(2) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let (first, second) = (cells[0], cells[1]);
        let claimed: Vec<String> = if second.starts_with("none") {
            Vec::new()
        } else {
            // The set flags; the cell may also name nsh's own flags (`--browser`).
            spans(second)
                .iter()
                .filter_map(|span| span.strip_prefix("--"))
                .filter(|name| ["web", "test", "ios", "android"].contains(name))
                .map(str::to_string)
                .collect()
        };
        let mut cases: Vec<(String, Option<String>)> = Vec::new();
        let words = spans(first);
        if first.starts_with("any command with") {
            for command in ["run", "dev", "package"] {
                cases.push((command.to_string(), Some("web".to_string())));
            }
        } else if let Some(target) = words.first().and_then(|w| w.strip_prefix("--target ")) {
            cases.push(("run".to_string(), Some(target.to_string())));
            for other in &words[1..] {
                cases.push(("run".to_string(), Some(other.to_string())));
            }
            if first.contains("no target") {
                cases.push(("run".to_string(), None));
            }
        } else if let Some(test) = words.first().and_then(|w| w.strip_suffix(" --target web")) {
            cases.push((test.to_string(), Some("web".to_string())));
        } else {
            // Commands that need nothing, whatever their target.
            for command in &words {
                cases.push((command.to_string(), Some("web".to_string())));
            }
        }
        for (command, target) in cases {
            let wants: Vec<String> = required_sets(&command, target.as_deref())
                .iter()
                .map(|set| set.name().to_string())
                .collect();
            checked.push((command.clone(), target.clone()));
            if wants != claimed {
                wrong.push(format!(
                    "{command} {target:?}: the table says {claimed:?}, the map {wants:?}"
                ));
            }
        }
    }
    (checked, wrong)
}

#[test]
fn the_sets_per_target_table_equals_the_prerequisite_map() {
    let readme = read("README.md");
    let (checked, wrong) = sets_table_errors(&section(&readme, "Sets per target"));
    assert!(wrong.is_empty(), "{wrong:#?}");
    // Every target and every command that needs nothing has its row.
    let case =
        |command: &str, target: Option<&str>| (command.to_string(), target.map(str::to_string));
    let mut expected = vec![case("test", Some("web"))];
    for command in ["run", "dev", "package"] {
        expected.push(case(command, Some("web")));
    }
    for target in [
        "ios",
        "ios-simulator",
        "catalyst",
        "android",
        "macos",
        "linux",
        "windows",
        "gpui",
    ] {
        expected.push(case("run", Some(target)));
    }
    expected.push(case("run", None));
    for command in [
        "build",
        "check",
        "init",
        "keygen",
        "certify",
        "publish",
        "verify",
        "revoke",
        "symbolicate",
    ] {
        expected.push(case(command, Some("web")));
    }
    let mut found = checked.clone();
    found.sort();
    expected.sort();
    assert_eq!(found, expected, "the rows cover exactly these cases");
    // A dropped row is caught.
    let dropped: String = section(&readme, "Sets per target")
        .lines()
        .filter(|line| !line.starts_with("| `--target android`"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(sets_table_errors(&dropped).0.len(), checked.len() - 1);
    // A wrong set in a row is caught.
    let table = section(&readme, "Sets per target").replace(
        "| `--target ios`, `ios-simulator` or `catalyst` | `--ios` |",
        "| `--target ios`, `ios-simulator` or `catalyst` | `--web` |",
    );
    assert_eq!(sets_table_errors(&table).1.len(), 3);
    let table = section(&readme, "Sets per target")
        .replace("or no target | none |", "or no target | `--test` |");
    assert_eq!(sets_table_errors(&table).1.len(), 5);
}

/// Every flag that a flag table in a section about `rayx <command>` lists (its first cell, as in
/// `| \`--web\` |`) and that command does not have.
fn table_flag_errors(readme: &str) -> (usize, Vec<String>) {
    let cli = Cli::command();
    let mut command: Option<&clap::Command> = None;
    let mut checked = 0;
    let mut wrong = Vec::new();
    for line in readme.lines() {
        if line.starts_with('#') {
            command = spans(line)
                .first()
                .filter(|span| span.starts_with("rayx "))
                .and_then(|span| {
                    span.split_whitespace()
                        .skip(1)
                        .take_while(|word| !word.starts_with('-'))
                        .try_fold(&cli, |command, word| command.find_subcommand(word))
                });
            continue;
        }
        let (Some(command), Some(rest)) = (command, line.strip_prefix("| `")) else {
            continue;
        };
        let cell = rest.split('`').next().unwrap_or_default();
        if !cell.starts_with('-') {
            continue;
        }
        let flags = flags_of(command);
        for flag in cell.split([' ', '|']).filter(|word| word.starts_with('-')) {
            checked += 1;
            if !flags.iter().any(|known| known == flag) {
                wrong.push(format!("{} has no {flag}", command.get_name()));
            }
        }
    }
    (checked, wrong)
}

#[test]
fn every_flag_a_readme_table_lists_is_a_flag_of_its_sections_command() {
    let readme = read("README.md");
    let (checked, wrong) = table_flag_errors(&readme);
    assert!(
        checked >= 11,
        "the setup and app tables were read: {checked}"
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
    // A stale or made-up row is caught.
    let stale = readme.replace("| `--wsl` |", "| `--gpu` | The GPU drivers. |\n| `--wsl` |");
    assert_eq!(table_flag_errors(&stale).1, ["setup has no --gpu"]);
    let made_up = readme.replace(
        "| `--no-install` |",
        "| `--quiet` | Prints less. |\n| `--no-install` |",
    );
    assert_eq!(table_flag_errors(&made_up).1, ["app has no --quiet"]);
}

/// The rows of the first Markdown table in `text`: the cells of each row, backticks removed.
fn table_rows(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .skip(2)
        .map(|line| {
            line.trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().replace('`', ""))
                .collect()
        })
        .collect()
}

/// The built-in default of each pin, as the tables name them.
fn defaults() -> Vec<(&'static str, String)> {
    vec![
        (
            "nsh",
            format!("{} at {}", DEFAULTS.nsh.tag, &DEFAULTS.nsh.commit[..7]),
        ),
        ("Rust toolchain", "stable".to_string()),
        ("wasm-bindgen", "none".to_string()),
        ("Node.js", DEFAULTS.node.to_string()),
        ("JDK", DEFAULTS.jdk.to_string()),
        ("Android platform", DEFAULTS.android_platform.to_string()),
        (
            "Android build tools",
            DEFAULTS.android_build_tools.to_string(),
        ),
        ("Android NDK", DEFAULTS.android_ndk.to_string()),
        (
            "Emulator system image",
            format!("<platform>;{}", DEFAULTS.android_image_tag),
        ),
        ("Xcode", "the newest stable".to_string()),
        ("Guest Ubuntu (WSL and Lima)", DEFAULTS.ubuntu.to_string()),
    ]
}

/// The pin of `pins` a table row names, with the project key that sets it.
fn pin_of<'a>(pins: &'a Pins, name: &str) -> Option<&'a rayx_cli::project::pins::Pin> {
    match name {
        "Node.js" => Some(&pins.node),
        "JDK" => Some(&pins.jdk),
        "Android platform" => Some(&pins.android_platform),
        "Android build tools" => Some(&pins.android_build_tools),
        "Android NDK" => Some(&pins.android_ndk),
        "Emulator system image" => Some(&pins.android_system_image),
        "Xcode" => pins.xcode.as_ref(),
        _ => None,
    }
}

/// A project whose `[workspace.metadata.rayx]` sets `key`.
fn project_with(key: &str) -> Project {
    Project {
        app_dir: PathBuf::from("/nowhere"),
        workspace_root: PathBuf::from("/nowhere"),
        packages: Vec::new(),
        rayx_metadata: Some(json!({ key: "documented" })),
        discovery: Discovery::Files,
    }
}

#[test]
fn the_readme_pins_table_equals_the_defaults_and_every_key_is_read() {
    let readme = read("README.md");
    let rows = table_rows(&section(&readme, "Versions and where they come from"));
    let documented: Vec<(String, String)> = rows
        .iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();
    let expected: Vec<(String, String)> = defaults()
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    assert_eq!(documented, expected);
    let mut keys = 0;
    for row in &rows {
        let key = &row[2];
        if key.starts_with("none") {
            assert!(pin_of(&Pins::resolve(None), &row[0]).is_none() || row[0] == "Xcode");
            continue;
        }
        let pins = Pins::resolve(Some(&project_with(key)));
        let pin = pin_of(&pins, &row[0]).unwrap_or_else(|| panic!("{} has a pin", row[0]));
        assert_eq!(
            (pin.value.as_str(), pin.source),
            ("documented", PinSource::Project),
            "{} is read from `{key}`",
            row[0]
        );
        keys += 1;
    }
    assert_eq!(keys, 7, "node, jdk, the four Android keys and xcode");
}

#[test]
fn the_projects_doc_names_the_built_in_nsh_pin() {
    let projects = read("docs/projects.md");
    let rows = table_rows(&section(&projects, "The nsh pin"));
    let cell = |first: &str| -> String {
        rows.iter()
            .find(|row| row[0].starts_with(first))
            .unwrap_or_else(|| panic!("a row for {first}"))[2]
            .clone()
    };
    assert_eq!(
        cell("built in"),
        format!("{} at {}", DEFAULTS.nsh.tag, &DEFAULTS.nsh.commit[..7])
    );
    let lock = cell("Cargo.lock");
    assert!(
        lock.contains(DEFAULTS.nsh.tag) && lock.contains(DEFAULTS.nsh.commit),
        "{lock}"
    );
}

#[test]
fn the_projects_doc_names_the_same_defaults_and_keys() {
    let projects = read("docs/projects.md");
    let rows = table_rows(&section(&projects, "The other pins"));
    let defaults = defaults();
    for row in &rows {
        let default = defaults
            .iter()
            .find(|(name, _)| *name == row[0])
            .unwrap_or_else(|| panic!("{} is a pin", row[0]));
        let value = row[2].split(':').next().unwrap_or_default().trim();
        assert_eq!(value, default.1, "{}", row[0]);
    }
    assert_eq!(rows.len(), 9, "every pin but nsh and the guests' Ubuntu");
    let keys = section(&projects, "The other pins");
    let mut read = 0;
    // Every row with a key, Xcode included (it has no pin without a project).
    for row in rows.iter().filter(|row| {
        pin_of(&Pins::resolve(Some(&project_with("probe"))), &row[0]).is_some() || row[0] == "Xcode"
    }) {
        assert!(
            keys.contains(&format!("{} = ", row[1])),
            "{} in the example",
            row[1]
        );
        // The key is the one `Pins::resolve` reads.
        let pins = Pins::resolve(Some(&project_with(&row[1])));
        let pin = pin_of(&pins, &row[0]).expect("a pin");
        assert_eq!(
            (pin.value.as_str(), pin.source),
            ("documented", PinSource::Project),
            "{} is read from `{}`",
            row[0],
            row[1]
        );
        read += 1;
    }
    assert_eq!(read, 7, "node, jdk, the four Android keys and xcode");
    let table = joined(&section(&projects, "`rayx.toml`"));
    for line in [
        "git = \"https://github.com/dXnSix/nsh\"",
        &format!("tag = \"{}\"", DEFAULTS.nsh.tag),
        &format!("commit = \"{}\"", DEFAULTS.nsh.commit),
    ] {
        assert!(table.contains(line), "{line}");
    }
}
