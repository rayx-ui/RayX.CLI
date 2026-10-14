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

   | Commit | Bump |
   | --- | --- |
   | `fix:`, `perf:` | patch (`1.0.0` → `1.0.1`) |
   | `feat:` | minor (`1.0.0` → `1.1.0`) |
   | `feat!:` or a `BREAKING CHANGE:` footer | major (`1.0.0` → `2.0.0`) |
   | `docs:`, `test:`, `ci:`, `chore:`, `refactor:` | no release on their own; listed in the next one |

   A breaking change that touches the `rayx app` grammar, its flags, the artifact layout or the
   `[package.metadata.rayx]` contract needs the matching RayX change (RayX's CI installs a pinned
   `rayx`); keep the two commits paired.
2. On every push to `main` release-plz opens or updates the release pull request: the version bump
   in `Cargo.toml` and `Cargo.lock` and a new `CHANGELOG.md` section. Review it; edit the changelog
   text in the pull request if a line reads badly.
3. Merge the release pull request. Its CI runs like any pull request (`ci.yml`, and the Release
   workflow builds all six targets without publishing). After the merge release-plz finds a version
   in `Cargo.toml` with no tag and pushes `vX.Y.Z`.
4. The tag starts the Release workflow. When it is green the GitHub Release exists with the
   archives, checksums, `rayx-cli-installer.sh`, `rayx-cli-installer.ps1` and `sha256.sum`.

The first release, `v1.0.0`, works the same way: `Cargo.toml` already says `1.0.0` and has no tag, and
in git-only mode release-plz treats a package without a matching tag as an initial release, so the
`release` job tags it on the first push to `main` that contains the release-plz workflow.

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
