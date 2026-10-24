//! Installing nsh (INST-1, INST-2, design §4.3): the release archive for the host with its digest,
//! the source fallback with its exact commands, and the CLI's version and commit check. Commands
//! are recorded; archives are built here and "downloaded" by the recorded `curl`.

mod common;

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::Workspace;
use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::nsh::install::{
    Install, RustTools, archive_name, archive_url, parse_version, platform, sha256_of, unpack,
};
use rayx_cli::nsh::pin::{Origin, Pin, Reference, ReleaseSource, Source};
use rayx_cli::nsh::store::{Kind, Store};

const COMMIT: &str = "327bb9ea65d08ffac4af498cc7fbbfa18a1b3623";
const ID: &str = "327bb9ea65d0";
const VERSION_LINE: &str = "nsh 0.2.0 (327bb9e)\n";

fn host(os: Os, arch: Arch) -> HostFacts {
    HostFacts {
        os,
        arch,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn mac() -> HostFacts {
    host(Os::MacOs, Arch::Arm64)
}

fn fork_pin(reference: Reference) -> Pin {
    Pin {
        source: Source::Git {
            url: "https://github.com/dXnSix/nsh".into(),
            reference,
            commit: COMMIT.into(),
        },
        origin: Origin::Project,
        version: Some("0.2.0".into()),
        dir: PathBuf::from("/work/app"),
        app: true,
    }
}

fn tag_pin() -> Pin {
    fork_pin(Reference::Tag("fork-0.2.0-4".into()))
}

/// A `nsh-<platform>.tar.gz` holding `nsh/bin/nsh` and a runtime file.
fn tar_gz(extra: &[(&str, &str)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::fast());
        let mut tar = tar::Builder::new(encoder);
        // The name goes into the header as it is, so a test can plant `..`, which the builder's
        // own path setter refuses.
        let mut add = |path: &str, contents: &str, mode: u32| {
            let mut header = tar::Header::new_old();
            header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_size(contents.len() as u64);
            header.set_mode(mode);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            tar.append(&header, contents.as_bytes()).expect("append");
        };
        add(
            "nsh/bin/nsh",
            "#!/bin/sh\necho nsh 0.2.0 (327bb9e)\n",
            0o755,
        );
        add("nsh/runtime/web/nsh.js", "// runtime", 0o644);
        for (path, contents) in extra {
            add(path, contents, 0o644);
        }
        tar.into_inner().expect("tar").finish().expect("gzip");
    }
    bytes
}

/// A `nsh-windows-x86_64.zip` holding `nsh/bin/nsh.exe`.
fn zip_bytes() -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zip.start_file("nsh/bin/nsh.exe", options).expect("file");
        zip.write_all(b"MZ").expect("write");
        zip.start_file("nsh/runtime/windows/nsh-windows-host.exe", options)
            .expect("file");
        zip.write_all(b"MZ").expect("write");
        zip.finish().expect("finish");
    }
    cursor.into_inner()
}

fn hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What the recorded `curl` serves: the URL's bytes, or `None` for a 404.
type Served = Rc<RefCell<Vec<(String, Option<Vec<u8>>)>>>;

/// A recorded runner whose `curl` writes what `served` holds to its `-o` file, whose CLI answers
/// `version_line`, and whose `git rev-parse HEAD` names the commit once a checkout ran.
fn runner(served: &Served, version_line: &str) -> Runner {
    runner_from(served, version_line, false)
}

/// [`runner`] whose checkout is at the commit from the start (`at_commit`).
fn runner_from(served: &Served, version_line: &str, at_commit: bool) -> Runner {
    let served = served.clone();
    let version_line = version_line.to_string();
    let checked_out = Rc::new(Cell::new(at_commit));
    let observed = checked_out.clone();
    Runner::record()
        .observe(move |spec| {
            if spec.program == "git" && spec.args.first().is_some_and(|a| a == "checkout") {
                observed.set(true);
            }
        })
        .responder(move |spec| {
            if spec.program == "curl" {
                let url = spec.args.last().expect("a URL").clone();
                let out = spec
                    .args
                    .iter()
                    .position(|a| a == "-o")
                    .map(|i| PathBuf::from(&spec.args[i + 1]))
                    .expect("-o");
                let found = served
                    .borrow()
                    .iter()
                    .find(|(u, _)| *u == url)
                    .map(|(_, body)| body.clone());
                return Some(match found.flatten() {
                    Some(body) => {
                        std::fs::write(&out, body).expect("write the download");
                        Outcome::success().with_stdout("200")
                    }
                    None => Outcome::success().with_stdout("404"),
                });
            }
            if spec.args == ["--version"] {
                return Some(Outcome::success().with_stdout(version_line.clone()));
            }
            if spec.program == "git" && spec.args == ["rev-parse", "HEAD"] {
                return Some(if checked_out.get() {
                    Outcome::success().with_stdout(format!("{COMMIT}\n"))
                } else {
                    Outcome::failure(128)
                });
            }
            None
        })
}

