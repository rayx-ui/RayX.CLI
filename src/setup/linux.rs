//! The base set on Ubuntu and Debian (also inside WSL): the apt packages NSH builds and runs
//! against, then rustup, the project's toolchain and the cargo bin directory on PATH.

use crate::host::{CommandSpec, Distro};

use super::{Cx, PlanEnv, Probed, Set, SetupError, Step, rust};

/// The packages NSH's Linux runtime builds and runs with: a C toolchain and libclang (QuickJS,
/// zstd, ring), LLVM for the wasm32 C builds, GTK 4, WebKitGTK 6.0 for web views and GStreamer's
/// plugins for video. No X11, Wayland or Vulkan library: NSH draws through GTK.
pub const BASE_PACKAGES: &[&str] = &[
    "build-essential",
    "pkg-config",
    "git",
    "curl",
    "clang",
    "llvm",
    "libclang-dev",
    "libgtk-4-dev",
    "libwebkitgtk-6.0-dev",
    "gstreamer1.0-plugins-base",
    "gstreamer1.0-plugins-good",
];

/// The oldest Ubuntu with GTK 4.14 and WebKitGTK 6.0.
pub const OLDEST_UBUNTU: (u32, u32) = (24, 4);

/// The oldest Debian with GTK 4.14 and WebKitGTK 6.0.
pub const OLDEST_DEBIAN: u32 = 13;

/// Probes apt packages with one `dpkg-query` call: `missing` lists the ones not installed.
pub fn probe_packages(cx: &mut Cx, packages: &[String]) -> Probed {
    let spec = CommandSpec::new("dpkg-query")
        .args(["-W", "-f=${Package}\t${db:Status-Abbrev}\n"])
        .args(packages.iter().cloned());
    let installed: Vec<String> = cx
        .query(&spec)
        .map(|outcome| {
            outcome
                .stdout
                .lines()
                .filter_map(|line| {
                    let (name, status) = line.split_once('\t')?;
                    // `Status-Abbrev` is three characters: want, status, error. Only status `i`
                    // is installed (`iU` is unpacked, `hi` is a held, installed package).
                    (status.chars().nth(1) == Some('i')).then(|| name.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    Probed::missing_items(
        packages
            .iter()
            .filter(|package| !installed.iter().any(|name| name == *package))
            .cloned()
            .collect(),
    )
}

/// Why `distro` is too old for NSH, if it is: GTK 4.14 and WebKitGTK 6.0 come with Ubuntu 24.04
/// and Debian 13. Debian's testing and unstable have no `VERSION_ID` and are new enough; a
/// derivative has its own numbering, so its packages decide. Inside WSL (`wsl`) the fix is a
/// newer distribution set up from Windows.
pub fn too_old(distro: &Distro, wsl: bool) -> Option<String> {
    let numbers: Vec<u32> = distro
        .version
        .split('.')
        .map_while(|part| part.parse().ok())
        .collect();
    let old = match (distro.id.as_str(), numbers.as_slice()) {
        ("ubuntu", [major, minor, ..]) => (*major, *minor) < OLDEST_UBUNTU,
        ("ubuntu", [major]) => (*major, 0) < OLDEST_UBUNTU,
        ("debian", [major, ..]) => *major < OLDEST_DEBIAN,
        _ => false,
    };
    let fix = if wsl {
        "Run `rayx setup --wsl` on Windows: it sets up a WSL distribution with Ubuntu 24.04, where \
         this setup then runs."
    } else {
        "Upgrade to Ubuntu 24.04 or Debian 13, or later, and run `rayx setup` again."
    };
    old.then(|| {
        format!(
            "NSH needs GTK 4.14 and WebKitGTK 6.0, which come with Ubuntu 24.04 and Debian 13 or \
             later, but this is {}. {fix}",
            distro.name
        )
    })
}

/// The base steps for the host, or an error naming the supported distributions.
pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    match &env.host.distro {
        Some(distro) if distro.is_supported() => {
            if let Some(message) = too_old(distro, env.host.wsl) {
                return Err(SetupError::Unsupported(message));
            }
        }
        other => {
            let found = other
                .as_ref()
                .map_or("an unknown Linux distribution".to_string(), |d| {
                    d.name.clone()
                });
            return Err(SetupError::Unsupported(format!(
                "`rayx setup` supports Ubuntu and Debian on Linux, but this is {found}. \
                 Install the equivalents of these packages yourself, then run `rayx doctor`: {}",
                BASE_PACKAGES.join(" ")
            )));
        }
    }
    Ok(vec![
        Step::apt(
            "apt-packages",
            Set::Base,
            "system packages for building NSH apps",
            BASE_PACKAGES,
        ),
        rust::rustup_unix_step(),
        rust::toolchain_step(env),
        rust::cargo_path_step(),
    ])
}
