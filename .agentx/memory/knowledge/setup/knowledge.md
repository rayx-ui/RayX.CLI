---
agentx:
  kind: knowledge
  scope: knowledge/setup
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T00:30:00+02:00
  topics:
    - setup
    - doctor
    - wsl
    - windows-arm64
    - installers
  supersedes: []
  superseded_by: []
---
# Setup Knowledge

- Windows ARM64 needs, beyond x64: the `Microsoft.VisualStudio.Component.VC.Tools.ARM64` Build Tools component, LLVM/Clang (the `ring` crate, pulled in through `rustls`, needs Clang for its ARM64 assembly), both the ARM64 and x64 VC++ redistributables, the `arm64` Windows SDK `fxc.exe` directory on PATH, and a native `aarch64-pc-windows-msvc` rustup host.
- gpui compiles HLSL at build time with `fxc.exe` from the Windows SDK (`Windows Kits\10\bin\<version>\<arch>\fxc.exe`); it is not on PATH outside a VS developer shell.
- WSL has no hardware Vulkan driver: native Linux rendering uses lavapipe (`mesa-vulkan-drivers`, `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json`), and Chromium's WebGPU needs SwiftShader flags.
- Ubuntu 24.04's own `nodejs` is version 18; Node 22 or later comes from NodeSource. .NET 10 is in 24.04's archive, Python 3 is preinstalled, macOS's Command Line Tools ship Python 3.9.6 (facts from the 2026-10-07 runtime choice, which picked Rust).
- The first launch of a new WSL distribution asks interactively for a Unix user; `wsl --install` may need administrator rights and a reboot. Neither can be automated.
- WSL's `ext4.vhdx` location is the `BasePath` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss\{id}`. Compacting it needs `fstrim` inside, `wsl --shutdown` (which stops every distribution and VS Code remote sessions) and an administrator `Optimize-VHD -Mode Full` or `diskpart compact vdisk`.
- `xcodes` downloads Xcode from Apple's developer site after its own Apple ID prompt; `xcodebuild -downloadPlatform iOS` fetches the Simulator runtime without one; the Command Line Tools install headlessly through `softwareupdate` once `/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress` exists.
- A Playwright package must be an ancestor of its spec files for `@playwright/test` to resolve, so a package kept in a per-user cache cannot run an app's specs.
- The WSL virtual disk grows past 100 GB with gpux and RayX build output and never shrinks on its own; sparse disks are disabled in WSL 2.6 because they can corrupt the disk.
- Playwright's per-user browser cache serves every worktree; RayX's `setup-wasm` relied on it instead of a local browser payload.