fn release_url(platform: &str) -> String {
    archive_url(
        &ReleaseSource {
            repository: "dXnSix/nsh".into(),
            tag: "fork-0.2.0-4".into(),
        },
        &archive_name(platform),
    )
}

/// Serves the macOS archive and its digest (`digest` overrides the right one).
fn serve_release(archive: Vec<u8>, digest: Option<String>) -> Served {
    let url = release_url("macos-arm64");
    let digest = digest.unwrap_or_else(|| hex(&archive));
    Rc::new(RefCell::new(vec![
        (
            format!("{url}.sha256"),
            Some(format!("{digest}  nsh-macos-arm64.tar.gz\n").into_bytes()),
        ),
        (url, Some(archive)),
    ]))
}

fn install(
    runner: &mut Runner,
    store: &Store,
    host: &HostFacts,
    pin: &Pin,
) -> (anyhow::Result<rayx_cli::nsh::install::Installed>, String) {
    let mut out = Vec::new();
    let result = Install {
        runner,
        store,
        host,
        tools: &RustTools::on_path(),
        out: &mut out,
    }
    .run(pin);
    (result, String::from_utf8(out).expect("UTF-8"))
}

/// The recorded commands as `program args…`, without the `cd <dir> &&` a rendered line has.
fn lines(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .map(|spec| format!("{} {}", spec.program, spec.args.join(" ")))
        .collect()
}

#[test]
fn platform_names_follow_the_release_archives() {
    assert_eq!(platform(&host(Os::MacOs, Arch::Arm64)), Some("macos-arm64"));
    assert_eq!(platform(&host(Os::Linux, Arch::X64)), Some("linux-x86_64"));
    assert_eq!(platform(&host(Os::Linux, Arch::Arm64)), Some("linux-arm64"));
    assert_eq!(
        platform(&host(Os::Windows, Arch::X64)),
        Some("windows-x86_64")
    );
    assert_eq!(
        platform(&host(Os::Windows, Arch::Arm64)),
        Some("windows-arm64")
    );
    assert_eq!(
        platform(&host(Os::MacOs, Arch::X64)),
        None,
        "Intel Macs build from source"
    );
    assert_eq!(platform(&host(Os::Linux, Arch::Other)), None);
}

#[test]
fn archive_names_and_urls_are_the_releases() {
    assert_eq!(archive_name("macos-arm64"), "nsh-macos-arm64.tar.gz");
    assert_eq!(archive_name("windows-x86_64"), "nsh-windows-x86_64.zip");
    assert_eq!(
        release_url("linux-x86_64"),
        "https://github.com/dXnSix/nsh/releases/download/fork-0.2.0-4/nsh-linux-x86_64.tar.gz"
    );
    let upstream = Pin {
        source: Source::Release {
            version: "0.2.0".into(),
        },
        version: Some("0.2.0".into()),
        ..tag_pin()
    };
    assert_eq!(
        archive_url(
            &upstream.release().expect("release"),
            &archive_name("macos-arm64")
        ),
        "https://github.com/longbridge/nsh/releases/download/v0.2.0/nsh-macos-arm64.tar.gz"
    );
}

