//! Release automation: release-plz configuration and workflow, the version and the changelog.

use std::fs;
use std::path::PathBuf;

use serde_yaml_ng::Value as Yaml;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn release_plz() -> toml::Table {
    read("release-plz.toml")
        .parse()
        .expect("release-plz.toml parses")
}

fn workflow() -> Yaml {
    serde_yaml_ng::from_str(&read(".github/workflows/release-plz.yml"))
        .expect("release-plz.yml parses")
}

fn get<'a>(value: &'a Yaml, key: &str) -> &'a Yaml {
    value
        .get(key)
        .unwrap_or_else(|| panic!("missing key `{key}`"))
}

fn steps(job: &Yaml) -> &Vec<Yaml> {
    get(job, "steps").as_sequence().expect("steps")
}

fn step_using<'a>(job: &'a Yaml, action_prefix: &str) -> &'a Yaml {
    steps(job)
        .iter()
        .find(|step| {
            step.get("uses")
                .and_then(Yaml::as_str)
                .is_some_and(|uses| uses.starts_with(action_prefix))
        })
        .unwrap_or_else(|| panic!("no step uses {action_prefix}"))
}

#[test]
fn release_plz_is_git_only_unpublished_and_leaves_the_github_release_to_dist() {
    let config = release_plz();
    let workspace = config["workspace"].as_table().expect("[workspace]");

    assert_eq!(workspace["git_only"].as_bool(), Some(true));
    assert_eq!(workspace["git_tag_enable"].as_bool(), Some(true));
    assert_eq!(workspace["git_tag_name"].as_str(), Some("v{{ version }}"));
    assert_eq!(workspace["git_release_enable"].as_bool(), Some(false));
    assert_eq!(workspace["changelog_update"].as_bool(), Some(true));
    assert_eq!(workspace["semver_check"].as_bool(), Some(false));
    assert_eq!(workspace["publish"].as_bool(), Some(false));
    assert_eq!(
        workspace["release_commits"].as_str(),
        Some("^(feat|fix|perf)"),
        "only feat, fix and perf commits start a release"
    );

    let packages = config["package"].as_array().expect("[[package]]");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0]["name"].as_str(), Some("rayx-cli"));
}

#[test]
fn changelog_groups_commit_types_and_drops_housekeeping() {
    let config = release_plz();
    let changelog = config["changelog"].as_table().expect("[changelog]");

    let body = changelog["body"].as_str().expect("body");
    assert!(
        body.contains("## [{{ version }}] - {{ timestamp | date(format=\"%Y-%m-%d\") }}"),
        "release headings are `## [X.Y.Z] - YYYY-MM-DD`, the form dist's changelog parser reads"
    );

    let parsers = changelog["commit_parsers"].as_array().expect("parsers");
    let skipped: Vec<&str> = parsers
        .iter()
        .filter(|p| p.get("skip").and_then(toml::Value::as_bool) == Some(true))
        .filter_map(|p| p["message"].as_str())
        .collect();
    assert!(skipped.contains(&"^chore\\(agentx\\)"), "{skipped:?}");
    assert!(skipped.contains(&"^chore\\(release\\)"), "{skipped:?}");

    let groups: Vec<&str> = parsers
        .iter()
        .filter_map(|p| p.get("group").and_then(toml::Value::as_str))
        .collect();
    for group in ["Features", "Fixes", "Documentation"] {
        assert!(groups.contains(&group), "missing group {group}: {groups:?}");
    }
}

#[test]
fn the_tag_form_agrees_with_dist_and_the_guest_installer() {
    let config = release_plz();
    let tag = config["workspace"]["git_tag_name"].as_str().expect("tag");
    assert_eq!(tag, "v{{ version }}");

    // dist's tag trigger accepts `v1.2.3` (the pattern is `**[0-9]+.[0-9]+.[0-9]+*`).
    let release = read(".github/workflows/release.yml");
    assert!(release.contains("- '**[0-9]+.[0-9]+.[0-9]+*'"));

    // The guest installer (WSL and Lima share it) builds its URLs from the same `v<version>` tag.
    assert!(
        read("src/setup/guest.rs").contains("{RELEASES}/v{version}/"),
        "src/setup/guest.rs downloads from releases/download/v<version>/"
    );
}

