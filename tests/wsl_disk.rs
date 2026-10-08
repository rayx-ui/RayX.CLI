//! `rayx wsl status` and `rayx wsl compact`: the registry view, the sizes, and the step plan of a
//! compaction, over a fake registry, fake `wsl` output and fixture sizes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rayx_cli::cli::CompactArgs;
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Privilege, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::wsl::{
    CompactError, Distribution, compact_script, compact_with, encode_powershell, human,
    parse_status, render_status, status, status_json,
};

const VHDX: &str = r"C:\Users\dev\AppData\Local\wsl\ubuntu\ext4.vhdx";

fn distribution(name: &str, version: u32, uid: u32) -> Distribution {
    Distribution {
        id: format!("{{{name}}}"),
        name: name.into(),
        base_path: PathBuf::from(r"\\?\C:\Users\dev\AppData\Local\wsl\ubuntu"),
        version: Some(version),
        default_uid: Some(uid),
    }
}

fn windows() -> HostFacts {
    HostFacts {
        os: Os::Windows,
        arch: Arch::X64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

const GIB: u64 = 1024 * 1024 * 1024;

fn status_output() -> Outcome {
    Outcome::success().with_stdout(format!(
        "DF\t{}\t{}\nOUT\t{}\t/home/dev/RayX/target\nOUT\t{}\t/home/dev/RayX/artifacts-temp\n",
        20 * GIB,
        80 * GIB,
        15 * GIB,
        GIB
    ))
}

fn is_status_script(spec: &rayx_cli::host::CommandSpec) -> bool {
    spec.args.last().is_some_and(|a| a.contains("wsl-clones"))
}

fn compact_args(clean: bool, yes: bool) -> CompactArgs {
    CompactArgs {
        distro: None,
        clean,
        yes,
    }
}

/// Runs a compaction and returns the result, the command lines in order and the questions asked.
fn compact(
    args: &CompactArgs,
    host: &HostFacts,
    machine: &FakeMachine,
    answer: bool,
) -> (
    Result<rayx_cli::wsl::CompactReport, CompactError>,
    Vec<String>,
    Vec<rayx_cli::host::CommandSpec>,
    Vec<String>,
) {
    let mut runner = Runner::record()
        .with_os(Os::Windows)
        .respond(is_status_script, status_output());
    let sizes = Rc::new(RefCell::new(vec![40 * GIB, 22 * GIB]));
    let size_state = sizes.clone();
    let size_of = move |_: &std::path::Path| {
        let mut sizes = size_state.borrow_mut();
        if sizes.len() > 1 {
            Some(sizes.remove(0))
        } else {
            sizes.first().copied()
        }
    };
    let mut asked = Vec::new();
    let result = compact_with(
        args,
        host,
        &mut runner,
        machine,
        &size_of,
        &mut |question| {
            asked.push(question.to_string());
            answer
        },
        &mut Vec::new(),
    );
    (
        result,
        runner.lines().to_vec(),
        runner.specs().to_vec(),
        asked,
    )
}

fn machine() -> FakeMachine {
    FakeMachine::new()
        .with_wsl_distributions(vec![distribution("Ubuntu-24.04", 2, 1000)])
        .with_size(VHDX, 40 * GIB)
}

#[test]
fn the_status_script_output_is_read_into_sizes() {
    let text = "DF\t100\t200\nOUT\t5\t/home/dev/x/target\nnoise\nOUT\tbad\t/skipped\nOUT\t7\t/home/dev/x/artifacts\n";
    let (used, free, outputs) = parse_status(text);
    assert_eq!((used, free), (Some(100), Some(200)));
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].path, "/home/dev/x/target");
    assert_eq!(outputs[1].bytes, 7);
    assert_eq!(parse_status(""), (None, None, Vec::new()));
}

#[test]
fn only_absolute_build_output_paths_are_ever_listed() {
    let (_, _, outputs) = parse_status(
        "OUT	5	work/rayx/target
OUT	7	/home/dev/x/target
",
    );

    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].path, "/home/dev/x/target");
}

#[test]
fn a_build_output_path_with_a_quote_is_never_deleted() {
    let mut runner = Runner::record().with_os(Os::Windows).respond(
        is_status_script,
        Outcome::success().with_stdout(
            "OUT	5	/home/o'brien/x/target
",
        ),
    );
    let result = compact_with(
        &compact_args(true, true),
        &windows(),
        &mut runner,
        &machine(),
        &|_| Some(GIB),
        &mut |_| true,
        &mut Vec::new(),
    );

    assert!(
        result
            .expect_err("refused")
            .to_string()
            .contains("not a plain build output")
    );
    assert!(!runner.lines().iter().any(|l| l.contains("rm -rf")));
}

