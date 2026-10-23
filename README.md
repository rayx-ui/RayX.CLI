# rayx

`rayx` is the RayX command-line tool for [Native Shell (NSH)](https://github.com/dXnSix/nsh) apps.
One prebuilt binary sets up a machine for NSH work and runs any NSH app through the `nsh` its
project pins.

## Install

Latest release, macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.sh | sh
```

Latest release, Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.ps1 | iex"
```

The installer puts `rayx` in `~/.rayx/bin` (`$RAYX_HOME/bin` when `RAYX_HOME` is set), adds that
directory to your `PATH` (open a new shell afterwards) and writes the install receipt that
`rayx self update` reads.

## Commands

| Command | What it does |
| --- | --- |
| `rayx setup` | Installs everything NSH development needs on this machine, including the project's nsh. |
| `rayx doctor` | Reports every requirement on this host with the exact fix. |
| `rayx wsl` | Shows the WSL status, compacts the WSL virtual disk and removes a distribution `rayx` created. |
| `rayx lima` | Removes a Lima VM `rayx` created (macOS). |
| `rayx self update` | Updates `rayx` to the latest release, or installs the release `--release` names. |
| `rayx fmt` | Formats the project, one workspace member at a time. |
| `rayx app` | Runs an nsh command with the project's pinned nsh. |
| `rayx nsh` | Shows, installs, locates and prunes the nsh builds `rayx` keeps. |
| `rayx new` | Creates a new NSH app pinned to `rayx`'s default nsh. |
| `rayx serve` | Serves a packaged web app on `127.0.0.1`. |

## License

Apache-2.0, see [LICENSE-APACHE](LICENSE-APACHE).