#[test]
fn workflow_runs_on_main_pushes_in_the_rayx_repository_only() {
    let workflow = workflow();
    let on = get(&workflow, "on");
    let branches = get(get(on, "push"), "branches")
        .as_sequence()
        .expect("push branches");
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].as_str(), Some("main"));
    assert!(on.get("workflow_dispatch").is_some());
    assert!(
        on.get("pull_request").is_none(),
        "ci.yml owns pull requests"
    );

    let jobs = get(&workflow, "jobs");
    for name in ["release", "release-pr"] {
        let job = get(jobs, name);
        assert_eq!(
            get(job, "if").as_str(),
            Some("github.repository == 'rayx-ui/RayX.CLI'"),
            "{name} runs in rayx-ui/RayX.CLI only"
        );
        assert_eq!(
            get(get(job, "concurrency"), "cancel-in-progress").as_bool(),
            Some(false),
            "{name} never cancels a running release"
        );
    }
}

#[test]
fn both_jobs_check_out_full_history_and_use_the_release_plz_token() {
    let workflow = workflow();
    let jobs = get(&workflow, "jobs");
    let token = "${{ secrets.RELEASE_PLZ_TOKEN }}";

    for (name, command) in [("release", "release"), ("release-pr", "release-pr")] {
        let job = get(jobs, name);

        let checkout = step_using(job, "actions/checkout@");
        assert_eq!(get(get(checkout, "with"), "fetch-depth").as_i64(), Some(0));
        assert_eq!(get(get(checkout, "with"), "token").as_str(), Some(token));

        let action = step_using(job, "release-plz/action@v");
        assert_eq!(get(get(action, "with"), "command").as_str(), Some(command));
        assert_eq!(
            get(get(action, "env"), "GITHUB_TOKEN").as_str(),
            Some(token)
        );

        step_using(job, "actions-rust-lang/setup-rust-toolchain@");
    }

    let permissions = get(get(jobs, "release-pr"), "permissions");
    assert_eq!(get(permissions, "pull-requests").as_str(), Some("write"));
}

#[test]
fn release_plz_token_is_the_only_secret_and_actions_are_pinned() {
    let text = read(".github/workflows/release-plz.yml");

    let mut secrets: Vec<&str> = text
        .match_indices("secrets.")
        .map(|(at, _)| {
            let rest = &text[at + "secrets.".len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .collect();
    secrets.sort_unstable();
    secrets.dedup();
    assert_eq!(secrets, ["RELEASE_PLZ_TOKEN"]);

    let workflow = workflow();
    for job in get(&workflow, "jobs").as_mapping().expect("jobs").values() {
        for step in steps(job) {
            let uses = step.get("uses").and_then(Yaml::as_str).expect("uses");
            let (_, version) = uses.split_once('@').expect("pinned action");
            assert!(
                version.starts_with('v') || version.len() == 40,
                "{uses} is pinned to a version or commit"
            );
        }
    }
}

#[test]
fn ci_workflow_stays_pull_request_only() {
    let ci: Yaml = serde_yaml_ng::from_str(&read(".github/workflows/ci.yml")).expect("ci.yml");
    let on = get(&ci, "on");
    assert!(on.get("pull_request").is_some());
    assert!(on.get("push").is_none(), "ci.yml never runs on pushes");
}

#[test]
fn version_is_at_least_one_and_the_changelog_has_its_section() {
    let version = semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("semver");
    assert!(version.major >= 1, "{version} is at least 1.0.0");

    let changelog = read("CHANGELOG.md");
    let heading = format!("## [{version}] - ");
    let line = changelog
        .lines()
        .find(|line| line.starts_with(&heading))
        .unwrap_or_else(|| panic!("CHANGELOG.md has a `{heading}YYYY-MM-DD` section"));
    let date = &line[heading.len()..];
    assert_eq!(date.len(), 10, "{line}");
    assert!(date.chars().enumerate().all(|(i, c)| match i {
        4 | 7 => c == '-',
        _ => c.is_ascii_digit(),
    }));
}

#[test]
fn first_release_names_the_commands_and_targets() {
    let changelog = read("CHANGELOG.md");
    let section = changelog
        .split("\n## [")
        .find(|s| s.starts_with("1.0.0]"))
        .expect("1.0.0 section");
    for needle in [
        "rayx setup",
        "rayx doctor",
        "rayx wsl",
        "rayx self update",
        "rayx app",
        "rayx fmt",
        "ARM64",
        "installers",
    ] {
        assert!(section.contains(needle), "1.0.0 section mentions {needle}");
    }
}