#[test]
fn a_release_install_checks_the_digest_unpacks_nsh_and_writes_a_distribution_receipt() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served = serve_release(tar_gz(&[]), None);
    let mut runner = runner(&served, VERSION_LINE);
    let (result, out) = install(&mut runner, &store, &mac(), &tag_pin());
    let installed = result.expect("installs");

    let entry = home.path("nsh").join(ID);
    assert_eq!(installed.cli, entry.join("nsh").join("bin").join("nsh"));
    assert!(installed.cli.is_file());
    assert!(
        entry.join("nsh/runtime/web/nsh.js").is_file(),
        "the runtimes come along"
    );
    assert!(
        !entry.join("download").exists(),
        "the download is cleaned up"
    );
    let receipt = store.receipt(ID).expect("complete");
    assert_eq!(receipt.kind, Kind::Distribution);
    assert_eq!(receipt.url, release_url("macos-arm64"));
    assert_eq!(receipt.reference.as_deref(), Some("fork-0.2.0-4"));
    assert_eq!(receipt.commit.as_deref(), Some(COMMIT));
    assert_eq!(receipt.nsh, "0.2.0");
    assert_eq!(receipt.cli, "nsh/bin/nsh");
    assert!(!installed.already);

    let commands = lines(&runner);
    assert!(
        commands[0].contains("curl --proto =https --tlsv1.2"),
        "{commands:?}"
    );
    assert!(
        commands[0].ends_with("nsh-macos-arm64.tar.gz.sha256"),
        "{commands:?}"
    );
    // The CLI's path as this host writes it (`\\` on Windows), compared by its components.
    assert!(
        runner.specs().iter().any(|spec| spec.args == ["--version"]
            && Path::new(&spec.program).ends_with(Path::new("nsh").join("bin").join("nsh"))),
        "{commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|l| l.starts_with("git") || l.starts_with("cargo"))
    );
    assert!(
        out.contains("Installed nsh fork-0.2.0-4 (327bb9e) (distribution)"),
        "{out}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed.cli)
            .expect("cli")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the CLI stays executable");
    }
}

#[test]
fn a_digest_mismatch_installs_nothing_and_names_both_digests() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let archive = tar_gz(&[]);
    let right = hex(&archive);
    let wrong = "0".repeat(64);
    let served = serve_release(archive, Some(wrong.clone()));
    let (result, _) = install(
        &mut runner(&served, VERSION_LINE),
        &store,
        &mac(),
        &tag_pin(),
    );
    let error = format!("{:#}", result.expect_err("refused"));
    assert!(error.contains(&wrong) && error.contains(&right), "{error}");
    assert!(error.contains("nothing was installed"), "{error}");
    assert!(store.receipt(ID).is_none());
    assert!(!home.path("nsh").join(ID).join("nsh").exists());
}

#[test]
fn a_release_without_an_archive_for_the_host_falls_back_to_source_naming_it() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served: Served = Rc::new(RefCell::new(Vec::new())); // every URL is a 404
    let mut runner = runner(&served, VERSION_LINE);
    let (result, out) = install(&mut runner, &store, &mac(), &tag_pin());
    let installed = result.expect("installs from source");
    assert!(
        out.contains("the release fork-0.2.0-4 of dXnSix/nsh has no nsh-macos-arm64.tar.gz; building nsh 327bb9ea65d0 from source"),
        "{out}"
    );
    assert_eq!(installed.receipt.kind, Kind::Source);
    assert_eq!(installed.receipt.cli, "source/target/release/nsh");
    assert_eq!(installed.receipt.url, "https://github.com/dXnSix/nsh");
    assert!(lines(&runner).iter().any(|l| l.starts_with("cargo build")));
}

#[test]
fn a_host_without_an_archive_builds_from_source() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let mut runner = runner(&served, VERSION_LINE);
    let (result, out) = install(&mut runner, &store, &host(Os::MacOs, Arch::X64), &tag_pin());
    assert_eq!(result.expect("installs").receipt.kind, Kind::Source);
    assert!(
        out.contains("no NSH archive is built for this host (macos x64)"),
        "{out}"
    );
    assert!(
        !lines(&runner).iter().any(|l| l.starts_with("curl")),
        "nothing to download"
    );
}

