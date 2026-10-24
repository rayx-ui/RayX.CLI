//! The nsh pin (PIN-1 to PIN-3, design §4.1): the NSH crates' source in the project's
//! `Cargo.lock`, else `rayx.toml`'s `[nsh]` table, else the built-in default.

mod common;

use std::path::{Path, PathBuf};

use common::Workspace;
use rayx_cli::host::{Outcome, Runner};
use rayx_cli::nsh::pin::{
    Origin, Pin, Reference, ReleaseSource, Source, classify, default_pin, is_nsh_crate,
    lock_source, resolve, verify_tag,
};
use rayx_cli::project::pins::DEFAULTS;

const COMMIT: &str = "327bb9ea65d08ffac4af498cc7fbbfa18a1b3623";
const FORK_TAG: &str =
    "git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4#327bb9ea65d08ffac4af498cc7fbbfa18a1b3623";

/// Reads the pin with real cargo queries (`locate-project`, `metadata`); nothing is installed.
fn pin_of(dir: &Path) -> anyhow::Result<Pin> {
    resolve(&mut Runner::print(), dir)
}

/// The fork app with every `nsh-*` source of its lock replaced by `source`.
fn fork_app_with(source: &str) -> Workspace {
    let app = Workspace::fixture("nsh-fork-app");
    let lock = app.read("Cargo.lock").replace(FORK_TAG, source);
    app.write("Cargo.lock", &lock);
    app
}

#[test]
fn a_fork_tag_in_the_lock_is_a_git_pin_with_a_12_digit_id() {
    let app = Workspace::fixture("nsh-fork-app");
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(
        pin.source,
        Source::Git {
            url: "https://github.com/dXnSix/nsh".into(),
            reference: Reference::Tag("fork-0.2.0-4".into()),
            commit: COMMIT.into(),
        }
    );
    assert_eq!(pin.origin, Origin::Project);
    assert_eq!(pin.origin.as_str(), "project");
    assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
    assert_eq!(pin.version.as_deref(), Some("0.2.0"));
    assert_eq!(pin.label(), "fork-0.2.0-4 (327bb9e)");
    assert_eq!(
        pin.release(),
        Some(ReleaseSource {
            repository: "dXnSix/nsh".into(),
            tag: "fork-0.2.0-4".into(),
        })
    );
    assert_eq!(pin.dir, app.root, "the pin is read where nsh will run");
    assert!(pin.app);
    assert_eq!(pin.default_warning(), None, "a pinned app gets no warning");
}

#[test]
fn a_branch_or_rev_source_is_a_git_pin_without_a_release() {
    // Cargo's version 4 lock percent-encodes the ref: `feat/x` is written `feat%2Fx`.
    for (query, reference) in [
        ("branch=feat%2Fx", Reference::Branch("feat/x".into())),
        ("branch=main", Reference::Branch("main".into())),
        ("tag=fork%2D0.2.0-4", Reference::Tag("fork-0.2.0-4".into())),
        ("rev=327bb9e", Reference::Rev("327bb9e".into())),
    ] {
        let app = fork_app_with(&format!(
            "git+https://github.com/dXnSix/nsh?{query}#{COMMIT}"
        ));
        let pin = pin_of(&app.root).expect("pin");
        let Source::Git {
            reference: found, ..
        } = &pin.source
        else {
            panic!("a git pin: {pin:?}");
        };
        assert_eq!(found, &reference);
        assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
        if !query.starts_with("tag=") {
            assert_eq!(
                pin.release(),
                None,
                "{query}: a branch or a rev has no release"
            );
        }
        assert_eq!(pin.git_ref().as_deref(), Some(COMMIT));
    }
}

#[test]
fn another_git_url_is_a_git_pin_with_a_release_only_on_github() {
    let app = fork_app_with(&format!(
        "git+https://gitlab.com/someone/nsh?tag=v1#{COMMIT}"
    ));
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(
        pin.git_url().as_deref(),
        Some("https://gitlab.com/someone/nsh")
    );
    assert_eq!(pin.release(), None);

    let app = fork_app_with(&format!("git+https://github.com/someone/nsh.git#{COMMIT}"));
    let pin = pin_of(&app.root).expect("pin");
    let Source::Git { reference, .. } = &pin.source else {
        panic!("{pin:?}");
    };
    assert_eq!(reference, &Reference::Default);
    assert_eq!(pin.release(), None, "the default branch has no release");
    assert_eq!(pin.label(), "327bb9e");
}

