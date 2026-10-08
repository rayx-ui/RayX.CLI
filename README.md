# rayx

`rayx` is the RayX command-line tool. One prebuilt binary sets up a machine for gpux and RayX
work and builds, runs, packages and tests any app that uses RayX, so a project needs no xtask of
its own.

## Install

macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.ps1 | iex"
```

Until the first release is published, build it from source: `cargo install --locked --git https://github.com/rayx-ui/RayX.CLI --branch feature/initial-setup rayx-cli` (drop `--branch` once the work is merged to `main`).

Release binaries cover Windows x64 and ARM64, Linux x64 and ARM64 (static musl), and macOS ARM64
and x64. Update an installed `rayx` with `rayx self update`.

## Commands

| Command | What it does |
| --- | --- |
| `rayx setup` | Installs everything gpux and RayX development needs on this machine. |
| `rayx doctor` | Reports every requirement on this host with the exact fix. |
| `rayx wsl` | Shows the WSL status and compacts the WSL virtual disk. |
| `rayx self update` | Updates `rayx` to the latest release. |
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

### `rayx doctor`

Reports every requirement of the selected sets as present, missing or the wrong version, with the
version found and the exact fix, plus the GPU adapters (`hardwareAdapter`), the desktop session
(`interactiveDesktop`) and, from Windows, the size of each WSL virtual disk. `--json` prints the
same for tools. It exits 0 whenever the host could be inspected.

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

## Building from source

```bash
cargo build
cargo test --test <group>
```

The package builds from a clean clone with crates.io dependencies only.

## License

Apache-2.0. See [LICENSE-APACHE](LICENSE-APACHE).
