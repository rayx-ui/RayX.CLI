//! WSL as seen from Windows: the installed distributions and their virtual disks.
//!
//! The registry view is the source of truth: each distribution is a subkey of
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss` whose `BasePath` holds its `ext4.vhdx`.
//! `rayx wsl status` and `rayx wsl compact` build on this module; `doctor` reports the sizes.

use std::path::{Path, PathBuf};

/// A WSL distribution registered for the current user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    /// The registry key name, a GUID in braces.
    pub id: String,
    /// `DistributionName`, for example `Ubuntu-24.04`.
    pub name: String,
    /// `BasePath`: the directory that holds the distribution's virtual disk.
    pub base_path: PathBuf,
    /// WSL version, 1 or 2.
    pub version: Option<u32>,
    /// `DefaultUid`: the user the distribution starts as (1000 is the first created user).
    pub default_uid: Option<u32>,
}

impl Distribution {
    /// The virtual disk file of a WSL 2 distribution.
    pub fn vhdx(&self) -> PathBuf {
        let base = self.base_path.to_string_lossy();
        // The registry stores `\\?\C:\...`; the prefix is valid for file APIs, but plain paths
        // read better and compare equal to what the user types.
        let plain = base.strip_prefix(r"\\?\").unwrap_or(&base);
        // A Windows path whatever system formats it, so the same text is reported everywhere.
        PathBuf::from(format!("{}\\ext4.vhdx", plain.trim_end_matches('\\')))
    }
}

/// The distributions registered in the current user's registry; empty off Windows.
pub fn registry_distributions() -> Vec<Distribution> {
    #[cfg(windows)]
    {
        registry::distributions()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
mod registry {
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, FILETIME};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, REG_DWORD, REG_SZ,
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW,
    };

    use super::Distribution;

    const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// An open registry key, closed on drop.
    struct Key(HKEY);

    impl Key {
        fn open(parent: HKEY, path: &str, access: u32) -> Option<Key> {
            let mut key: HKEY = std::ptr::null_mut();
            let path = wide(path);
            // SAFETY: `path` is NUL-terminated and `key` is a live out-pointer.
            let status = unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, access, &mut key) };
            (status == ERROR_SUCCESS).then_some(Key(key))
        }

        fn string(&self, name: &str) -> Option<String> {
            let (kind, data) = self.value(name)?;
            if kind != REG_SZ {
                return None;
            }
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|&unit| unit != 0)
                .collect();
            Some(String::from_utf16_lossy(&units))
        }

        fn dword(&self, name: &str) -> Option<u32> {
            let (kind, data) = self.value(name)?;
            (kind == REG_DWORD && data.len() >= 4)
                .then(|| u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }

        fn value(&self, name: &str) -> Option<(u32, Vec<u8>)> {
            let name = wide(name);
            let mut kind = 0u32;
            let mut size = 0u32;
            // SAFETY: a null data pointer asks only for the type and the size in bytes.
            let status = unsafe {
                RegQueryValueExW(
                    self.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    std::ptr::null_mut(),
                    &mut size,
                )
            };
            if status != ERROR_SUCCESS {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            // SAFETY: `data` holds `size` writable bytes.
            let status = unsafe {
                RegQueryValueExW(
                    self.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut size,
                )
            };
            (status == ERROR_SUCCESS).then(|| {
                data.truncate(size as usize);
                (kind, data)
            })
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from `RegOpenKeyExW` and is closed exactly once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    pub fn distributions() -> Vec<Distribution> {
        let Some(lxss) = Key::open(
            HKEY_CURRENT_USER,
            LXSS,
            KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS,
        ) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut length = name.len() as u32;
            let mut last_write = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            // SAFETY: `name` holds `length` UTF-16 units and the other out-pointers are null or live.
            let status = unsafe {
                RegEnumKeyExW(
                    lxss.0,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut last_write,
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let id = String::from_utf16_lossy(&name[..length as usize]);
            let Some(key) = Key::open(lxss.0, &id, KEY_QUERY_VALUE) else {
                continue;
            };
            let (Some(distro), Some(base)) =
                (key.string("DistributionName"), key.string("BasePath"))
            else {
                continue;
            };
            found.push(Distribution {
                id,
                name: distro,
                base_path: PathBuf::from(base),
                version: key.dword("Version"),
                default_uid: key.dword("DefaultUid"),
            });
        }
        found
    }
}

// ---------------------------------------------------------------------------------------------
// Status and compaction

use std::io::Write;

use serde_json::json;

use crate::cli::CompactArgs;
use crate::host::{CommandSpec, HostFacts, Os, Outcome, Runner};
use crate::setup::Machine;
use crate::setup::wsl::DISTRIBUTION;

/// Where `setup --wsl --clone` records the directories it created, one per line, inside the
/// distribution.
pub const CLONES_FILE: &str = "$HOME/.config/rayx/wsl-clones";

/// The build output directories of a checkout.
pub const BUILD_OUTPUT: [&str; 3] = ["target", "artifacts", "artifacts-temp"];

/// A build output directory inside a distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildOutput {
    /// The Linux path.
    pub path: String,
    pub bytes: u64,
}

/// One distribution as `rayx wsl status` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistributionStatus {
    pub name: String,
    pub version: Option<u32>,
    pub default_uid: Option<u32>,
    pub vhdx: PathBuf,
    /// The size of `ext4.vhdx` on the Windows disk.
    pub vhdx_bytes: Option<u64>,
    /// Space used and free inside the distribution (`df /`); `None` when it could not be read.
    pub used_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub build_output: Vec<BuildOutput>,
}

impl DistributionStatus {
    /// The total size of the build output under the cloned checkouts.
    pub fn build_output_bytes(&self) -> u64 {
        self.build_output.iter().map(|output| output.bytes).sum()
    }
}

/// The script that prints `DF<TAB><used><TAB><free>` and one `OUT<TAB><bytes><TAB><path>` line per
/// existing build output directory under the recorded clones.
pub fn status_script() -> String {
    format!(
        "printf 'DF\\t'; df -B1 --output=used,avail / | tail -n1 | awk '{{print $1\"\\t\"$2}}'; \
         f=\"{CLONES_FILE}\"; if [ -f \"$f\" ]; then while IFS= read -r d; do \
         for o in {outputs}; do if [ -d \"$d/$o\" ]; then printf 'OUT\\t'; du -sb \"$d/$o\"; fi; done; \
         done < \"$f\"; fi",
        outputs = BUILD_OUTPUT.join(" ")
    )
}

/// Reads the output of [`status_script`]: used bytes, free bytes and the build output.
pub fn parse_status(text: &str) -> (Option<u64>, Option<u64>, Vec<BuildOutput>) {
    let (mut used, mut free, mut outputs) = (None, None, Vec::new());
    for line in text.lines() {
        let mut fields = line.trim_end().split('\t');
        match fields.next() {
            Some("DF") => {
                used = fields.next().and_then(|v| v.trim().parse().ok());
                free = fields.next().and_then(|v| v.trim().parse().ok());
            }
            Some("OUT") => {
                if let (Some(bytes), Some(path)) = (
                    fields.next().and_then(|v| v.trim().parse().ok()),
                    fields.next().filter(|path| path.starts_with('/')),
                ) {
                    outputs.push(BuildOutput {
                        path: path.to_string(),
                        bytes,
                    });
                }
            }
            _ => {}
        }
    }
    (used, free, outputs)
}

fn in_distribution(name: &str, script: &str) -> CommandSpec {
    CommandSpec::new("wsl")
        .args(["-d", name, "--exec", "bash", "-lc"])
        .arg(script)
}

/// The status of every distribution WSL knows.
pub fn status(runner: &mut Runner, machine: &dyn Machine) -> Vec<DistributionStatus> {
    machine
        .wsl_distributions()
        .into_iter()
        .map(|distribution| {
            let vhdx = distribution.vhdx();
            let (used_bytes, free_bytes, build_output) = if distribution.version == Some(1) {
                (None, None, Vec::new())
            } else {
                runner
                    .query(&in_distribution(&distribution.name, &status_script()))
                    .ok()
                    .filter(Outcome::is_success)
                    .map(|outcome| parse_status(&outcome.stdout))
                    .unwrap_or((None, None, Vec::new()))
            };
            DistributionStatus {
                vhdx_bytes: machine.file_size(&vhdx),
                name: distribution.name,
                version: distribution.version,
                default_uid: distribution.default_uid,
                vhdx,
                used_bytes,
                free_bytes,
                build_output,
            }
        })
        .collect()
}

/// A size as `12.3 GiB`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn size_or_unknown(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "unknown".to_string(), human)
}

/// The text of `rayx wsl status`.
pub fn render_status(distributions: &[DistributionStatus]) -> String {
    if distributions.is_empty() {
        return "No WSL distributions are installed.\n".to_string();
    }
    let mut out = String::new();
    for d in distributions {
        out.push_str(&format!(
            "{} (WSL {}, default user {})\n",
            d.name,
            d.version.map_or("?".to_string(), |v| v.to_string()),
            match d.default_uid {
                Some(0) => "root: none created yet".to_string(),
                Some(uid) => format!("uid {uid}"),
                None => "unknown".to_string(),
            }
        ));
        out.push_str(&format!(
            "  virtual disk  {} ({} on the Windows disk)\n",
            d.vhdx.display(),
            size_or_unknown(d.vhdx_bytes)
        ));
        out.push_str(&format!(
            "  inside        {} used, {} free\n",
            size_or_unknown(d.used_bytes),
            size_or_unknown(d.free_bytes)
        ));
        if d.build_output.is_empty() {
            out.push_str("  build output  none under the cloned checkouts\n");
        } else {
            out.push_str(&format!(
                "  build output  {} under the cloned checkouts\n",
                human(d.build_output_bytes())
            ));
            for output in &d.build_output {
                out.push_str(&format!("    {}  {}\n", human(output.bytes), output.path));
            }
        }
    }
    out
}

/// The JSON of `rayx wsl status --json`.
pub fn status_json(distributions: &[DistributionStatus]) -> serde_json::Value {
    json!({
        "distributions": distributions.iter().map(|d| json!({
            "name": d.name,
            "wslVersion": d.version,
            "defaultUid": d.default_uid,
            "vhdx": d.vhdx.display().to_string(),
            "vhdxBytes": d.vhdx_bytes,
            "usedBytes": d.used_bytes,
            "freeBytes": d.free_bytes,
            "buildOutputBytes": d.build_output_bytes(),
            "buildOutput": d.build_output.iter().map(|o| json!({
                "path": o.path,
                "bytes": o.bytes,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Runs `rayx wsl status` and returns the exit code.
pub fn run_status(json: bool) -> u8 {
    let host = crate::host::facts();
    if host.os != Os::Windows {
        eprintln!("rayx: `wsl status` reports WSL from Windows; there is no WSL here");
        return 1;
    }
    let distributions = status(&mut Runner::execute(), &crate::setup::SystemMachine);
    if json {
        println!("{}", status_json(&distributions));
    } else {
        print!("{}", render_status(&distributions));
    }
    0
}

/// What `rayx wsl compact` stopped on.
#[derive(Debug, PartialEq, Eq)]
pub enum CompactError {
    NotWindows,
    /// Run inside WSL: the Windows command to run instead.
    InsideWsl,
    NoDistribution(String),
    Declined(String),
    Failed(String),
}

impl std::fmt::Display for CompactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompactError::NotWindows => {
                f.write_str("`wsl compact` gives WSL disk space back to Windows; run it on Windows")
            }
            CompactError::InsideWsl => f.write_str(
                "run `rayx wsl compact` from Windows (PowerShell): the virtual disk can only be \
                 compacted from outside WSL",
            ),
            CompactError::NoDistribution(message)
            | CompactError::Declined(message)
            | CompactError::Failed(message) => f.write_str(message),
        }
    }
}

/// What a compaction did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactReport {
    pub distribution: String,
    pub vhdx: PathBuf,
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub deleted: Vec<BuildOutput>,
}

impl CompactReport {
    pub fn render(&self) -> String {
        let freed = match (self.before, self.after) {
            (Some(before), Some(after)) if before >= after => {
                format!(", {} given back", human(before - after))
            }
            _ => String::new(),
        };
        format!(
            "{}: virtual disk {} before, {} after{freed}\n",
            self.distribution,
            size_or_unknown(self.before),
            size_or_unknown(self.after)
        )
    }
}

/// UTF-16LE base64, the form `powershell -EncodedCommand` takes.
pub fn encode_powershell(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(ALPHABET[(n >> 18) as usize & 63] as char);
        encoded.push(ALPHABET[(n >> 12) as usize & 63] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    encoded
}

/// The PowerShell that compacts a virtual disk: `Optimize-VHD` when the Hyper-V module is there,
/// otherwise a `diskpart` script (`select vdisk`, `attach vdisk readonly`, `compact vdisk`,
/// `detach vdisk`). It never touches the sparse-disk setting.
pub fn compact_script(vhdx: &Path) -> String {
    // The path goes in as base64 data, never as code: PowerShell ends a single-quoted string at
    // typographic quotes too, and the registry value it comes from is writable without rights.
    let path = base64(vhdx.display().to_string().as_bytes());
    format!(
        "$ProgressPreference = 'SilentlyContinue'\n\
         $vhdx = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{path}'))\n\
         $code = 0\n\
         $done = $false\n\
         if (Get-Command Optimize-VHD -ErrorAction SilentlyContinue) {{\n\
         \x20 try {{ Optimize-VHD -Path $vhdx -Mode Full -ErrorAction Stop; $done = $true }} catch {{ Write-Warning ('Optimize-VHD failed, using diskpart: ' + $_) }}\n\
         }}\n\
         if (-not $done) {{\n\
         \x20 $attach = New-TemporaryFile\n\
         \x20 $detach = New-TemporaryFile\n\
         \x20 $select = ('select vdisk file=\"' + $vhdx + '\"')\n\
         \x20 Set-Content -Path $attach -Value @($select, 'attach vdisk readonly', 'compact vdisk', 'exit')\n\
         \x20 Set-Content -Path $detach -Value @($select, 'detach vdisk', 'exit')\n\
         \x20 try {{ diskpart /s $attach; $code = $LASTEXITCODE }} finally {{ diskpart /s $detach; Remove-Item $attach, $detach -ErrorAction SilentlyContinue }}\n\
         }}\n\
         exit $code\n"
    )
}

fn compact_command(vhdx: &Path) -> CommandSpec {
    CommandSpec::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encode_powershell(&compact_script(vhdx)),
        ])
        .admin()
}

/// Runs `rayx wsl compact` against the real machine and returns the exit code.
pub fn run_compact(args: &CompactArgs) -> u8 {
    use std::io::BufRead;

    let host = crate::host::facts();
    let machine = crate::setup::SystemMachine;
    let mut runner = Runner::execute();
    let size_of = |path: &Path| std::fs::metadata(path).ok().map(|meta| meta.len());
    let mut confirm = |question: &str| -> bool {
        print!("{question} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).is_ok()
            && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    };
    match compact_with(
        args,
        &host,
        &mut runner,
        &machine,
        &size_of,
        &mut confirm,
        &mut std::io::stdout(),
    ) {
        Ok(report) => {
            print!("{}", report.render());
            0
        }
        Err(CompactError::InsideWsl) => {
            println!("{}", CompactError::InsideWsl);
            0
        }
        Err(error) => {
            eprintln!("rayx: {error}");
            1
        }
    }
}

/// The body of `rayx wsl compact` over injectable collaborators.
pub fn compact_with(
    args: &CompactArgs,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    size_of: &dyn Fn(&Path) -> Option<u64>,
    confirm: &mut dyn FnMut(&str) -> bool,
    out: &mut dyn Write,
) -> Result<CompactReport, CompactError> {
    if host.wsl {
        return Err(CompactError::InsideWsl);
    }
    if host.os != Os::Windows {
        return Err(CompactError::NotWindows);
    }
    let distributions = machine.wsl_distributions();
    let distribution = match &args.distro {
        Some(name) => distributions.iter().find(|d| &d.name == name),
        None => distributions
            .iter()
            .find(|d| d.name == DISTRIBUTION)
            .or_else(|| distributions.iter().find(|d| d.version != Some(1))),
    }
    .cloned()
    .ok_or_else(|| {
        CompactError::NoDistribution(match &args.distro {
            Some(name) => format!("no WSL distribution is named {name}"),
            None => "no WSL 2 distribution is installed".to_string(),
        })
    })?;
    if distribution.version == Some(1) {
        return Err(CompactError::NoDistribution(format!(
            "{} is a WSL 1 distribution and has no virtual disk",
            distribution.name
        )));
    }
    let name = distribution.name.clone();
    let vhdx = distribution.vhdx();
    // Refuse anything that is not an existing virtual disk file before the first side effect: the
    // shutdown cannot be undone and the path comes from a registry value.
    let vhdx_text = vhdx.to_string_lossy().to_ascii_lowercase();
    if !vhdx_text.ends_with(".vhdx") || vhdx_text.chars().any(char::is_control) {
        return Err(CompactError::Failed(format!(
            "{} does not look like a virtual disk (.vhdx) file",
            vhdx.display()
        )));
    }
    let before = size_of(&vhdx);
    if before.is_none() {
        return Err(CompactError::Failed(format!(
            "the virtual disk {} was not found",
            vhdx.display()
        )));
    }

    let mut deleted = Vec::new();
    if args.clean {
        let (_, _, outputs) = runner
            .query(&in_distribution(&name, &status_script()))
            .ok()
            .filter(Outcome::is_success)
            .map(|outcome| parse_status(&outcome.stdout))
            .unwrap_or((None, None, Vec::new()));
        if outputs.is_empty() {
            let _ = writeln!(out, "No build output under the cloned checkouts.");
        } else {
            let _ = writeln!(out, "Build output under the cloned checkouts:");
            for output in &outputs {
                let _ = writeln!(out, "  {}  {}", human(output.bytes), output.path);
            }
            let total: u64 = outputs.iter().map(|o| o.bytes).sum();
            if args.yes || confirm(&format!("Delete these ({})?", human(total))) {
                for output in &outputs {
                    let last = output.path.rsplit('/').next().unwrap_or("");
                    if output.path.contains('\'')
                        || output.path.chars().any(char::is_control)
                        || !BUILD_OUTPUT.contains(&last)
                    {
                        return Err(CompactError::Failed(format!(
                            "not deleting {}: it is not a plain build output directory",
                            output.path
                        )));
                    }
                    runner
                        .run_checked(&in_distribution(
                            &name,
                            &format!("rm -rf -- '{}'", output.path),
                        ))
                        .map_err(|error| {
                            CompactError::Failed(format!("deleting {}: {error}", output.path))
                        })?;
                }
                deleted = outputs;
            }
        }
    }

    let _ = writeln!(out, "==> wsl: trimming the file system inside {name}");
    runner
        .run_checked(
            &CommandSpec::new("wsl")
                .args(["-d", &name, "--exec", "sudo", "fstrim", "-av"])
                .interactive(),
        )
        .map_err(|error| CompactError::Failed(format!("fstrim: {error}")))?;

    if !args.yes
        && !confirm(
            "`wsl --shutdown` stops every WSL distribution, including VS Code remote sessions. \
             Continue?",
        )
    {
        return Err(CompactError::Declined(
            "stopped before `wsl --shutdown`; nothing was compacted".to_string(),
        ));
    }
    runner
        .run_checked(&CommandSpec::new("wsl").arg("--shutdown"))
        .map_err(|error| CompactError::Failed(format!("wsl --shutdown: {error}")))?;

    let _ = writeln!(out, "==> wsl: compacting {}", vhdx.display());
    runner
        .run_checked(&compact_command(&vhdx).interactive())
        .map_err(|error| CompactError::Failed(format!("compacting the virtual disk: {error}")))?;

    Ok(CompactReport {
        distribution: name,
        after: size_of(&vhdx),
        vhdx,
        before,
        deleted,
    })
}
