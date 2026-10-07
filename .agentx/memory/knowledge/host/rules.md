---
agentx:
  kind: rules
  scope: knowledge/host
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:59:00+02:00
  topics:
    - host-facts
    - command-runner
    - privilege
    - path
  supersedes: []
  superseded_by: []
---
# Host Rules

- Detect the native CPU architecture, never the process architecture: `IsWow64Process2` on Windows, `sysctl.proc_translated` on macOS. An x64 `rayx` emulated on Windows ARM64 must still install ARM64 tools.
- Run every external program through `host::Runner`; `Print` mode has no side effects at all, PATH edits included.
- Never read, store or forward a password or token, and never run `rayx` itself elevated. Root steps run through `sudo` in the user's console (directly when already root); a Windows step that needs administrator rights runs as one elevated child process through the system UAC prompt (`ShellExecuteExW` with `runas`), and installers that raise their own UAC prompt run unelevated. Third-party credential prompts (an Apple ID in `xcodes`) reach the user directly.
- Authenticate a non-root `Root` step with an interactive `sudo -v` before the wrapped command; a refused prompt is `RunError::Declined` and the command must not run.
- Treat the `WM_SETTINGCHANGE` broadcast as best effort: the registry write is the edit, and a timed-out broadcast must not fail it.
- Keep PATH edits idempotent: one marked block per shell profile on Unix, `REG_EXPAND_SZ` preserved in `HKCU\Environment\Path` on Windows, followed by a `WM_SETTINGCHANGE` broadcast.