#[test]
fn status_lists_each_distribution_with_its_disk_and_build_output() {
    let machine = machine();
    let mut runner = Runner::record()
        .with_os(Os::Windows)
        .respond(is_status_script, status_output());

    let distributions = status(&mut runner, &machine);

    assert_eq!(distributions.len(), 1);
    let ubuntu = &distributions[0];
    assert_eq!(ubuntu.name, "Ubuntu-24.04");
    assert_eq!(ubuntu.vhdx, PathBuf::from(VHDX));
    assert_eq!(ubuntu.vhdx_bytes, Some(40 * GIB));
    assert_eq!(
        (ubuntu.used_bytes, ubuntu.free_bytes),
        (Some(20 * GIB), Some(80 * GIB))
    );
    assert_eq!(ubuntu.build_output_bytes(), 16 * GIB);

    let text = render_status(&distributions);
    assert!(
        text.contains("Ubuntu-24.04 (WSL 2, default user uid 1000)"),
        "{text}"
    );
    assert!(text.contains("40.0 GiB on the Windows disk"), "{text}");
    assert!(text.contains("20.0 GiB used, 80.0 GiB free"), "{text}");
    assert!(
        text.contains("16.0 GiB under the cloned checkouts"),
        "{text}"
    );
    assert!(text.contains("/home/dev/RayX/target"), "{text}");
}

#[test]
fn status_json_carries_the_same_numbers() {
    let machine = machine();
    let mut runner = Runner::record()
        .with_os(Os::Windows)
        .respond(is_status_script, status_output());

    let json = status_json(&status(&mut runner, &machine));

    let ubuntu = &json["distributions"][0];
    assert_eq!(ubuntu["name"], "Ubuntu-24.04");
    assert_eq!(ubuntu["vhdxBytes"], 40 * GIB);
    assert_eq!(ubuntu["usedBytes"], 20 * GIB);
    assert_eq!(ubuntu["freeBytes"], 80 * GIB);
    assert_eq!(ubuntu["buildOutputBytes"], 16 * GIB);
    assert_eq!(ubuntu["buildOutput"][0]["path"], "/home/dev/RayX/target");
    assert_eq!(ubuntu["wslVersion"], 2);
}

#[test]
fn a_wsl_1_distribution_is_listed_without_asking_it_anything() {
    let machine = FakeMachine::new()
        .with_wsl_distributions(vec![distribution("Debian", 1, 0)])
        .with_size(VHDX, GIB);
    let mut runner = Runner::record().with_os(Os::Windows);

    let distributions = status(&mut runner, &machine);

    assert!(
        runner.lines().is_empty(),
        "no command runs for a WSL 1 distribution"
    );
    let text = render_status(&distributions);
    assert!(
        text.contains("Debian (WSL 1, default user root: none created yet)"),
        "{text}"
    );
    assert!(
        text.contains("inside        unknown used, unknown free"),
        "{text}"
    );
}

#[test]
fn no_distributions_say_so() {
    assert_eq!(render_status(&[]), "No WSL distributions are installed.\n");
}

#[test]
fn sizes_read_in_binary_units() {
    assert_eq!(human(0), "0 B");
    assert_eq!(human(1023), "1023 B");
    assert_eq!(human(1536), "1.5 KiB");
    assert_eq!(human(40 * GIB), "40.0 GiB");
    assert_eq!(human(3 * 1024 * GIB), "3.0 TiB");
}

#[test]
fn compact_trims_confirms_the_shutdown_and_compacts_in_one_admin_child() {
    let (result, lines, specs, asked) =
        compact(&compact_args(false, false), &windows(), &machine(), true);

    let report = result.expect("compacts");
    assert_eq!(report.before, Some(40 * GIB));
    assert_eq!(report.after, Some(22 * GIB));
    assert!(
        report
            .render()
            .contains("40.0 GiB before, 22.0 GiB after, 18.0 GiB given back")
    );

    let trim = lines
        .iter()
        .position(|l| l.contains("sudo fstrim -av"))
        .expect("fstrim");
    let shutdown = lines
        .iter()
        .position(|l| l == "wsl --shutdown")
        .expect("shutdown");
    let compaction = lines
        .iter()
        .position(|l| l.contains("-EncodedCommand"))
        .expect("compact");
    assert!(trim < shutdown && shutdown < compaction, "{lines:?}");
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains("wsl --shutdown") && asked[0].contains("VS Code"),
        "{asked:?}"
    );

    let admin: Vec<_> = specs
        .iter()
        .filter(|s| s.privilege == Privilege::Admin)
        .collect();
    assert_eq!(admin.len(), 1, "one elevated child");
    assert!(
        specs
            .iter()
            .find(|s| s.args.iter().any(|a| a == "fstrim"))
            .expect("fstrim")
            .interactive
    );
}

