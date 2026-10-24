---
agentx:
  kind: index
  scope: knowledge
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - rayx
    - taxonomy
  supersedes: []
  superseded_by: []
---

# rayx Knowledge Index

Current source, tests, manifests and `docs/` establish behaviour; this tree records durable routing, rules and rationale for `rayx`, the RayX command-line tool for NSH apps. Decisions live in the [decision index](../decisions/INDEX.md).

## Domains

- [host](host/INDEX.md): host facts (OS, native architecture, WSL, distribution), the command runner and user PATH editing.
- [project](project/INDEX.md): workspace discovery through Cargo and the version pins a project declares, with the built-in defaults table.
- [setup](setup/INDEX.md): the requirement sets for NSH (base, web, test, iOS, Android), the plan/probe/execute engine, doctor, setup.json, and the WSL and Lima guests.
- [nsh](nsh/INDEX.md): the nsh a project pins: reading the pin, the store under $RAYX_HOME/nsh, installing from a release or from source, and running it.
- [app](app/INDEX.md): `rayx app` (nsh commands with the pinned nsh and their prerequisites), `rayx new` and `rayx serve`.
- [distribution](distribution/INDEX.md): CI, release-plz and cargo-dist releases for six targets, the installers, and `rayx self update`.
- [repository](repository/INDEX.md): repository governance: the package and dependency rules, the library guard, commits, consumers, the fork, and this memory tree.

## Source Anchors

- [Repository instructions](../../../AGENTS.md)
- [Design](../../../docs/architecture/rayx-nsh-proposal.md)
- [First epic: rayx-1-0](../../tasks/rayx-1-0/requirements.json)
