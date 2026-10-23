# Releasing `rayx`

A release is a version tag `vX.Y.Z` on `main`. Two tools split the work:

- **release-plz** (`.github/workflows/release-plz.yml`, `release-plz.toml`) turns Conventional
  Commits into a release pull request and, when that pull request is merged, pushes the tag. It
  creates no GitHub Release and publishes nothing to crates.io (`git_only = true`).
- **cargo-dist** (`.github/workflows/release.yml`, generated from `dist-workspace.toml`) starts on
  the tag, builds the six targets, writes the checksums, attestations and installers, and creates
  the GitHub Release with the notes from the matching `CHANGELOG.md` section.

## How a release happens

1. Commit to `main` (through pull requests) with Conventional Commits. The type decides the next
   version:

   | Commit | Release |
   | --- | --- |
   | `fix:`, `perf:` | patch (`1.0.0` → `1.0.1`) |
   | `feat:` | minor (`1.0.0` → `1.1.0`) |
   | `docs:`, `test:`, `ci:`, `build:`, `chore:`, `refactor:` | none on their own (`release_commits` in `release-plz.toml` lists `feat`, `fix` and `perf`); they appear in the changelog of the next release |

   A change that breaks consumers follows the 1.x policy below. One that touches `rayx app`, the
   pins, `rayx.toml` or the nsh store layout needs the matching change in the projects that run
   through `rayx` (Gamify and RayX's apps); keep the commits paired.
2. On every push to `main` release-plz opens or updates the release pull request: the version bump
   in `Cargo.toml` and `Cargo.lock` and a new `CHANGELOG.md` section. Review it; edit the changelog
   text in the pull request if a line reads badly.
3. Merge the release pull request. Its CI runs like any pull request (`ci.yml`, and the Release
   workflow builds all six targets without publishing), and `ci.yml` runs again on the merge to
   `main`. After the merge release-plz finds a version in `Cargo.toml` with no tag and pushes
   `vX.Y.Z`.
4. The tag starts the Release workflow. When it is green the GitHub Release exists with the
   archives, checksums, `rayx-cli-installer.sh`, `rayx-cli-installer.ps1` and `sha256.sum`.

## The first release (1.0.0)

This `rayx` is a new CLI with its own history, and its first release is `v1.0.0`. `Cargo.toml`
already says `1.0.0` and the repository has no tag, and in git-only mode release-plz treats a
package without a matching tag as an initial release: the `release` job tags `v1.0.0` on the first
push to `main` that contains the release-plz workflow, without a release pull request.

An initial release gets no changelog section from release-plz, so `CHANGELOG.md` carries a
hand-written `## [1.0.0] - YYYY-MM-DD` section, which dist reads for the release notes. Before that
first push to `main`:

1. Move `RayX.CLI.Old`'s README install links to `rayx-ui/RayX.CLI.Old` URLs and push them: until
   then they point at this repository's name and would install this `rayx`.
2. Create `rayx-ui/RayX.CLI` (public) and set the `RELEASE_PLZ_TOKEN` secret (below).
3. Set the date of the `1.0.0` section to the release day.
4. Push `main`. release-plz tags `v1.0.0`, and dist builds and publishes the release.

A machine with the old rayx 1.4.1 does not move by itself: its `rayx self update` sees 1.0.0 as
older than 1.4.1. Run the installer of 1.0.0 there; it replaces `~/.rayx/bin/rayx` and the install
receipt, and keeps `setup.json` and `guests.json`, so `rayx setup --update` refreshes the recorded
setups and guests afterwards.

## The 1.x policy

While the owner is the only consumer of `rayx`, releases stay on 1.x. release-plz proposes a major
version for a `!` commit or a `BREAKING CHANGE:` footer and cannot cap it, so:

- No commit is marked breaking: no `!` after the type or scope (`feat!:`, `feat(app)!:`) and no
  `BREAKING CHANGE:` footer.
- A break is committed as `feat` (or `fix` when it is a fix) whose body states what breaks, with a
  `Migration:` footer that tells consumers what to change, for example:

  ```text
  feat(nsh): read the pin of an app without Rust crates from rayx.toml only

  An app without NSH crates no longer falls back to the default pin silently.

  Migration: add the [nsh] table of rayx.toml beside the app's nsh.json.
  ```

  The changelog prints each `Migration:` footer under its commit's line. A break is therefore a
  minor bump (`1.3.0` → `1.4.0`); releases stay 1.x.
- A release pull request that proposes `2.0.0` means a stray marker. Do not merge it: find the
  commit (`git log --grep='^BREAKING' --grep='!:' -E v<last>..origin/main`), keep a local ref at the
  current head, reword the commit, `git fetch origin && git rebase origin/main`, push with
  `git push --force-with-lease origin main`, and read the pull request again.
- `semver_check = false` and `release_commits = "^(feat|fix|perf)"` stay as they are.

The policy ends when a second consumer appears; then `!` and major bumps return.

## The `RELEASE_PLZ_TOKEN` secret

Events made with the workflow's default `GITHUB_TOKEN` start no other workflows. With it the release
pull request would get no CI and the tag release-plz pushes would not start the Release workflow, so
release-plz uses a fine-grained personal access token stored as the repository secret
`RELEASE_PLZ_TOKEN`. `rayx` itself never reads or stores it; it exists only in GitHub Actions.

Create it as the repository owner:

1. GitHub → Settings → Developer settings → Personal access tokens → Fine-grained tokens →
   Generate new token.
2. Resource owner `rayx-ui`; repository access: only `RayX.CLI`.
3. Repository permissions: **Contents** read and write, **Pull requests** read and write (Metadata
   read is added automatically).
4. Set an expiry (at most one year) and note the date.
5. In `rayx-ui/RayX.CLI` → Settings → Secrets and variables → Actions → New repository secret named
   `RELEASE_PLZ_TOKEN`.

Release pull requests, tags and the commits release-plz makes then appear as the token's owner.

**When it expires, nothing fails loudly:** the `release` and `release-pr` jobs fail at checkout or
push (the Actions tab shows red Release-plz runs), so no release pull request is updated and no tag
is pushed; releases silently stop until the token is replaced. Rotate before the expiry date:
generate a new token with the same settings and replace the secret's value. Nothing else changes.

## By-hand fallback

If release-plz cannot cut a release (a broken workflow, an expired token you cannot replace yet),
release by hand on an up-to-date `main`:

```bash
# 1. Set the version in Cargo.toml and refresh the lockfile
cargo update -p rayx-cli --offline
# 2. Add a `## [X.Y.Z] - YYYY-MM-DD` section to CHANGELOG.md
# 3. Commit it through a pull request, then tag the merge commit and push the tag
git tag vX.Y.Z
git push origin vX.Y.Z
```

The tag starts the Release workflow exactly as release-plz's tag does. Tags always have the form
`vX.Y.Z`, because dist's trigger, the WSL installer (`rayx setup --wsl`) and `rayx self update` all
read it. release-plz continues from the by-hand tag on its next run.

## Checking a release

```bash
gh release view vX.Y.Z --repo rayx-ui/RayX.CLI
gh release download vX.Y.Z --repo rayx-ui/RayX.CLI --pattern 'rayx-cli-aarch64-apple-darwin.tar.xz'
gh attestation verify rayx-cli-aarch64-apple-darwin.tar.xz --repo rayx-ui/RayX.CLI
```

Each archive has a `.sha256` file next to it, and `sha256.sum` lists all of them. The shell
installer verifies the download against its checksum; the [README](../README.md#install) has the
install one-liners.

macOS binaries and Windows executables are not signed or notarized yet; Gatekeeper only quarantines
archives downloaded by hand in a browser, not the installers' downloads.