#[test]
fn declining_the_shutdown_stops_before_it() {
    let (result, lines, _, _) = compact(&compact_args(false, false), &windows(), &machine(), false);

    let error = result.expect_err("declined");
    assert!(matches!(error, CompactError::Declined(_)), "{error}");
    assert!(!lines.iter().any(|l| l == "wsl --shutdown"), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains("-EncodedCommand")),
        "{lines:?}"
    );
}

#[test]
fn yes_skips_every_question() {
    let (result, _, _, asked) = compact(&compact_args(true, true), &windows(), &machine(), false);

    let report = result.expect("compacts");
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(report.deleted.len(), 2);
}

#[test]
fn clean_lists_and_deletes_build_output_after_confirmation() {
    let (result, lines, _, asked) =
        compact(&compact_args(true, false), &windows(), &machine(), true);

    let report = result.expect("compacts");
    assert_eq!(
        asked.len(),
        2,
        "one question for the deletion, one for the shutdown: {asked:?}"
    );
    assert!(asked[0].contains("16.0 GiB"), "{asked:?}");
    let removed: Vec<_> = lines.iter().filter(|l| l.contains("rm -rf")).collect();
    assert_eq!(removed.len(), 2, "{lines:?}");
    assert!(removed[0].contains("/home/dev/RayX/target"));
    assert_eq!(report.deleted.len(), 2);
    let rm = lines.iter().position(|l| l.contains("rm -rf")).expect("rm");
    let trim = lines
        .iter()
        .position(|l| l.contains("fstrim"))
        .expect("fstrim");
    assert!(rm < trim, "deletion comes before the trim: {lines:?}");
}

#[test]
fn clean_deletes_nothing_the_owner_declines() {
    let mut runner = Runner::record()
        .with_os(Os::Windows)
        .respond(is_status_script, status_output());
    let mut answers = vec![false, true].into_iter();
    let result = compact_with(
        &compact_args(true, false),
        &windows(),
        &mut runner,
        &machine(),
        &|_| Some(GIB),
        &mut |_| answers.next().unwrap_or(false),
        &mut Vec::new(),
    )
    .expect("still compacts");

    assert!(result.deleted.is_empty());
    assert!(!runner.lines().iter().any(|l| l.contains("rm -rf")));
}

#[test]
fn compaction_uses_optimize_vhd_with_a_diskpart_fallback_and_never_sets_sparse() {
    let script = compact_script(&PathBuf::from(VHDX));

    assert!(script.contains("Get-Command Optimize-VHD"));
    assert!(script.contains("Optimize-VHD -Path $vhdx -Mode Full"));
    for step in [
        "select vdisk file=",
        "attach vdisk readonly",
        "compact vdisk",
        "detach vdisk",
    ] {
        assert!(script.contains(step), "{step}");
    }
    // `'a' + $b + 'c', 'd'` binds the comma first and collapses the array into one line: the
    // select line is parenthesised so diskpart gets one command per line.
    assert!(script.contains("$select = ('select vdisk file=\"' + $vhdx + '\"')"));
    assert!(!script.contains("@('select vdisk"));
    // The disk is detached even when compacting fails, and Optimize-VHD failing falls back.
    assert!(script.contains("finally { diskpart /s $detach"));
    assert!(script.contains("-ErrorAction Stop"));
    assert!(!script.contains("sparse"), "WSL 2.6 disabled sparse disks");
    // The path is data: base64 inside the script, never in a quoted string.
    assert!(script.contains(&rayx_cli::wsl::base64(VHDX.as_bytes())));
    assert!(!script.contains(VHDX));
    for hostile in [
        "C:\\o'brien\\ext4.vhdx",
        "C:\\it\u{2019}s\\ext4.vhdx",
        "C:\\a$(calc)\\ext4.vhdx",
    ] {
        let script = compact_script(&PathBuf::from(hostile));
        assert!(
            !script.contains("calc") && !script.contains('\u{2019}') && !script.contains("o'brien"),
            "{script}"
        );
    }
    // A failing diskpart is the script's exit code, not masked by the cleanup after it.
    assert!(script.contains("$code = $LASTEXITCODE"));
    assert!(script.trim_end().ends_with("exit $code"));
}

#[test]
fn the_encoded_command_round_trips() {
    // `powershell -EncodedCommand` takes base64 of the UTF-16LE text.
    let text = "Write-Output 'ok'";
    let encoded = encode_powershell(text);
    assert_eq!(encoded, "VwByAGkAdABlAC0ATwB1AHQAcAB1AHQAIAAnAG8AawAnAA==");
}

