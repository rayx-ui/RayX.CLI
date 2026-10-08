#!/usr/bin/env python3
"""Runs a focused RayX Test Suite spec through this repository's `rayx`, against a sibling set
that is known to be consistent.

The RayX checkout builds and its Test Suite starts only when the sibling `RayX.Dtcg` is at a
revision whose token runtime accepts RayX's DTCG token files. While `RayX.Dtcg` is on its
2025.10 conformance branch, every RayX web app stops at start-up with `reference type mismatch`
(the same on RayX's own xtask), so the spec cannot say anything about `rayx`. This script builds
the set it needs under `artifacts-temp/rxproof/` without touching any checkout:

    rxproof/RayX         `git archive HEAD` of the RayX checkout
    rxproof/RayX.Dtcg    `git archive main` of the RayX.Dtcg checkout
    rxproof/Gpux, wgpu, RayX.Reactive, RayX.SignalR   links to the real checkouts

and then runs `rayx app apps/test_suite test wasm --headed --webgpu ...` there. Run it from the
RayX checkout (the proof check does), or pass `--rayx <dir>`. It needs the web and test sets and
an interactive desktop (`rayx doctor`).
"""

import argparse
import os
import subprocess
import sys
import tarfile
import io
from pathlib import Path

LINKED = ["Gpux", "wgpu", "RayX.Reactive", "RayX.SignalR"]
SPEC = "test_suite/rayx-collapsible-sizes.spec.ts"


def archive(repository: Path, revision: str, destination: Path) -> None:
    data = subprocess.run(
        ["git", "-C", str(repository), "archive", revision],
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data)) as tar:
        tar.extractall(destination)


def link(target: Path, name: Path) -> None:
    if name.exists():
        return
    if os.name == "nt":
        subprocess.run(
            ["cmd", "/c", "mklink", "/J", str(name), str(target)],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    else:
        name.symlink_to(target, target_is_directory=True)


def prepare(rayx: Path, root: Path, dtcg_revision: str) -> Path:
    """The RayX copy inside `root`, created from the checkout at `rayx` when it is missing or
    when the checkout moved to another commit."""
    head = subprocess.run(
        ["git", "-C", str(rayx), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    marker = root / "RayX" / ".rxproof-head"
    if not (marker.exists() and marker.read_text().strip() == head):
        # Extract over the old copy: its `target` and `node_modules` keep the build warm.
        archive(rayx, "HEAD", root / "RayX")
        marker.write_text(head)
    dtcg = rayx.parent / "RayX.Dtcg"
    if not (root / "RayX.Dtcg").exists():
        archive(dtcg, dtcg_revision, root / "RayX.Dtcg")
    for name in LINKED:
        link(rayx.parent / name, root / name)
    return root / "RayX"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rayx", default=".", help="the RayX checkout (default: .)")
    parser.add_argument("--dtcg-revision", default="main")
    parser.add_argument("--spec", default=SPEC)
    args = parser.parse_args()

    rayx = Path(args.rayx).resolve()
    cli = Path(__file__).resolve().parent.parent
    root = cli / "artifacts-temp" / "rxproof"
    copy = prepare(rayx, root, args.dtcg_revision)
    command = [
        "cargo",
        "run",
        "--release",
        "--manifest-path",
        str(cli / "Cargo.toml"),
        "--",
        "app",
        "apps/test_suite",
        "test",
        "wasm",
        "--headed",
        "--webgpu",
        "--no-default-features",
        "--features",
        "rayx_diagnostics",
        "--playwright-test",
        args.spec,
    ]
    print(f"running in {copy}: {' '.join(command)}", flush=True)
    return subprocess.run(command, cwd=copy).returncode


if __name__ == "__main__":
    sys.exit(main())
