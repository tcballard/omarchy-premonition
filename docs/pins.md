# Pinned world

Audit timestamp: `2026-08-30T22:35:38Z`.

| Input | Immutable pin | Use |
| --- | --- | --- |
| Omarchy Quattro | `981274b20af8e85c09845071ac33c6230909f119` | Shell, manifest, IPC, loader, and official validator contract |
| Original Premonition | `2a4a85b0575d889c9e49c12e481dd0a16147d1ea` | Read-only product and safety evidence |
| Rust | `1.98.0` (`rustc 88d9e12ae`) | Build, format, lint, test, and rustdoc toolchain |
| Bootstrap base | `3e0657a94024ea30fd477c65c2db1117cc55e8aa` | Empty-repository PR base established with a title-only README |

## Authoritative Omarchy sources

At the pinned Quattro commit, `shell/services/PluginRegistry.qml` is the
executable manifest schema, `bin/omarchy-plugin-validate` is the publication
validator, and `shell/shell.qml` defines the actual loader lifecycle.

## Plugin-builder decision

`tcballard/build-omarchy-plugins` had no tags or GitHub releases at audit time.
Its moving `main` was therefore excluded. This repository is scaffolded
directly against the official Quattro sources.

## Loader decision

Quattro creates one on-demand loader per plugin ID and chooses `panel` before
`overlay` before `menu`. Premonition therefore declares `bar-widget` plus one
`panel`; that panel owns the internal full-screen review surface.
