# Houdini

*Behind every work-from-home employee is a cat that knows all its company secrets.*

Houdini, named after one such cat, quietly keeps track of what you use AI for. It
gathers your prompts and replies from the AI tools you already use into one tidy,
private place on your Mac.

[![Release](https://img.shields.io/github/v/release/memfoldai/houdini?sort=semver)](https://github.com/memfoldai/houdini/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/memfoldai/houdini/ci.yml?branch=main)](https://github.com/memfoldai/houdini/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/macOS-14%2B-black?logo=apple)](#install)
[![Downloads](https://img.shields.io/github/downloads/memfoldai/houdini/total)](https://github.com/memfoldai/houdini/releases)
[![License](https://img.shields.io/github/license/memfoldai/houdini)](LICENSE)

<!-- Screenshot slot: drop a menu-bar dropdown capture at docs/menu.png and it renders here.
     ![Houdini menu bar](docs/menu.png) -->

## What it does

- Follows your AI work across the tools you use: Claude Code, Codex, and OpenClaw
  on the command line, and ChatGPT, Claude, and Gemini on the web (with a small
  browser extension).
- Turns every exchange into one tidy record with the provider, tool, model, and
  the prompt and reply, ready for analysis.
- Lives in your menu bar as a quiet ring that fills in when it catches something.
  Take a break whenever you like.

## Private by design

Everything Houdini gathers stays on your Mac, encrypted, with sensitive details
like keys, emails, and card numbers stripped out before anything is saved. What it
keeps is deliberate and yours. The full picture is in [docs/privacy.md](docs/privacy.md).

## Install

Download the latest [`.dmg`](https://github.com/memfoldai/houdini/releases), drag
**Houdini** to Applications, and open it. To also catch web chats, load the bundled
browser extension. Full steps and uninstall are in [docs/install.md](docs/install.md).

## Using it

Click the menu-bar ring for today's count and to:

- **Export my data…** writes a flat, one-row-per-message file, ready for a warehouse.
- **Take a break** pauses for 15 minutes, an hour, or until you're back.
- **Quit**.

## How it works

Every source, whether a command-line transcript or a web chat, flows into the same
encrypted store through one small, uniform pipeline. The details are in
[docs/architecture.md](docs/architecture.md).

## Contributing

Build it, run it, and verify it with [CONTRIBUTING.md](CONTRIBUTING.md), and please
follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE). Built by Rahul Biliyar (<rahul@memfold.ai>).