#[test]
fn a_crates_io_source_is_an_upstream_release_pin() {
    let app = fork_app_with("registry+https://github.com/rust-lang/crates.io-index");
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(
        pin.source,
        Source::Release {
            version: "0.2.0".into()
        }
    );
    assert_eq!(pin.id().as_deref(), Some("v0.2.0"));
    assert_eq!(pin.label(), "v0.2.0");
    assert_eq!(
        pin.release(),
        Some(ReleaseSource {
            repository: "longbridge/nsh".into(),
            tag: "v0.2.0".into(),
        })
    );
    assert_eq!(
        pin.git_url().as_deref(),
        Some("https://github.com/longbridge/nsh")
    );
    assert_eq!(pin.git_ref().as_deref(), Some("v0.2.0"));
    assert_eq!(pin.commit(), None);
}

#[test]
fn a_dry_run_writes_no_missing_lock_and_says_the_pin_is_unknown() {
    let app = Workspace::fixture("nsh-path-app");
    assert!(!app.path("Cargo.lock").exists());
    // `--check`, `doctor` and `rayx nsh status` read through a printing runner.
    let error = resolve(&mut Runner::print(), &app.root.join("app"))
        .expect_err("unknown")
        .to_string();
    assert!(
        error.contains("has no Cargo.lock yet, so the nsh it pins is unknown"),
        "{error}"
    );
    assert!(!app.path("Cargo.lock").exists(), "nothing was written");
}

#[test]
fn a_lock_entry_without_a_source_is_a_path_pin_to_the_checkout() {
    let app = Workspace::fixture("nsh-path-app");
    // The fixture has no lock: an executing runner's `cargo metadata` writes it first.
    assert!(!app.path("Cargo.lock").exists());
    let pin = resolve(&mut Runner::execute(), &app.root.join("app")).expect("pin");
    assert!(
        app.path("Cargo.lock").is_file(),
        "the missing lock was written"
    );
    assert_eq!(
        pin.source,
        Source::Path {
            checkout: app.path("nsh")
        }
    );
    assert_eq!(pin.id(), None, "a path checkout has no store entry");
    assert_eq!(pin.release(), None);
    assert_eq!(pin.origin, Origin::Project);
    assert_eq!(pin.dir, app.path("app"));
}

#[test]
fn two_sources_in_one_lock_stop_with_both_named() {
    let app = Workspace::fixture("nsh-fork-app");
    let lock = app.read("Cargo.lock").replacen(
        FORK_TAG,
        &format!("git+https://github.com/dXnSix/nsh?branch=main#{COMMIT}"),
        1,
    );
    app.write("Cargo.lock", &lock);
    let error = pin_of(&app.root).expect_err("two sources").to_string();
    assert!(error.contains("two sources"), "{error}");
    assert!(
        error.contains("nsh-core") && error.contains("nsh-macros"),
        "{error}"
    );
    assert!(
        error.contains("?branch=main") && error.contains("?tag=fork-0.2.0-4"),
        "{error}"
    );
}

#[test]
fn the_lock_of_the_workspace_around_the_directory_is_read() {
    let app = Workspace::fixture("nsh-fork-app");
    let nested = app.path("src/nested");
    std::fs::create_dir_all(&nested).expect("dir");
    let pin = pin_of(&nested).expect("pin");
    assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
    assert_eq!(
        pin.dir, nested,
        "the pin is read in the directory, not the root"
    );
    assert!(!pin.app, "src/nested has no nsh.json");
}