#[test]
fn compact_inside_wsl_prints_the_windows_command_and_runs_nothing() {
    let mut host = windows();
    host.os = Os::Linux;
    host.wsl = true;
    let (result, lines, _, _) = compact(&compact_args(false, false), &host, &machine(), true);

    let error = result.expect_err("inside WSL");
    assert_eq!(error, CompactError::InsideWsl);
    assert!(error.to_string().contains("from Windows"));
    assert!(lines.is_empty());
}

#[test]
fn compact_elsewhere_is_refused() {
    let mut host = windows();
    host.os = Os::MacOs;
    let (result, lines, _, _) = compact(&compact_args(false, false), &host, &machine(), true);

    assert_eq!(result.expect_err("not Windows"), CompactError::NotWindows);
    assert!(lines.is_empty());
}

#[test]
fn the_distribution_is_chosen_by_name_or_defaults_to_ubuntu() {
    let two = FakeMachine::new()
        .with_wsl_distributions(vec![
            distribution("Debian", 2, 1000),
            distribution("Ubuntu-24.04", 2, 1000),
        ])
        .with_size(VHDX, GIB);
    let (result, lines, _, _) = compact(&compact_args(false, true), &windows(), &two, true);
    assert_eq!(result.expect("compacts").distribution, "Ubuntu-24.04");
    assert!(lines.iter().any(|l| l.contains("-d Ubuntu-24.04")));

    let named = CompactArgs {
        distro: Some("Debian".into()),
        clean: false,
        yes: true,
    };
    let (result, _, _, _) = compact(&named, &windows(), &two, true);
    assert_eq!(result.expect("compacts").distribution, "Debian");

    let missing = CompactArgs {
        distro: Some("Nope".into()),
        clean: false,
        yes: true,
    };
    let (result, lines, _, _) = compact(&missing, &windows(), &two, true);
    assert!(result.expect_err("unknown").to_string().contains("Nope"));
    assert!(lines.is_empty());

    let one = FakeMachine::new().with_wsl_distributions(vec![distribution("Old", 1, 1000)]);
    let (result, _, _, _) = compact(&compact_args(false, true), &windows(), &one, true);
    assert!(
        result
            .expect_err("no WSL 2")
            .to_string()
            .contains("no WSL 2")
    );
    let named_old = CompactArgs {
        distro: Some("Old".into()),
        clean: false,
        yes: true,
    };
    let (result, _, _, _) = compact(&named_old, &windows(), &one, true);
    assert!(result.expect_err("WSL 1").to_string().contains("WSL 1"));
}

#[test]
fn only_a_build_output_directory_is_ever_deleted() {
    // `du` prints `<bytes><TAB><path>`: a tab in a directory name truncates the path to a prefix.
    for hostile in ["/home/dev/x", "/home/dev/x/src", "/home/dev/x/target/../.."] {
        let mut runner = Runner::record().with_os(Os::Windows).respond(
            is_status_script,
            Outcome::success().with_stdout(format!("OUT\t5\t{hostile}\n")),
        );
        let result = compact_with(
            &compact_args(true, true),
            &windows(),
            &mut runner,
            &machine(),
            &|_| Some(GIB),
            &mut |_| true,
            &mut Vec::new(),
        );
        assert!(result.is_err(), "{hostile}");
        assert!(
            !runner.lines().iter().any(|l| l.contains("rm -rf")),
            "{hostile}"
        );
    }
}

#[test]
fn a_missing_virtual_disk_stops_before_any_side_effect() {
    let mut runner = Runner::record().with_os(Os::Windows);

    let result = compact_with(
        &compact_args(false, true),
        &windows(),
        &mut runner,
        &machine(),
        &|_| None,
        &mut |_| true,
        &mut Vec::new(),
    );

    assert!(
        result
            .expect_err("no such disk")
            .to_string()
            .contains("not found")
    );
    assert!(
        runner.lines().is_empty(),
        "no fstrim and no shutdown: {:?}",
        runner.lines()
    );
}

#[test]
fn a_path_that_is_not_a_vhdx_file_is_refused_before_any_side_effect() {
    let mut odd = distribution("Ubuntu-24.04", 2, 1000);
    odd.base_path = PathBuf::from("C:\\x\u{0007}y");
    let machine = FakeMachine::new().with_wsl_distributions(vec![odd]);
    let mut runner = Runner::record().with_os(Os::Windows);

    let result = compact_with(
        &compact_args(false, true),
        &windows(),
        &mut runner,
        &machine,
        &|_| Some(GIB),
        &mut |_| true,
        &mut Vec::new(),
    );

    assert!(result.is_err());
    assert!(runner.lines().is_empty(), "{:?}", runner.lines());
}
