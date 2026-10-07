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
| `rayx fmt` | Formats the project. |

Run `rayx --help` for the options of each command.

## Building from source

```bash
cargo build
cargo test --test <group>
```

The package builds from a clean clone with crates.io dependencies only.

## License

Apache-2.0. See [LICENSE-APACHE](LICENSE-APACHE).