#[test]
fn a_branch_pin_builds_from_source_with_exactly_these_commands_without_cargo() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let mut runner = runner(&served, VERSION_LINE);
    let pin = fork_pin(Reference::Branch("feat/x".into()));
    let (result, _) = install(&mut runner, &store, &mac(), &pin);
    let installed = result.expect("installs");
    let source = home.path("nsh").join(ID).join("source");
    assert_eq!(
        installed.cli,
        source.join("target").join("release").join("nsh")
    );
    assert_eq!(installed.receipt.reference.as_deref(), Some("feat/x"));

    let specs: Vec<&CommandSpec> = runner
        .specs()
        .iter()
        .filter(|s| s.args != ["--version"] && !(s.program == "git" && s.args[0] == "rev-parse"))
        .collect();
    let shown: Vec<String> = specs
        .iter()
        .map(|s| format!("{} {}", s.program, s.args.join(" ")))
        .collect();
    assert_eq!(
        shown,
        [
            "git init --quiet".to_string(),
            format!("git fetch --depth 1 https://github.com/dXnSix/nsh {COMMIT}"),
            "git checkout --quiet --detach FETCH_HEAD".to_string(),
            "rustup toolchain install".to_string(),
            "cargo build --release --locked -p nsh-cli --target-dir target".to_string(),
        ]
    );
    for spec in &specs {
        assert_eq!(spec.cwd.as_deref(), Some(source.as_path()), "{spec:?}");
    }
    for spec in specs
        .iter()
        .filter(|s| s.program == "cargo" || s.program == "rustup")
    {
        assert!(
            spec.unset.contains(&"CARGO".to_string()),
            "{spec:?} runs without CARGO"
        );
        // A caller started through rustup's proxy (`cargo run`, `cargo xtask`) has its own
        // toolchain in RUSTUP_TOOLCHAIN, which would override NSH's rust-toolchain.toml.
        assert!(
            spec.unset.contains(&"RUSTUP_TOOLCHAIN".to_string()),
            "{spec:?} runs without RUSTUP_TOOLCHAIN"
        );
    }
    assert!(!lines(&runner).iter().any(|l| l.contains("core.longPaths")));
}

#[test]
fn the_source_build_runs_the_rustup_and_cargo_it_is_given() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let mut runner = runner(&served, VERSION_LINE);
    let tools = RustTools {
        rustup: "/Users/dev/.cargo/bin/rustup".into(),
        cargo: "/Users/dev/.cargo/bin/cargo".into(),
    };
    let mut out = Vec::new();
    Install {
        runner: &mut runner,
        store: &store,
        host: &mac(),
        tools: &tools,
        out: &mut out,
    }
    .run(&fork_pin(Reference::Branch("main".into())))
    .expect("installs");
    let programs: Vec<&str> = runner
        .specs()
        .iter()
        .map(|spec| spec.program.as_str())
        .filter(|program| program.contains(".cargo"))
        .collect();
    assert_eq!(
        programs,
        [
            "/Users/dev/.cargo/bin/rustup",
            "/Users/dev/.cargo/bin/cargo"
        ]
    );
}

#[test]
fn a_host_that_refuses_a_commit_is_fetched_by_the_ref() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let mut runner = runner(&served, VERSION_LINE).respond(
        |spec| spec.program == "git" && spec.args.get(4).is_some_and(|a| a == COMMIT),
        Outcome::failure(128),
    );
    let pin = fork_pin(Reference::Branch("main".into()));
    install(&mut runner, &store, &mac(), &pin)
        .0
        .expect("installs");
    let fetches: Vec<String> = lines(&runner)
        .into_iter()
        .filter(|l| l.starts_with("git fetch"))
        .collect();
    assert_eq!(fetches.len(), 2, "{fetches:?}");
    assert!(
        fetches[1].ends_with("https://github.com/dXnSix/nsh main"),
        "{fetches:?}"
    );
}

#[test]
fn a_windows_checkout_uses_long_paths() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let mut runner = runner(&served, VERSION_LINE);
    let pin = fork_pin(Reference::Branch("main".into()));
    let (result, _) = install(&mut runner, &store, &host(Os::Windows, Arch::X64), &pin);
    assert_eq!(
        result.expect("installs").receipt.cli,
        "source/target/release/nsh.exe"
    );
    let commands = lines(&runner);
    let long = commands
        .iter()
        .position(|l| l == "git config core.longPaths true")
        .unwrap_or_else(|| panic!("{commands:?}"));
    let fetch = commands
        .iter()
        .position(|l| l.starts_with("git fetch"))
        .expect("fetch");
    assert!(long < fetch, "before the checkout: {commands:?}");
}

