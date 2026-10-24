---
agentx:
  kind: knowledge
  scope: knowledge/setup
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - setup
  supersedes: []
  superseded_by: []
---

# Setup Knowledge

- nsh needs GTK 4.14 and WebKitGTK 6.0 on Linux (Ubuntu 24.04, Debian 13); derivatives are allowed and apt decides.
- Playwright Chromium layouts: `chrome-mac` (Chromium), `chrome-mac-arm64` and `chrome-mac-x64` (Chrome for Testing), `chrome-linux`, `chrome-linux64`, `chrome-win`, `chrome-win64`.
- nsh reads `ANDROID_HOME`/`ANDROID_SDK_ROOT` for the SDK and `ANDROID_NDK_HOME` (unchecked) for the NDK, else the newest NDK of the SDK.