#[test]
fn outside_any_workspace_the_crates_nsh_json_lists_decide() {
    let app = Workspace::fixture("nsh-split-app");
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
    assert_eq!(pin.origin, Origin::Project);

    // The listed crates disagree: both are named.
    let two = app.read("rust/two/Cargo.lock").replace(
        FORK_TAG,
        &format!(
            "git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-3#{}",
            "a".repeat(40)
        ),
    );
    app.write("rust/two/Cargo.lock", &two);
    let error = pin_of(&app.root)
        .expect_err("different sources")
        .to_string();
    assert!(error.contains("different sources"), "{error}");
    // Each lock as this host writes its path (`\\` on Windows).
    let lock = |name: &str| {
        Path::new("rust")
            .join(name)
            .join("Cargo.lock")
            .display()
            .to_string()
    };
    assert!(
        error.contains(&lock("one")) && error.contains(&lock("two")),
        "{error}"
    );
}

#[test]
fn an_app_without_nsh_crates_takes_the_nsh_table_of_rayx_toml() {
    let app = Workspace::fixture("nsh-ts-app");
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(pin.origin, Origin::RayxToml);
    assert_eq!(pin.origin.as_str(), "rayx.toml");
    assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
    assert_eq!(pin.label(), "fork-0.2.0-4 (327bb9e)");
    assert_eq!(pin.version, None, "rayx.toml names no NSH version");
    assert_eq!(pin.default_warning(), None);

    app.write("rayx.toml", "[nsh]\nversion = \"0.2.0\"\n");
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(
        pin.source,
        Source::Release {
            version: "0.2.0".into()
        }
    );
    assert_eq!(pin.version.as_deref(), Some("0.2.0"));
}

#[test]
fn a_broken_nsh_table_is_an_error() {
    let app = Workspace::fixture("nsh-ts-app");
    for (table, needle) in [
        (
            "[nsh]\ngit = \"https://github.com/dXnSix/nsh\"\n",
            "without `git` and `commit`",
        ),
        (
            "[nsh]\ngit = \"https://github.com/dXnSix/nsh\"\ncommit = \"327bb9e\"\n",
            "full commit",
        ),
        ("[nsh\n", "reading"),
    ] {
        app.write("rayx.toml", table);
        let error = format!("{:#}", pin_of(&app.root).expect_err(table));
        assert!(error.contains(needle), "{table:?}: {error}");
    }
}

#[test]
fn nsh_crates_and_an_nsh_table_together_stop_with_both_named() {
    let app = Workspace::fixture("nsh-fork-app");
    app.write(
        "rayx.toml",
        &Workspace::fixture("nsh-ts-app").read("rayx.toml"),
    );
    let error = pin_of(&app.root).expect_err("both").to_string();
    assert!(error.contains("pins nsh twice"), "{error}");
    assert!(
        error.contains("Cargo.lock") && error.contains("rayx.toml"),
        "{error}"
    );
}

#[test]
fn an_app_that_pins_nothing_gets_the_default_with_a_warning_showing_the_three_lines() {
    let app = Workspace::fixture("nsh-ts-app");
    std::fs::remove_file(app.path("rayx.toml")).expect("remove");
    let pin = pin_of(&app.root).expect("pin");
    assert_eq!(pin.origin, Origin::Default);
    assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
    let warning = pin.default_warning().expect("an app is warned");
    for line in [
        "[nsh]",
        "git = \"https://github.com/dXnSix/nsh\"",
        "tag = \"fork-0.2.0-4\"",
        "commit = \"327bb9ea65d08ffac4af498cc7fbbfa18a1b3623\"",
    ] {
        assert!(warning.contains(line), "{warning}");
    }
    assert!(
        warning.contains("rayx.toml") && warning.contains("nsh.json"),
        "{warning}"
    );
}

#[test]
fn outside_any_app_the_default_is_used_and_reported_as_default() {
    let empty = Workspace::empty();
    let pin = pin_of(&empty.root).expect("pin");
    assert_eq!(pin.origin, Origin::Default);
    assert_eq!(pin.origin.as_str(), "default");
    assert!(!pin.app);
    assert_eq!(pin.default_warning(), None, "no app, no warning");
    assert_eq!(pin, default_pin(&empty.root));
}

