# Manual verification

What automated tests and hosted CI runners cannot prove is run by hand and recorded here. Each row
names the machine, the command, the result and the date. A row without a result is still open.

The automated proofs are the ignored `proof_*` groups, run on demand:

```bash
cargo test --test proof_nsh_install -- --ignored --test-threads=1
cargo test --test proof_app_web -- --ignored --test-threads=1
cargo test --test proof_app_apple -- --ignored --test-threads=1
cargo test --test proof_gamify -- --ignored --test-threads=1
```

`proof_nsh_install` installs upstream's `v0.2.0` release and builds the fork from source;
`proof_app_web` and `proof_app_apple` make an app with `rayx new` and build, run, test and package
it; `proof_gamify` reads Gamify's frontend and changes none of its tracked files. Each uses a
temporary `RAYX_HOME`, so a run leaves the machine's store alone.

## Linux (a Lima guest on the Mac)

The web proof runs `rayx app` without a terminal, which installs nothing that needs `sudo`, and
`rayx app test --target web` needs the test set and a browser nsh can find. So the guest is set up in
a terminal first: `rayx setup --lima --web --test --clone` from this checkout (answer the guest's
`sudo` prompts), then inside the guest (`limactl shell rayx`, in the clone)
`pnpm dlx playwright install --with-deps chromium`: a guest has no browser of its own, and rayx
passes Playwright's Chromium (in `~/.cache/ms-playwright`) to nsh as `--browser` when none is
installed. Ubuntu's Chromium is a snap whose private `/tmp` hides the profile nsh waits for, so it
must not be installed instead. Ubuntu 24.04 also keeps unprivileged programs from creating the user
namespaces Chromium's sandbox needs, and nsh starts the browser sandboxed, so give Playwright's
Chromium the allowance Ubuntu gives Google Chrome:

```bash
sudo tee /etc/apparmor.d/playwright-chromium >/dev/null <<'PROFILE'
abi <abi/4.0>,
include <tunables/global>

profile playwright-chromium /home/*/.cache/ms-playwright/chromium-*/chrome-linux*/chrome flags=(unconfined) {
  userns,
}
PROFILE
sudo apparmor_parser -r /etc/apparmor.d/playwright-chromium
```

| Check | Command | Result |
|---|---|---|
| The web and test sets are complete | `rayx setup --web --test --check` | |
| The browser rayx passes | `ls ~/.cache/ms-playwright` shows a `chromium-<revision>`, and `command -v chromium chromium-browser google-chrome google-chrome-stable` prints nothing | |
| It starts sandboxed | `~/.cache/ms-playwright/chromium-*/chrome-linux*/chrome --headless=new --dump-dom about:blank` prints `<html>` (no "No usable sandbox!") | |
| Doctor in the guest | `rayx doctor` | |
| The web proof | `cargo test --test proof_app_web -- --ignored --test-threads=1` | |

## Windows and WSL (the owner's PC)

On Windows, `rayx setup --web --test --yes` first (in a terminal: `--yes` gives winget its license
answers up front, which the proof's `rayx app` cannot ask for); Edge, part of Windows, is the
browser nsh finds. In WSL, `rayx setup --wsl --web --test --clone` from Windows, then inside the
distribution, in the clone, Playwright's Chromium as in the Lima guest
(`pnpm dlx playwright install --with-deps chromium`).

| Check | Command | Result |
|---|---|---|
| The web and test sets on Windows | `rayx setup --web --test --check` | |
| Doctor on Windows | `rayx doctor` | |
| The web proof on Windows | `cargo test --test proof_app_web -- --ignored --test-threads=1` | |
| Ctrl+C reaches nsh | Ctrl+C in `rayx app dev`: nsh stops its session, and `rayx` exits with nsh's code | |
| A WSL guest | `rayx setup --wsl --web --test --clone` | |
| The web and test sets in WSL | `rayx setup --web --test --check` inside the distribution, in the clone | |
| The browser rayx passes in WSL | `ls ~/.cache/ms-playwright` shows a `chromium-<revision>` | |
| Doctor in WSL | `rayx doctor` inside the distribution | |
| The web proof in WSL | `cargo test --test proof_app_web -- --ignored --test-threads=1` inside the distribution | |
| Build output of NSH apps | `rayx wsl status`, then `rayx wsl compact --clean` after a proof run | |

## Releases

### The fork's first release (`fork-0.2.0-4`)

| Check | Command | Result |
|---|---|---|
| `fork/release` merged | its pull request into the fork's `main`, merged with a merge commit after the fork's CI | |
| The release runs | `gh workflow run fork-release.yml --repo dXnSix/nsh -f tag=fork-0.2.0-4` | |
| Four archives and their digests | `gh release view fork-0.2.0-4 --repo dXnSix/nsh` lists `nsh-macos-arm64.tar.gz`, `nsh-linux-x86_64.tar.gz`, `nsh-linux-arm64.tar.gz` and `nsh-windows-x86_64.zip` with a `.sha256` each (`nsh-windows-arm64.zip` when it built) | |
| The archive's CLI | `nsh/bin/nsh --version` from `nsh-macos-arm64.tar.gz` prints `nsh 0.2.0 (327bb9e)` | |
| A distribution entry | `rayx nsh install` in Gamify's `src/frontend`, then `rayx nsh status` shows `installed` and `receipt.json` has `"kind": "distribution"` | |

### `rayx` 1.0.0

| Check | Command | Result |
|---|---|---|
| The old README's links | `RayX.CLI.Old`'s install links name `rayx-ui/RayX.CLI.Old`, pushed before this repository exists | |
| The repository | `rayx-ui/RayX.CLI` created public, `RELEASE_PLZ_TOKEN` set, the `1.0.0` section of `CHANGELOG.md` dated | |
| The first release | the first push to `main`: release-plz tags `v1.0.0` (no release pull request) and cargo-dist publishes six targets | |
| Over the old rayx | the installer over a rayx 1.4.1 install keeps `setup.json` and `guests.json`, and `rayx setup --update` refreshes the recorded setup and guests | |

How a release is made and checked is in [Releasing `rayx`](releasing.md).
