# rayx

`rayx` is the RayX command-line tool. One prebuilt binary sets up a machine for gpux and RayX
work and builds, runs, packages and tests any app that uses RayX, so a project needs no xtask of
its own.

## Install

Latest release, macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.sh | sh
```

Latest release, Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.ps1 | iex"
```

A pinned version (here 1.0.0) uses the same installers from that release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.0/rayx-cli-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.0/rayx-cli-installer.ps1 | iex"
```

The installer puts `rayx` in `~/.rayx/bin` (`$RAYX_HOME/bin` when `RAYX_HOME` is set), adds that
directory to your `PATH` (open a new shell afterwards) and writes the install receipt that
`rayx self update` reads. Run the installer again, or `rayx self update`, to update in place.

Release binaries cover Windows x64 and ARM64, Linux x64 and ARM64 (static musl), and macOS ARM64
and x64. Every archive carries a GitHub artifact attestation; check one you downloaded with

```bash
gh attestation verify rayx-cli-aarch64-apple-darwin.tar.xz --repo rayx-ui/RayX.CLI
```

If you download an archive by hand in a browser on macOS, remove the quarantine flag with
`xattr -d com.apple.quarantine rayx` (the binaries are not notarized yet); the installers need no
such step. How releases are made, and how to check one, is in [Releasing `rayx`](docs/releasing.md).

## Commands

| Command | What it does |
| --- | --- |
| `rayx setup` | Installs everything gpux and RayX development needs on this machine. |
| `rayx doctor` | Reports every requirement on this host with the exact fix. |
| `rayx wsl` | Shows the WSL status, compacts the WSL virtual disk and removes a distribution `rayx` created. |
| `rayx lima` | Removes a Lima VM `rayx` created (macOS). |
| `rayx self update` | Updates `rayx` to the latest release and offers to refresh this machine and its guests. |
| `rayx app` | Builds, runs, packages, publishes, deploys and tests a RayX app. |
| `rayx fmt` | Formats the project, one workspace member at a time. |

Run `rayx --help` for the options of each command.

### `rayx setup`

`rayx setup` installs the base set; flags add the others, and `--all` adds every set the host
supports. `--check` only reports what is missing (with the exact commands), and a second run
installs nothing. `rayx` never reads or stores a password or token, never relaunches itself
elevated, and accepts licenses only with `--yes`; what needs `sudo` or administrator rights asks
for it once, in your console.

Something that is installed but outdated or unusable (a Node older than the pin, a `wasm-bindgen`
at the wrong version, a Homebrew too old to run on this macOS) is an update, not a first install:
in a terminal `rayx setup` asks `… is outdated or unusable (…). Update it now? [y/N]` before it
touches it. `--yes` updates without asking. Without a terminal and without `--yes` nothing is
updated: the step is reported with the commands that would update it and `rayx setup` exits 1.

| Flag | Installs |
| --- | --- |
| (none) | The base set: system packages (apt on Ubuntu and Debian, Visual Studio Build Tools, LLVM and the Windows SDK on Windows, the Command Line Tools and Homebrew on macOS), `rustup` and the project's Rust toolchain. Windows x64 and ARM64, Ubuntu and Debian x64 and ARM64, and macOS are supported. |
| `--web` | The threaded-WASM nightly with `rust-src`, the `wasm32` targets and `wasm-bindgen-cli` at the version `Cargo.lock` locks. |
| `--test` | Node.js, pnpm (through corepack), the Linux libraries headed Chromium needs, and the project's Playwright package with its Chromium. |
| `--android` | JDK 21 and `JAVA_HOME`, Android Studio, the SDK with its command-line tools, platform, build tools and NDK, the emulator system image and a default AVD, emulator acceleration, `cargo-ndk` and the Rust Android targets. |
| `--ios` | (macOS) Xcode through `xcodes`, its command line tools, license and first launch, the Simulator runtime, XcodeGen and the iOS targets. |
| `--gpu` | The GPU driver for every adapter found. winget has no NVIDIA or AMD display driver package, so on Windows this uses Windows Update and says what it did; Ubuntu uses `ubuntu-drivers` for NVIDIA and Mesa for AMD and Intel. |
| `--wsl` | From Windows: WSL 2 with `Ubuntu-24.04`, the Linux `rayx` of the same version in it, and the same `rayx setup` run inside it. `--clone [<dir>]` also clones the current checkout (only that one) into the distribution. |
| `--lima` | From macOS: the Lima VM `rayx` with Ubuntu 24.04, the Linux `rayx` of the same version in it, and the same `rayx setup` run inside it; see [`rayx setup --lima`](#rayx-setup---lima). Never part of `--all`. |
| `--update` | Re-runs the setup recorded on this machine and refreshes its guests; see [`rayx setup --update`](#records-rayx-setup---update-and-the-refresh). |

### `rayx setup --lima`

On macOS, `rayx setup --lima [--clone [<dir>]] [set flags]` makes a Linux guest the way `--wsl`
does on Windows. It needs macOS 13 or later and Homebrew (run `rayx setup` first if you have none),
installs Lima with `brew install lima` when it is missing (an older Lima is an update and is asked
about), and creates **one** VM named `rayx`: `vz`, the Ubuntu 24.04 template, 6 CPUs, 12 GiB memory
and a 100 GiB sparse disk, with your macOS home mounted **read-only** and Lima's container runtime
off. Then it puts the Linux `rayx` of the same version inside (the release installer, so `rayx self
update` works there too) and runs `rayx setup` with the same set flags inside. `--check` prints
every command, including the `limactl create` line, and creates nothing. `--all` does not imply
`--lima`: a VM and its 620 MB image are not a requirement of the host. No step needs `sudo`.

Sizes are capped to what the Mac can spare: all CPUs but two, and half the memory (a 16 GiB Mac
gets 8 GiB, and `rayx` says so). In a terminal, before it creates the VM, `rayx` prints the
effective settings and asks once `Create Lima VM rayx: … Customize? [y/N]`; `y` asks for CPUs,
memory, disk and whether the home is writable. Unattended runs use `--vm-cpus <n>`,
`--vm-memory <GiB>`, `--vm-disk <GiB>` and `--vm-home-writable` (values above the caps, or a disk
under 30 GiB, are refused before anything runs); `--yes` and a missing terminal ask nothing. A VM
needs 10 GiB free on the volume Lima uses, and `rayx` warns when the running VMs plus the new one
would take more than three quarters of the memory.

The home is read-only on purpose, so the VM cannot damage your macOS files. That is why a
checkout is built inside the VM from its own copy: `--clone [<dir>]` clones **the current checkout
only** (default `~/<checkout name>`; it must be under your home, which the VM mounts) into the VM,
and Cargo fetches every dependency there. Run `rayx setup --lima --clone` again to fetch new
commits.

A re-run without `--vm-*` flags asks nothing about the VM and changes nothing. `--vm-*` flags that
differ from the existing VM are an update: `rayx` lists it (`4 -> 6 CPUs`), asks, and runs
`limactl stop`, `limactl edit` and `limactl start`; a smaller disk is refused because Lima cannot
shrink one. VMs `rayx` did not create (your own `gpux`, say) are never started, stopped, edited,
updated or removed.

### Records, `rayx setup --update` and the refresh

Every `rayx setup` run records on the machine it ran on which sets it installed and in which
project, in `setup.json` next to `guests.json` in `$XDG_CONFIG_HOME/rayx` (`~/.config/rayx`), or
`%LOCALAPPDATA%\rayx` on Windows. `setup --wsl` and `--lima` also record the guests they manage in
`guests.json`: kind, name, Ubuntu version, set flags, clones and VM settings. The records hold
names, versions, flags, paths and sizes, nothing secret, and `--check` records nothing.

`rayx setup --update [--yes] [--check]` re-runs setup for the recorded sets once per recorded
project (in that project, so its pins apply; a project that no longer exists is reported and
dropped), then refreshes every current guest of this host: it asks `Update lima guest rayx? [y/N]`
(no question with `--yes`, and no terminal without `--yes` means not updated and exit 1), starts a
stopped VM and stops it again afterwards, puts the Linux `rayx` of the running version inside, runs
`sudo apt-get update` and `sudo apt-get upgrade`, and runs `rayx setup --update` inside. A failing
guest is reported and the next still runs. `apt-get upgrade` runs only inside guests `rayx`
manages, never on the host. `--check` inspects running guests (a stopped VM is not started) and
changes nothing.

After `rayx self update` installed a newer release and a record exists, it asks `Refresh this
machine's setup and guests with rayx <new>? [y/N]`; yes, or `rayx self update --yes`, runs the new
binary's `setup --update`, and its exit code is the command's. Otherwise it prints
``run `rayx setup --update` to refresh the setup and guests``.

