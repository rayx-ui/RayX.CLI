//! The base set on Ubuntu and Debian (also inside WSL): the apt packages gpux and RayX build
//! against, then rustup, the project's toolchain and the cargo bin directory on PATH.

use crate::host::CommandSpec;

use super::{Cx, PlanEnv, Probed, Set, SetupError, Step, rust};

/// The packages for building gpux and RayX natively.
pub const BASE_PACKAGES: &[&str] = &[
    "build-essential",
    "pkg-config",
    "git",
    "curl",
    "jq",
    "cmake",
    "clang",
    "llvm",
    "lld",
    "libclang-dev",
    "libssl-dev",
    "libzstd-dev",
    "libsqlite3-dev",
    "libglib2.0-dev",
    "libxcb1-dev",
    "libxkbcommon-dev",
    "libxkbcommon-x11-dev",
    "libwayland-dev",
    "libfontconfig1-dev",
    "libfreetype-dev",
    "libasound2-dev",
    "libvulkan1",
    "mesa-vulkan-drivers",
    "xdg-utils",
];

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

/// The base steps for the host, or an error naming the supported distributions.
pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    match &env.host.distro {
        Some(distro) if distro.is_supported() => {}
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
            "system packages for building gpux and RayX",
            BASE_PACKAGES,
        ),
        rust::rustup_unix_step(),
        rust::toolchain_step(env),
        rust::cargo_path_step(),
    ])
}
