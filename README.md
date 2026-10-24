# rayx

`rayx` is the RayX command-line tool for [Native Shell (NSH)](https://github.com/dXnSix/nsh) apps.
One prebuilt binary sets up a machine for NSH work and runs any NSH app through the `nsh` its
project pins, so a project needs no tooling of its own to find, build or install nsh.

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

An installed `rayx` can switch release, in either direction, with `rayx self update --release
1.0.1` (see [Choosing a release](#choosing-a-release-and-going-back)).

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

## A new CLI, and the old one

This `rayx` is a new CLI with its own history, starting at 1.0.0. The previous `rayx`, for GPUI and
gpux apps, lives on as [`rayx-ui/RayX.CLI.Old`](https://github.com/rayx-ui/RayX.CLI.Old); its
releases (up to 1.4.1) stay there.

- **The machine side carried over:** `setup`, `doctor`, the WSL and Lima guests, `self update` and
  `fmt` work as before, without the GPU set (NSH draws with the system's own graphics stack).
- **The app side is new:** `rayx app` runs nsh's own commands (`rayx app run --target web`), so
  the old grammar (`rayx app <dir> build wasm`) is gone, and GPUI apps stay on the old CLI.
- **The installer replaces the old binary:** both install `~/.rayx/bin/rayx`. A machine that still
  builds GPUI apps keeps the old `rayx` by not installing this one. The old `rayx self update` does
  not move to this CLI by itself (it sees 1.0.0 as older than 1.4.1); run this installer instead.
- **Records and guests carry over:** `setup.json` and `guests.json` stay where the old CLI keeps
  them, in the same format, so `rayx setup --update` re-runs the recorded setups and refreshes the
  WSL and Lima guests the old CLI created, putting this `rayx` into them.

## Commands

| Command | What it does |
| --- | --- |
| `rayx setup` | Installs everything NSH development needs on this machine, including the project's nsh. |
| `rayx doctor` | Reports every requirement on this host with the exact fix, the pins and the guests. |
| `rayx wsl` | Shows the WSL status, compacts the WSL virtual disk and removes a distribution `rayx` created. |
| `rayx lima` | Removes a Lima VM `rayx` created (macOS). |
| `rayx self update` | Updates `rayx` to the latest release, or installs the release `--release` names, and offers to refresh this machine and its guests. |
| `rayx fmt` | Formats the project, one workspace member at a time. |
| `rayx app` | Runs an nsh command with the project's pinned nsh. |
| `rayx nsh` | Shows, installs, locates and prunes the nsh builds `rayx` keeps. |
| `rayx new` | Creates a new NSH app pinned to `rayx`'s default nsh. |
| `rayx serve` | Serves a packaged web app on `127.0.0.1`. |

Run `rayx --help`, or `rayx <command> --help`, for the options of each command.

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
| (none) | The base set: system packages (NSH's build and GTK 4 packages through apt on Ubuntu 24.04 and Debian 13 or later, Visual Studio Build Tools, LLVM and the WebView2 runtime check on Windows, the Command Line Tools and Homebrew on macOS), `rustup`, the project's Rust toolchain, and last the project's nsh (see [How `rayx` chooses and installs nsh](#how-rayx-chooses-and-installs-nsh)). Windows x64 and ARM64, Ubuntu and Debian x64 and ARM64, and macOS are supported. |
| `--web` | The `wasm32-unknown-unknown` target, `wasm-bindgen-cli` at the version `Cargo.lock` locks (only when it locks one), and Homebrew's LLVM on macOS (Linux and Windows have LLVM in the base set). |
| `--test` | Node.js, pnpm (through corepack), the Linux libraries headed Chromium needs, and the project's Playwright package with its Chromium. |
| `--android` | JDK 21 and `JAVA_HOME`, Android Studio, the SDK with its command-line tools, platform, build tools and NDK (a set `ANDROID_NDK_HOME` must name an NDK; `rayx app` sets it to the pinned one), the emulator system image and a default AVD, emulator acceleration, and the Rust Android targets. |
| `--ios` | (macOS) Xcode through `xcodes`, its command line tools, license and first launch, the Simulator runtime, and the iOS, simulator and Catalyst targets. |
| `--wsl` | From Windows: WSL 2 with `Ubuntu-24.04`, the Linux `rayx` of the same version in it, and the same `rayx setup` run inside it. `--clone [<dir>]` also clones the current checkout (only that one) into the distribution. |
| `--lima` | From macOS: a Lima VM (`rayx`, or the name `--vm` gives) with Ubuntu 24.04, the Linux `rayx` of the same version in it, and the same `rayx setup` run inside it; see [`rayx setup --lima`](#rayx-setup---lima). Never part of `--all`. |
| `--update` | Re-runs the setup recorded on this machine and refreshes its guests; see [`rayx setup --update`](#records-rayx-setup---update-and-the-refresh). |
| `--release <X.Y.Z>` | With `--lima`, `--wsl` or `--update`: puts exactly that release of the Linux `rayx` in the guest; see [Choosing a release](#choosing-a-release-and-going-back). |
| `--vm <name>` | With `--lima`: the VM's name (default `rayx`). With `--update`: refreshes only the named guests (repeatable). |

The sets each `rayx app` command needs are in [Sets per target](#sets-per-target).

### `rayx setup --lima`

On macOS, `rayx setup --lima [--clone [<dir>]] [set flags]` makes a Linux guest the way `--wsl`
does on Windows. It needs macOS 13 or later and Homebrew (run `rayx setup` first if you have none),
installs Lima with `brew install lima` when it is missing (an older Lima is an update and is asked
about), and creates **one** VM named `rayx` (or the name `--vm <name>` gives, see below): `vz`, the
Ubuntu 24.04 template, 6 CPUs, 12 GiB memory and a 100 GiB sparse disk, with your macOS home
mounted **read-only** and Lima's container runtime off. Then it puts the Linux `rayx` of the same
version inside (the release installer, so `rayx self update` works there too) and runs `rayx setup`
with the same set flags inside. `--check` prints every command, including the `limactl create`
line, and creates nothing. `--all` does not imply `--lima`: a VM and its 620 MB image are not a
requirement of the host. No step needs `sudo`.

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
shrink one. VMs `rayx` did not create are never started, stopped, edited, updated or removed.

`--vm <name>` gives the VM its own name, so one VM per project repository works:
`rayx setup --lima --vm app` creates, starts and records a VM `app` (default `rayx`, which behaves
exactly as before). A name is 1 to 40 lowercase letters, digits and hyphens, starts with a letter
and is not `default`; anything else is refused with that rule before any command runs. Every VM has
its own record with its own flags, clones and settings, the sizing flags change only the VM they are
given with, and a migration of `app` keeps the old VM as `app-<old version>`. A VM that exists in
Lima but is not in `guests.json` is **never adopted**, whatever its name and `rayx` included: remove
or rename it with `limactl`, or choose another `--vm` name. Nothing is created while `guests.json`
cannot be read.

### Choosing a release and going back

By default a machine runs the latest `rayx` and its guests run the version of the `rayx` that
refreshes them. To try another release, in either direction:

- `rayx self update --release 1.0.1 [--yes] [--check]` installs exactly that release over the
  installed one, for a `rayx` the release installers made (the install receipt). The name is
  `--release`, not `--version`. A release that does not exist exits 1 with `release X was not found`,
  and `--check` prints the installed release, the requested one and the direction (`upgrade`,
  `downgrade` or `same`) and installs nothing.
- `rayx setup --lima|--wsl --release 1.0.1` and `rayx setup --update --release 1.0.1` put that
  release of the Linux `rayx` in the guest, from that tag's own installer, instead of the running
  version. The release is looked for first (a header request, nothing downloaded) and nothing
  changes if it is missing. It cannot be combined with `RAYX_LIMA_BINARY` or `RAYX_WSL_BINARY`.

Installing an older release than the one in place is a downgrade and is confirmed:
`Install rayx 1.0.1 over 1.2.0, an older release? [y/N]` for the host and `Install rayx 1.0.1 in
<guest> over 1.2.0, an older release? [y/N]` for a guest. `--yes` skips the question, no terminal
and no `--yes` refuses, and a no changes nothing (exit 1). Nothing is remembered: a later plain
`rayx self update` installs the latest release, and a later plain `rayx setup --update` brings the
guests back to the running version.

`1.0.0` is the oldest release that can be named, the first of this CLI; an older version is refused.
`--release` names this CLI's releases only: the old CLI's (1.0.0 to 1.4.1) are in
`rayx-ui/RayX.CLI.Old` and cannot be installed by this `rayx`. Every release of this CLI has
`setup --update` and the refresh offer.

### Records, `rayx setup --update` and the refresh

Every `rayx setup` run records on the machine it ran on which sets it installed and in which
project, in `setup.json` next to `guests.json` in `$XDG_CONFIG_HOME/rayx` (`~/.config/rayx`), or
`%LOCALAPPDATA%\rayx` on Windows. `setup --wsl` and `--lima` also record the guests they manage in
`guests.json`: kind, name, Ubuntu version, set flags, clones and VM settings. The records hold
names, versions, flags, paths and sizes, nothing secret, and `--check` records nothing.

`rayx setup --update [--yes] [--check]` re-runs setup for the recorded sets once per recorded
project (in that project, so its pins apply, its nsh among them; a project that no longer exists is
reported and dropped), then refreshes every current guest of this host: it asks `Update lima guest rayx? [y/N]` (no question
with `--yes`, and no terminal without `--yes` means not updated and exit 1), starts a stopped VM and
stops it again afterwards, puts the Linux `rayx` of the running version inside, runs
`sudo apt-get update` and `sudo apt-get upgrade`, and runs `rayx setup --update` inside. A failing
guest is reported and the next still runs. `apt-get upgrade` runs only inside guests `rayx`
manages, never on the host. Last it offers `rayx nsh prune` for the nsh builds no recorded project
pins any more. `--check` inspects running guests (a stopped VM is not started) and changes nothing.

`rayx setup --update --vm <name>` (repeat the flag for more) refreshes only the named recorded,
current guests, Lima VMs and WSL distributions alike, and skips the host part: an explicit target
is not a host run. A name that is not a recorded current guest of this host is refused with the
recorded names, before any command runs. `--check` then inspects only the named running guests.

After `rayx self update` installed a newer release and a record exists, it asks `Refresh this
machine's setup and guests with rayx <new>? [y/N]`; yes, or `rayx self update --yes`, runs the new
binary's `setup --update`, and its exit code is the command's. Otherwise it prints
``run `rayx setup --update` to refresh the setup and guests``.

### Ubuntu versions and replacing a guest

All guests use the one Ubuntu pin (24.04, built in: a guest serves every project on the machine).
A guest older than the pin is never upgraded in place. `rayx setup --lima`, `--wsl` and
`--update` list the clones in it that hold uncommitted work or commits on no remote, say the old
guest is kept, and ask `Create <new> with Ubuntu 24.04 next to <old>? [y/N]`. On yes, a Lima VM is
renamed `rayx-<old version>` (`<name>-<old version>` for a VM chosen with `--vm`) and a new `rayx`
is created from the pinned image with the same size and home mount; on WSL `Ubuntu-24.04` is
installed next to the old distribution. Either way the Linux `rayx`, the recorded clones (copied
again from their checkouts) and the recorded setup flags are restored in the new guest, and the old
one is kept, retired, until you remove it. An interrupted migration continues on the next run. A
guest newer than the pin is reported and its release left alone.

### `rayx lima` and removing guests

- `rayx lima remove <name> [--yes]` (macOS) runs `limactl delete --force <name>`.
- `rayx wsl remove <name> [--yes]` (Windows) runs `wsl --unregister <name>`.

Both work only for guests in `guests.json` (a name `rayx` did not create is refused with how to
remove it yourself), list the clones in the guest that hold unpushed work, ask `Remove <name> and
everything in it? [y/N]` (without a terminal and without `--yes` they refuse), and drop the guest
from the record afterwards. Removing the current guest says that `rayx setup --lima` (with
`--vm <name>` for a named VM) or `--wsl` creates it again; `rayx doctor` names the removal command
for every retired guest.

### `rayx doctor`

Reports every requirement of the selected sets as present, missing or the wrong version, with the
version found and the exact fix, plus the desktop session (`interactiveDesktop`) and, from Windows,
the size of each WSL virtual disk. It shows every pin with its source, the project's nsh among them
(`nsh  fork-0.2.0-4 (327bb9e)  project  installed  3.1 GiB`). On macOS it reports Lima (the
`limactl` version, or missing with `rayx setup --lima`), and on macOS and Windows every guest
`rayx` manages: state (running, stopped, missing), CPUs, memory and disk, whether the home is
read-only or writable, the Ubuntu version, whether the guest is current, outdated or retired, and
for a retired or missing one the command that removes it. `--json` prints the same for tools, with
an `nsh` object and a `guests` array. Doctor never starts a VM, never installs nsh, and exits 0
whenever the host could be inspected.

### `rayx wsl`

- `rayx wsl status [--json]` lists the WSL distributions with their default user, the size of each
  `ext4.vhdx` on the Windows disk, the space used and free inside, and the size of the build output
  (`target/`, `artifacts/`, `artifacts-temp/` and the apps' `.nsh/`) under the checkouts
  `setup --wsl --clone` made.
- `rayx wsl compact [--distro <name>] [--clean] [--yes]` gives unused space back to Windows: it
  optionally deletes that build output, runs `fstrim` inside the distribution, asks before
  `wsl --shutdown` (which stops every distribution, including VS Code remote sessions), and
  compacts the virtual disk with `Optimize-VHD` (or `diskpart`) in one elevated child, reporting
  the size before and after. It never enables sparse disks, which WSL 2.6 disabled because they can
  corrupt a distribution.
- `rayx wsl remove <name> [--yes]` unregisters a distribution `rayx` created; see
  [`rayx lima` and removing guests](#rayx-lima-and-removing-guests).

### `rayx app`

`rayx app [-C <dir>] [--no-install] <command> [nsh arguments] [-- app arguments]` runs an nsh
command with the nsh the project pins:

| Option | What it does |
| --- | --- |
| `-C <dir>` | Runs in `<dir>` instead of the current directory: the pin is read and nsh runs there. |
| `--no-install` | Reports missing prerequisites (the pinned nsh among them) with the `rayx setup` command that installs them, instead of installing them. |

The commands are nsh's: `init`, `build`, `check`, `dev`, `run`, `test`, `package`, `publish`,
`keygen`, `certify`, `verify`, `revoke` and `symbolicate`. Everything after the command goes to nsh
unchanged, `--` and what follows included: `rayx` reads only the command name, `--target` and
`--browser`, so nsh's own documentation applies and a new nsh flag needs no `rayx` release.
`rayx app <command> --help` prints nsh's own help.

Before nsh starts, `rayx` checks the base set and the sets the command and its `--target` need
([Sets per target](#sets-per-target)). What is missing and needs no administrator rights and no
license answer (the pinned nsh, a Rust target) is installed on the spot, with a line saying so.
Anything that needs `sudo`, administrator rights or a license is asked about in a terminal, and
otherwise the command stops with the exact `rayx setup` command. On a host `rayx setup` does not
support, only the pinned nsh is checked, with `rayx nsh install -C <dir>` as its command.

nsh runs in the directory the pin was read in, without `CARGO` in its environment. When they are
unset, `rayx` sets `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown` to the LLVM the web
set installed, and `ANDROID_NDK_HOME` to the pinned NDK; everything else passes through. On macOS
and Linux `rayx` replaces itself with nsh, so a process manager that stops `rayx` stops nsh; on
Windows it leaves Ctrl+C to nsh, waits for it and exits with nsh's code.

#### Sets per target

| `rayx app` command and target | Sets beyond the base set |
| --- | --- |
| any command with `--target web` | `--web` |
| `test --target web` | `--web` and `--test`; with no Chrome, Edge or Chromium installed, `rayx` passes nsh `--browser` with the test set's Chromium |
| `--target ios`, `ios-simulator` or `catalyst` | `--ios` |
| `--target android` | `--android` |
| `--target macos`, `linux`, `windows` or `gpui`, or no target | none |
| `build`, `check`, `init`, `keygen`, `certify`, `publish`, `verify`, `revoke`, `symbolicate` | none |

### How `rayx` chooses and installs nsh

`rayx` never runs the `nsh` on your `PATH`. Every project names its nsh, and `rayx` reads that pin
in the directory it runs nsh in:

1. **`Cargo.lock`:** in the project's Cargo workspace (`cargo locate-project --workspace`, or the
   crates `nsh.json` lists under `"rust"` when there is none), every NSH package (NSH's own crates,
   not an app's `nsh-app-*`) has one and the same `source`. Without a lock, `cargo metadata` writes it first, and two sources stop with both
   named. A git source with a tag (`git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4#327bb9e…`)
   installs from that tag's release, a branch or bare commit from source, crates.io's `0.2.0` from
   upstream's `v0.2.0` release, and a path dependency runs that checkout (see below).
2. **`rayx.toml`:** an app without Rust crates names its nsh in the `[nsh]` table of `rayx.toml`
   beside its `nsh.json` (`git`, `tag` and `commit`, or `version = "0.2.0"` for an upstream
   release). The commit is checked against what the tag gives. An app with both is refused: its
   pin is `Cargo.lock`'s, so remove the table.
3. **The default:** with neither, `rayx` uses the nsh this release was tested with (`fork-0.2.0-4`
   at `327bb9e`) and says so in an app, with the three lines that pin it.

The project contract (the pins, their sources and the `rayx.toml` table) is in
[Projects](docs/projects.md).

**The store.** Each pin is installed once per machine, in `$RAYX_HOME/nsh/<id>` (`~/.rayx/nsh`):
the id is the commit's first 12 hex digits (`327bb9ea65d0`), or `v0.2.0` for an upstream release.
An install downloads the release's `nsh-<platform>.tar.gz` (`.zip` on Windows) and its `.sha256`,
checks the digest and unpacks it. A pin with no release archive for this host is built from source:
a shallow checkout of the commit, its own Rust toolchain, and
`cargo build --release --locked -p nsh-cli --target-dir target`. Either way `nsh --version` must name the pinned version
and commit, and `receipt.json`, written last, makes the entry complete; an unfinished entry is
downloaded again, or its build resumed. A second `rayx` installing the same pin waits for the first.
Everything runs as you, with no question and no administrator rights.

| Command | What it does |
| --- | --- |
| `rayx nsh status [-C <dir>] [--json]` | Shows the pin, where it comes from (`project`, `rayx.toml` or `default`) and its store entry. |
| `rayx nsh path [-C <dir>] [--no-install]` | Prints the pinned CLI's path, installing it first when it is missing (for scripts that need the binary). |
| `rayx nsh install [-C <dir>]` | Installs the pin into the store. |
| `rayx nsh prune [--yes]` | Lists the entries no recorded project and no default pins, with their sizes, and removes them after asking. |

**Working on NSH itself.** A path dependency on an NSH checkout makes `rayx` run
`cargo build --release -p nsh-cli --target-dir target` in that checkout and run its `target/release/nsh`, copying
nothing into the store. `RAYX_NSH=<path>` runs that binary instead of the pin's, with a warning on
every command: `rayx: RAYX_NSH=<path> replaces the project's nsh 327bb9ea65d0`. A relative path is
read against the directory `rayx` started in (never searched on `PATH`), and `rayx nsh path` prints
it too, for scripts that need the binary.

A source entry or a path checkout builds nsh's hosts and runtimes into its own `target/` the first
time a command needs them. nsh's web-runtime build follows `CARGO_TARGET_DIR` but looks for its
output in the checkout's `target/`, so unset `CARGO_TARGET_DIR` while such an nsh builds its web
runtime (the fork's release archives bring every runtime prebuilt).

### `rayx new`

`rayx new <dir> [--name <name>] [--template mixed|ts|rust]` creates an app that names its nsh from
its first commit: it installs the default pin, runs its `nsh init`, and pins it in the app. `mixed`
(the default: TSX with a Rust crate at the app's root) and `rust` get a `.cargo/config.toml` that
patches every NSH crate to the default pin's tag and a `Cargo.lock` from `cargo generate-lockfile`;
`ts` gets a `rayx.toml` with the `[nsh]` table. Then it prints the next steps: `rayx setup --web`
(with `--ios` on a Mac) and `rayx app run --target web`. A directory nsh refuses (one with files in
it) is left unchanged, and so is one that already has the file `rayx new` would write
(`.cargo/config.toml`, or `rayx.toml` for `ts`).

### `rayx serve`

`rayx serve [--dir <site>] [--port <port>]` serves what `rayx app package --target web` writes, on
`127.0.0.1` only (the port from `--port`, else `PORT`, else 8080): files with their MIME types
(`.wasm` as `application/wasm`), `index.html` for a directory, `GET /health`, nothing outside the
site, and `Cache-Control: no-store` on every response.

### `rayx fmt`

`rayx fmt [--check]` formats the project's Rust sources one workspace member at a time; `--check`
reports the unformatted members instead.

### Versions and where they come from

Every version `rayx` needs is resolved in this order: the project (`Cargo.lock` for `nsh` and
`wasm-bindgen`, `rust-toolchain.toml` for Rust, `[workspace.metadata.rayx]` for the rest), then the
built-in default. `rayx doctor` shows each pin with its source, `project` or `default`.

| Pin | Default | Project key |
| --- | --- | --- |
| nsh | `fork-0.2.0-4` at `327bb9e` | none: the NSH crates' source in `Cargo.lock`, or `rayx.toml` |
| Rust toolchain | stable | none: `rust-toolchain.toml` |
| wasm-bindgen | none | none: `Cargo.lock` |
| Node.js | 22 | `node` |
| JDK | 21 | `jdk` |
| Android platform | `android-35` | `android-platform` |
| Android build tools | `35.0.1` | `android-build-tools` |
| Android NDK | 28.0.12674087 | `android-ndk` |
| Emulator system image | `<platform>;google_apis` | `android-system-image` |
| Xcode | the newest stable | `xcode` |
| Guest Ubuntu (WSL and Lima) | 24.04 | none: built in, since a guest serves every project |

## Building from source

```bash
cargo build
cargo test --test <group>
```

The package builds from a clean clone with crates.io dependencies only. The real proofs, which
install nsh and run apps, are the ignored `proof_*` groups (`cargo test --test proof_<name> --
--ignored`); what is checked by hand on Linux, Windows and WSL is in
[Manual verification](docs/manual-verification.md).

## License

Apache-2.0, see [LICENSE-APACHE](LICENSE-APACHE).