### Ubuntu versions and replacing a guest

All guests use the one Ubuntu pin (24.04, built in: a guest serves every project on the machine).
A guest older than the pin is never upgraded in place. `rayx setup --lima`, `--wsl` and
`--update` list the clones in it that hold uncommitted work or commits on no remote, say the old
guest is kept, and ask `Create <new> with Ubuntu 24.04 next to <old>? [y/N]`. On yes, a Lima VM is
renamed `rayx-<old version>` and a new `rayx` is created from the pinned image with the same size
and home mount; on WSL `Ubuntu-24.04` is installed next to the old distribution. Either way the
Linux `rayx`, the recorded clones (copied again from their checkouts) and the recorded setup flags
are restored in the new guest, and the old one is kept, retired, until you remove it. An
interrupted migration continues on the next run. A guest newer than the pin is reported and its
release left alone.

### `rayx lima` and removing guests

- `rayx lima remove <name> [--yes]` (macOS) runs `limactl delete --force <name>`.
- `rayx wsl remove <name> [--yes]` (Windows) runs `wsl --unregister <name>`.

Both work only for guests in `guests.json` (a name `rayx` did not create is refused with how to
remove it yourself), list the clones in the guest that hold unpushed work, ask `Remove <name> and
everything in it? [y/N]` (without a terminal and without `--yes` they refuse), and drop the guest
from the record afterwards. Removing the current guest says that `rayx setup --lima` or `--wsl`
creates it again; `rayx doctor` names the removal command for every retired guest.

