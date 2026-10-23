//! The 1.x release policy: no commit is marked breaking, a break is a `feat`/`fix` commit with a
//! `Migration:` footer the changelog renders, and releases stay 1.x until a second consumer appears.

use std::path::PathBuf;

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn release_plz() -> toml::Table {
    read("release-plz.toml").parse().expect("release-plz.toml")
}

#[test]
fn semver_checks_stay_off_and_only_feat_fix_perf_start_a_release() {
    let config = release_plz();
    let workspace = config["workspace"].as_table().expect("[workspace]");
    assert_eq!(workspace["semver_check"].as_bool(), Some(false));
    assert_eq!(
        workspace["release_commits"].as_str(),
        Some("^(feat|fix|perf)")
    );
}

#[test]
fn the_changelog_body_prints_migration_footers_under_their_commit() {
    let config = release_plz();
    let body = config["changelog"]["body"].as_str().expect("body");
    let commit_loop = body
        .split("{% for commit in commits %}")
        .nth(1)
        .expect("the commit loop");
    let footers = commit_loop
        .find("{%- for footer in commit.footers")
        .expect("a loop over the commit's footers");
    let message = commit_loop
        .rfind("{{ commit.message }}")
        .expect("the commit line");
    assert!(message < footers, "footers come after the commit line");
    let footer_loop = &commit_loop[footers..];
    assert!(
        footer_loop.contains("footer.token == \"Migration\""),
        "{footer_loop}"
    );
    assert!(
        footer_loop.contains("  Migration: {{ footer.value }}"),
        "an indented `Migration: <value>` line: {footer_loop}"
    );
    assert!(
        footer_loop.find("{%- endfor -%}") < footer_loop.find("{% endfor -%}"),
        "the footer loop closes inside the commit loop: {footer_loop}"
    );
}

#[test]
fn releasing_md_states_the_1x_policy_and_drops_the_breaking_advice() {
    let releasing = read("docs/releasing.md");
    let text = flat(&releasing);
    for needle in [
        "## The 1.x policy",
        "No commit is marked breaking",
        "`BREAKING CHANGE:` footer",
        "`Migration:` footer",
        "releases stay 1.x",
        "proposes `2.0.0` means a stray marker",
        "reword the commit",
        "--force-with-lease",
        "The policy ends when a second consumer appears",
    ] {
        assert!(text.contains(needle), "docs/releasing.md lacks `{needle}`");
    }
    assert!(
        !text.contains("| major (`1.0.0` → `2.0.0`)"),
        "the commit table has no major row"
    );
    assert!(
        !text.contains("Write a breaking change as `feat!`"),
        "the feat! advice is gone"
    );
    assert!(!read("release-plz.toml").contains("A breaking change is written `feat!`"));
}

#[test]
fn claude_and_agents_state_the_policy_in_a_releases_paragraph() {
    assert_eq!(
        read("CLAUDE.md").trim(),
        "@AGENTS.md",
        "CLAUDE.md includes AGENTS.md"
    );
    let agents = read("AGENTS.md");
    let releases = agents
        .split("\n## Releases\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .map(flat)
        .expect("a Releases section");
    for needle in [
        "stays on 1.x",
        "no `!`",
        "no `BREAKING CHANGE:` footer",
        "`Migration:` footer",
        "The first release is 1.0.0",
        "only consumer",
        "owner's go-ahead",
    ] {
        assert!(releases.contains(needle), "Releases lacks `{needle}`");
    }
}

#[test]
fn release_plz_states_the_policy_and_names_rayx_in_the_changelog_header() {
    let text = read("release-plz.toml");
    let comment = flat(
        &text
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix('#'))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    for needle in [
        "rayx stays on 1.x",
        "`Migration:` footer",
        "never `!` or `BREAKING CHANGE:`",
        "The first release is 1.0.0",
    ] {
        assert!(
            comment.contains(needle),
            "release-plz.toml's comment lacks `{needle}`"
        );
    }
    let header = release_plz()["changelog"]["header"]
        .as_str()
        .expect("changelog header")
        .to_string();
    assert!(header.starts_with("# Changelog"), "{header}");
    assert!(header.contains("changes to `rayx`"), "{header}");
    assert!(
        read("CHANGELOG.md").starts_with(header.trim_end()),
        "CHANGELOG.md starts with the configured header"
    );
}

/// The text of the `## <title>` section of `docs/releasing.md`, flattened.
fn releasing_section(title: &str) -> String {
    let releasing = read("docs/releasing.md");
    let heading = format!("\n## {title}\n");
    let section = releasing
        .split_once(&heading)
        .map(|(_, rest)| rest.split("\n## ").next().unwrap_or(rest))
        .unwrap_or_else(|| panic!("docs/releasing.md has a `## {title}` section"));
    flat(section)
}

#[test]
fn releasing_md_covers_the_first_release() {
    let section = releasing_section("The first release (1.0.0)");
    for needle in [
        "first release is `v1.0.0`",
        "without a release pull request",
        "hand-written `## [1.0.0] - YYYY-MM-DD` section",
        "Move `RayX.CLI.Old`'s README install links",
        "Create `rayx-ui/RayX.CLI` (public)",
        "`RELEASE_PLZ_TOKEN`",
        "Set the date of the `1.0.0` section",
        "sees 1.0.0 as older than 1.4.1",
        "keeps `setup.json` and `guests.json`",
    ] {
        assert!(
            section.contains(needle),
            "the first-release section lacks `{needle}`"
        );
    }
}

#[test]
fn releasing_md_covers_the_token_the_fallback_and_checking_a_release() {
    let token = releasing_section("The `RELEASE_PLZ_TOKEN` secret");
    for needle in [
        "fine-grained personal access token",
        "only `RayX.CLI`",
        "**Contents** read and write",
        "**Pull requests** read and write",
        "Rotate before the expiry date",
        "`rayx` itself never reads or stores it",
    ] {
        assert!(token.contains(needle), "the token section lacks `{needle}`");
    }
    let fallback = releasing_section("By-hand fallback");
    for needle in [
        "git tag vX.Y.Z",
        "git push origin vX.Y.Z",
        "`## [X.Y.Z] - YYYY-MM-DD`",
    ] {
        assert!(
            fallback.contains(needle),
            "the fallback section lacks `{needle}`"
        );
    }
    let checking = releasing_section("Checking a release");
    for needle in [
        "gh release view vX.Y.Z --repo rayx-ui/RayX.CLI",
        "gh attestation verify",
        "`sha256.sum`",
    ] {
        assert!(
            checking.contains(needle),
            "the checking section lacks `{needle}`"
        );
    }
}