#[test]
fn the_defaults_table_holds_the_default_pin_and_its_crate_list() {
    let default = &DEFAULTS.nsh;
    assert_eq!(default.git, "https://github.com/dXnSix/nsh");
    assert_eq!(default.tag, "fork-0.2.0-4");
    assert_eq!(default.commit, COMMIT);
    assert_eq!(default.version, "0.2.0");
    assert_eq!(default.crates.len(), 26, "{:?}", default.crates);
    for name in default.crates {
        assert!(name.starts_with("nsh-"), "{name}");
    }
    for name in [
        "nsh-core",
        "nsh-cli",
        "nsh-macros",
        "nsh-platform-web",
        "nsh-ios-host",
    ] {
        assert!(default.crates.contains(&name), "{name}");
    }
    let mut sorted = default.crates.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), default.crates.len(), "each crate once");
}

#[test]
fn lock_sources_are_classified() {
    assert_eq!(
        classify(
            "registry+https://github.com/rust-lang/crates.io-index",
            "0.3.1"
        )
        .expect("ok"),
        Source::Release {
            version: "0.3.1".into()
        }
    );
    for bad in [
        "git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4",
        "git+https://github.com/dXnSix/nsh#327bb9e",
        "sparse+https://index.example/",
    ] {
        assert!(classify(bad, "0.2.0").is_err(), "{bad}");
    }
    let lock = "version = 4\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n";
    assert_eq!(
        lock_source(lock, Path::new("Cargo.lock")).expect("ok"),
        None
    );
}

#[test]
fn a_locked_project_is_read_from_its_files_when_cargo_cannot_run() {
    // A fresh machine: no cargo yet (or the project's toolchain is not installed).
    let no_cargo = || Runner::record().respond(|_| true, Outcome::failure(127));
    let app = fork_app_with(&format!(
        "git+https://github.com/dXnSix/nsh?branch=main#{COMMIT}"
    ));
    let pin = resolve(&mut no_cargo(), &app.root).expect("read from the files");
    assert_eq!(
        pin.origin,
        Origin::Project,
        "the project's pin, not the default"
    );
    assert!(
        matches!(&pin.source, Source::Git { reference: Reference::Branch(name), .. } if name == "main"),
        "{pin:?}"
    );
    // Outside any workspace, the crates `nsh.json` lists are found the same way.
    let split = Workspace::fixture("nsh-split-app");
    let pin = resolve(&mut no_cargo(), &split.root).expect("read from the files");
    assert_eq!(
        (pin.origin, pin.id().as_deref()),
        (Origin::Project, Some("327bb9ea65d0"))
    );
}

/// `rayx nsh <args>` in `dir` on a machine without cargo on `PATH` (before `rayx setup` installed
/// Rust), its store in `home`.
fn rayx_without_cargo(home: &Workspace, dir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    assert_cmd::cargo::cargo_bin_cmd!("rayx")
        .arg("nsh")
        .args(args)
        .current_dir(dir)
        .env("RAYX_HOME", &home.root)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("CARGO")
        .assert()
}

#[test]
fn without_cargo_on_the_path_the_lock_still_names_the_pin() {
    let home = Workspace::empty();
    let app = fork_app_with(&format!(
        "git+https://github.com/dXnSix/nsh?branch=main#{COMMIT}"
    ));
    let status = rayx_without_cargo(&home, &app.root, &["status"]).success();
    let text = String::from_utf8(status.get_output().stdout.clone()).expect("UTF-8");
    assert!(
        text.contains("main (327bb9e)") && text.contains("project"),
        "{text}"
    );
}

#[test]
fn a_workspace_with_neither_a_lock_nor_a_working_cargo_is_an_error() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    std::fs::remove_file(app.path("Cargo.lock")).expect("no lock");
    // `path` executes, so it would write a missing lock, but there is no cargo to write it.
    rayx_without_cargo(&home, &app.root, &["path", "--no-install"])
        .code(1)
        .stderr(predicates::str::contains(
            "has no Cargo.lock, and cargo could not write one",
        ));
    assert!(!app.path("Cargo.lock").exists());
    assert!(!home.path("nsh").exists(), "nothing installed in its place");
}