### `rayx doctor`

Reports every requirement of the selected sets as present, missing or the wrong version, with the
version found and the exact fix, plus the GPU adapters (`hardwareAdapter`), the desktop session
(`interactiveDesktop`) and, from Windows, the size of each WSL virtual disk. On macOS it reports
Lima (the `limactl` version, or missing with `rayx setup --lima`), and on macOS and Windows every
guest `rayx` manages: state (running, stopped, missing), CPUs, memory and disk, whether the home is
read-only or writable, the Ubuntu version, whether the guest is current, outdated or retired, and
for a retired or missing one the command that removes it. `--json` prints the same for tools, with
a `guests` array. Doctor never starts a VM and exits 0 whenever the host could be inspected.

### `rayx wsl`

- `rayx wsl status [--json]` lists the WSL distributions with their default user, the size of each
  `ext4.vhdx` on the Windows disk, the space used and free inside, and the size of the build output
  (`target/`, `artifacts/`, `artifacts-temp/`) under the checkouts `setup --wsl --clone` made.
- `rayx wsl compact [--distro <name>] [--clean] [--yes]` gives unused space back to Windows: it
  optionally deletes that build output, runs `fstrim` inside the distribution, asks before
  `wsl --shutdown` (which stops every distribution, including VS Code remote sessions), and
  compacts the virtual disk with `Optimize-VHD` (or `diskpart`) in one elevated child, reporting
  the size before and after. It never enables sparse disks, which WSL 2.6 disabled because they can
  corrupt a distribution.
- `rayx wsl remove <name> [--yes]` unregisters a distribution `rayx` created; see
  [`rayx lima` and removing guests](#rayx-lima-and-removing-guests).

### `rayx app` and `rayx fmt`

See [Using `rayx app` in a project](docs/app-project.md) for the grammar, the project contract, the
prerequisite step, the tests and the Playwright package template.

### Versions and where they come from

Every version `rayx` needs is resolved in this order: the project (`Cargo.lock` for
`wasm-bindgen`, `rust-toolchain.toml` for Rust, `[workspace.metadata.rayx]` for the rest), then,
inside a gpux checkout, gpux's own Android profile, then the built-in default. `rayx doctor` shows
each pin with its source.

| Pin | Default | Project key |
| --- | --- | --- |
| threaded-WASM nightly | `nightly-2026-06-23` | `web-toolchain` |
| Node.js | 22 | `node` |
| JDK | 21 | `jdk` |
| Playwright | 1.59.1 | `playwright` |
| Android platform | `android-34` (`android-36` in a gpux checkout) | `android-platform` |
| Android NDK | 28.0.12674087 | `android-ndk` |
| NDK API level | 31 | `android-ndk-api` |
| Android build tools | none (`36.0.0` in a gpux checkout) | `android-build-tools` |
| Emulator system image | `<platform>;google_apis` | `android-system-image` |
| Xcode | the newest stable | `xcode` |
| Guest Ubuntu (WSL and Lima) | 24.04 | none: built in, since a guest serves every project |

## Building from source

```bash
cargo build
cargo test --test <group>
```

The package builds from a clean clone with crates.io dependencies only.

## License

Apache-2.0. See [LICENSE-APACHE](LICENSE-APACHE).