#[test]
fn a_cli_of_another_version_or_commit_is_refused_and_nothing_is_recorded() {
    for (line, needle) in [
        (
            "nsh 0.3.0 (327bb9e)\n",
            "version 0.3.0, but the project's NSH crates are 0.2.0",
        ),
        (
            "nsh 0.2.0 (5f0c2a9)\n",
            "built from 5f0c2a9, but the pin is 327bb9ea65d0",
        ),
        ("nsh 0.2.0\n", "names no commit"),
        ("nonsense\n", "not `nsh <version> (<commit>)`"),
    ] {
        let home = Workspace::empty();
        let store = Store::at(home.path("nsh"));
        let served = serve_release(tar_gz(&[]), None);
        let (result, _) = install(&mut runner(&served, line), &store, &mac(), &tag_pin());
        let error = format!("{:#}", result.expect_err(line));
        assert!(error.contains(needle), "{line:?}: {error}");
        assert!(store.receipt(ID).is_none(), "{line:?}: nothing is recorded");
    }
}

#[test]
fn an_upstream_release_pin_checks_only_the_version() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let pin = Pin {
        source: Source::Release {
            version: "0.2.0".into(),
        },
        ..tag_pin()
    };
    let url = "https://github.com/longbridge/nsh/releases/download/v0.2.0/nsh-macos-arm64.tar.gz";
    let archive = tar_gz(&[]);
    let served: Served = Rc::new(RefCell::new(vec![
        (
            format!("{url}.sha256"),
            Some(format!("{}  x\n", hex(&archive)).into_bytes()),
        ),
        (url.to_string(), Some(archive)),
    ]));
    let installed = install(&mut runner(&served, "nsh 0.2.0\n"), &store, &mac(), &pin)
        .0
        .expect("installs");
    assert_eq!(installed.receipt.id, "v0.2.0");
    assert_eq!(installed.receipt.reference.as_deref(), Some("v0.2.0"));
    assert_eq!(installed.receipt.commit, None);
}

#[test]
fn a_complete_entry_is_not_installed_again() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served = serve_release(tar_gz(&[]), None);
    install(
        &mut runner(&served, VERSION_LINE),
        &store,
        &mac(),
        &tag_pin(),
    )
    .0
    .expect("first");
    let mut again = runner(&served, VERSION_LINE);
    let installed = install(&mut again, &store, &mac(), &tag_pin())
        .0
        .expect("again");
    assert!(installed.already);
    assert_eq!(
        lines(&again).len(),
        1,
        "only the CLI's --version: {:?}",
        lines(&again)
    );
}

#[test]
fn an_unfinished_source_entry_reuses_its_checkout() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    // An earlier install got as far as the checkout and stopped in the build.
    home.write(&format!("nsh/{ID}/source/.git/HEAD"), "ref");
    home.write(&format!("nsh/{ID}/source/target/release/.cargo-lock"), "");
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let mut runner = runner_from(&served, VERSION_LINE, true);
    let pin = fork_pin(Reference::Branch("main".into()));
    install(&mut runner, &store, &mac(), &pin)
        .0
        .expect("resumes");
    let commands = lines(&runner);
    assert!(
        !commands
            .iter()
            .any(|l| l.starts_with("git init") || l.starts_with("git fetch")),
        "{commands:?}"
    );
    assert!(commands.iter().any(|l| l.starts_with("cargo build")));
    assert!(
        home.path(&format!("nsh/{ID}/source/target/release/.cargo-lock"))
            .exists(),
        "the target directory is kept"
    );
}

#[test]
fn an_unfinished_distribution_is_downloaded_again() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    home.write(&format!("nsh/{ID}/nsh/bin/half-written"), "x");
    let served = serve_release(tar_gz(&[]), None);
    let mut runner = runner(&served, VERSION_LINE);
    install(&mut runner, &store, &mac(), &tag_pin())
        .0
        .expect("installs");
    assert!(
        !home
            .path(&format!("nsh/{ID}/nsh/bin/half-written"))
            .exists()
    );
    assert!(home.path(&format!("nsh/{ID}/nsh/bin/nsh")).is_file());
    assert_eq!(
        lines(&runner)
            .iter()
            .filter(|l| l.starts_with("curl"))
            .count(),
        2,
        "the digest and the archive again"
    );
}