#[test]
fn an_apps_own_crates_are_not_nsh_packages() {
    // `nsh init` names an app's crate `nsh-app-<name>`: a workspace member, with no source, at the
    // app's own version.
    let lock = format!(
        "version = 4\n[[package]]\nname = \"nsh-app-hello\"\nversion = \"0.1.0\"\n\
         dependencies = [\"nsh-core\"]\n[[package]]\nname = \"nsh-core\"\nversion = \"0.2.0\"\n\
         source = \"{FORK_TAG}\"\n[[package]]\nname = \"nsh-app-hello-storage\"\n\
         version = \"0.1.0\"\n"
    );
    assert_eq!(
        lock_source(&lock, Path::new("Cargo.lock")).expect("one source"),
        Some((Some(FORK_TAG.to_string()), "0.2.0".to_string())),
        "the source and the version are NSH's"
    );
    let only_the_app = "version = 4\n[[package]]\nname = \"nsh-app-hello\"\nversion = \"0.1.0\"\n";
    assert_eq!(
        lock_source(only_the_app, Path::new("Cargo.lock")).expect("ok"),
        None
    );
}

#[test]
fn nsh_platform_gpui_is_an_nsh_package_outside_the_patch_list() {
    assert!(is_nsh_crate("nsh-platform-gpui"));
    assert!(!DEFAULTS.nsh.crates.contains(&"nsh-platform-gpui"));
    assert!(!is_nsh_crate("nsh-app-hello"));
    let lock = |gpui_source: &str| {
        format!(
            "version = 4\n[[package]]\nname = \"nsh-core\"\nversion = \"0.2.0\"\n\
             source = \"{FORK_TAG}\"\n[[package]]\nname = \"nsh-platform-gpui\"\n\
             version = \"0.2.0\"\nsource = \"{gpui_source}\"\n"
        )
    };
    assert!(lock_source(&lock(FORK_TAG), Path::new("Cargo.lock")).is_ok());
    let error = lock_source(
        &lock("registry+https://github.com/rust-lang/crates.io-index"),
        Path::new("Cargo.lock"),
    )
    .expect_err("two sources")
    .to_string();
    assert!(error.contains("nsh-platform-gpui"), "{error}");
}

#[test]
fn a_tag_whose_commit_is_not_the_recorded_one_is_an_error() {
    let app = Workspace::fixture("nsh-ts-app");
    let pin = pin_of(&app.root).expect("pin");
    let answer = |commit: &str| {
        let line = format!("{commit}\trefs/tags/fork-0.2.0-4\n");
        Runner::record().respond(
            |spec| spec.program == "git" && spec.args.first().is_some_and(|a| a == "ls-remote"),
            Outcome::success().with_stdout(line),
        )
    };
    let mut same = answer(COMMIT);
    verify_tag(&mut same, &pin).expect("the tag gives the recorded commit");
    let command = same.lines().join("\n");
    assert!(
        command.contains("git ls-remote https://github.com/dXnSix/nsh refs/tags/fork-0.2.0-4"),
        "{command}"
    );

    let other = "b".repeat(40);
    let error = verify_tag(&mut answer(&other), &pin)
        .expect_err("a moved tag")
        .to_string();
    assert!(
        error.contains(&other[..12]) && error.contains("327bb9ea65d0"),
        "{error}"
    );

    // The peeled commit of an annotated tag wins over the tag object.
    let peeled = format!(
        "{}\trefs/tags/fork-0.2.0-4\n{COMMIT}\trefs/tags/fork-0.2.0-4^{{}}\n",
        "c".repeat(40)
    );
    let mut annotated = Runner::record().respond(|_| true, Outcome::success().with_stdout(peeled));
    verify_tag(&mut annotated, &pin).expect("peeled");

    // A lock pin was checked by Cargo already; nothing is asked.
    let fork = Workspace::fixture("nsh-fork-app");
    let mut quiet = Runner::record();
    verify_tag(&mut quiet, &pin_of(&fork.root).expect("pin")).expect("nothing to check");
    assert!(quiet.lines().is_empty());
}

#[test]
fn the_pin_of_a_relative_directory_is_absolute() {
    let app = Workspace::fixture("nsh-fork-app");
    let pin = pin_of(&app.root.join("src").join("..")).expect("pin");
    assert!(pin.dir.is_absolute());
    assert_eq!(pin.id().as_deref(), Some("327bb9ea65d0"));
    let _: PathBuf = pin.dir;
}