#[test]
fn a_windows_release_is_a_zip() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let url = release_url("windows-x86_64");
    let archive = zip_bytes();
    let served: Served = Rc::new(RefCell::new(vec![
        (
            format!("{url}.sha256"),
            Some(format!("{}  nsh-windows-x86_64.zip\n", hex(&archive)).into_bytes()),
        ),
        (url.clone(), Some(archive)),
    ]));
    let installed = install(
        &mut runner(&served, VERSION_LINE),
        &store,
        &host(Os::Windows, Arch::X64),
        &tag_pin(),
    )
    .0
    .expect("installs");
    assert_eq!(installed.receipt.cli, "nsh/bin/nsh.exe");
    assert!(installed.cli.is_file());
    assert!(
        home.path(&format!(
            "nsh/{ID}/nsh/runtime/windows/nsh-windows-host.exe"
        ))
        .is_file()
    );
    assert_eq!(installed.receipt.url, url);
}

#[test]
fn an_archive_with_anything_outside_nsh_is_refused() {
    let dir = Workspace::empty();
    let bad = tar_gz(&[("../escape", "x")]);
    let file = dir.path("nsh-macos-arm64.tar.gz");
    std::fs::write(&file, &bad).expect("write");
    assert!(unpack(&file, &dir.path("out")).is_err());
    assert!(!dir.root.join("escape").exists());

    let other = tar_gz(&[("README.md", "x")]);
    std::fs::write(&file, &other).expect("write");
    let error = format!(
        "{:#}",
        unpack(&file, &dir.path("out")).expect_err("outside nsh/")
    );
    assert!(error.contains("outside nsh/"), "{error}");
    assert_eq!(sha256_of(&file).expect("digest"), hex(&other));
}

#[test]
fn a_rayx_toml_tag_that_moved_is_refused_before_anything_is_installed() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let pin = Pin {
        origin: Origin::RayxToml,
        version: None,
        ..tag_pin()
    };
    let served = serve_release(tar_gz(&[]), None);
    let mut runner = runner(&served, VERSION_LINE).respond(
        |spec| spec.program == "git" && spec.args.first().is_some_and(|a| a == "ls-remote"),
        Outcome::success().with_stdout(format!("{}\trefs/tags/fork-0.2.0-4\n", "d".repeat(40))),
    );
    let error = format!(
        "{:#}",
        install(&mut runner, &store, &mac(), &pin)
            .0
            .expect_err("moved")
    );
    assert!(error.contains("is at dddddddddddd"), "{error}");
    assert!(!lines(&runner).iter().any(|l| l.starts_with("curl")));
    assert!(store.receipt(ID).is_none());
}

#[test]
fn the_cli_version_line_is_read_with_or_without_a_commit() {
    let both = parse_version("nsh 0.2.0 (327bb9e)\nmore").expect("version");
    assert_eq!(
        (both.version.as_str(), both.commit.as_deref()),
        ("0.2.0", Some("327bb9e"))
    );
    let bare = parse_version("nsh 0.2.0\n").expect("version");
    assert_eq!((bare.version.as_str(), bare.commit), ("0.2.0", None));
    assert_eq!(parse_version("rayx 1.0.0"), None);
    assert!(
        Path::new(&archive_name("linux-arm64"))
            .extension()
            .is_some()
    );
}

#[test]
fn a_broken_entry_loses_its_receipt_before_it_is_installed_again() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    let served = serve_release(tar_gz(&[]), None);
    install(
        &mut runner(&served, VERSION_LINE),
        &store,
        &mac(),
        &tag_pin(),
    )
    .0
    .expect("first");
    // The CLI now answers as another nsh, and the release is gone: the repair cannot finish.
    let gone: Served = Rc::new(RefCell::new(Vec::new()));
    let mut broken = runner(&gone, "nsh 0.1.9 (1111111)");
    let (result, out) = install(&mut broken, &store, &mac(), &tag_pin());
    assert!(result.is_err(), "the repair fails: {out}");
    assert!(
        out.contains("rayx: nsh 327bb9ea65d0 does not answer as nsh fork-0.2.0-4 (327bb9e) should; installing it again"),
        "{out}"
    );
    assert_eq!(
        store.receipt("327bb9ea65d0"),
        None,
        "an interrupted repair leaves an unfinished entry, never an installed one"
    );
}

#[test]
fn a_dry_run_writes_nothing_into_the_store() {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    for pin in [tag_pin(), fork_pin(Reference::Branch("main".into()))] {
        let mut runner = Runner::print();
        let (result, out) = install(&mut runner, &store, &mac(), &pin);
        result.expect("a dry run plans the install");
        assert!(!home.path("nsh").exists(), "no lock, no entry: {out}");
    }
}
