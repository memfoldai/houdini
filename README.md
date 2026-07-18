# Houdini

Houdini is a private, local-only macOS menu-bar app that records **what you use AI for** — the observation instrument for an internal AI-usage study. It reads the real prompts and replies from AI tools' own local logs and web chats. It does **not** capture the screen and does **not** watch network traffic.

[![Release](https://img.shields.io/github/v/release/memfoldai/houdini?sort=semver)](https://github.com/memfoldai/houdini/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/memfoldai/houdini/ci.yml?branch=main)](https://github.com/memfoldai/houdini/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/macOS-14%2B-black?logo=apple)](#install)
[![Downloads](https://img.shields.io/github/downloads/memfoldai/houdini/total)](https://github.com/memfoldai/houdini/releases)
[![License](https://img.shields.io/github/license/memfoldai/houdini)](LICENSE)

<!-- Screenshot slot: drop a menu-bar dropdown capture at docs/menu.png and it renders here.
     ![Houdini menu bar](docs/menu.png) -->

## Features

- **Reads the real messages**, from two sources — no screenshots, no OCR, no "an app was open" noise.
  - **CLI/agent tools** via their local transcripts: Claude Code, Codex (incl. the Codex view of the new ChatGPT desktop app), OpenClaw.
  - **Web chats** via an optional browser extension: ChatGPT, Claude, and Gemini.
- **Local-only and encrypted at rest.** Nothing uploads; the store is an encrypted SQLite database whose key lives in the macOS Keychain.
- **Redacted before it touches disk** — secrets, emails, cards, SSNs, phones.
- **Menu-bar only**, no window, no permission prompts. Pause anytime.
- **Updates itself** over the air from GitHub Releases.

## Install

Download the latest [`.dmg`](https://github.com/memfoldai/houdini/releases), drag **Houdini** to Applications, and right-click → **Open** once. To also capture web chats, load the bundled browser extension. Full steps, uninstall, and distribution: **[docs/install.md](docs/install.md)**.

## Usage

Click the menu-bar icon — its shape is the status: a **hollow ring** when quiet, a **filled disc** while recording, **two bars** when paused. The menu shows today's session count and:

- **Take a break** — pause for 15 min, an hour, or until you're back.
- **Export my data…** — write a flat, one-row-per-message snapshot (`data/interactions.jsonl`) and reveal it.
- **Quit**.

## How it works

Every source — CLI transcript or web chat — is normalized into the **same one-row-per-turn record** (provider, tool, surface, model, redacted text) in the encrypted store. The web extension is a thin, local bridge: it reads the exchange in your own tab and hands it to the running app over a local socket; only the app writes the database. See **[docs/architecture.md](docs/architecture.md)**.

## Privacy

Houdini is built for a consenting internal study, so the data model is deliberate: **local-only, encrypted at rest, content redacted, tool/provider identity kept in the clear** (that's the research signal), and pausable at any time. What exactly is recorded, and what never leaves the device, is spelled out in **[docs/privacy.md](docs/privacy.md)**.

### Honest limits

- The extension is loaded per browser and tracks each site's page shape, so a redesign can need a small selector fix.
- **Native desktop *chat* apps** (ChatGPT.app, Claude.app) keep conversations server-side/encrypted and are **out of scope** — capturing them would require screen-recording, Accessibility, or a MITM proxy, all of which Houdini refuses. Use the web (extension) or CLI/agent tools. (Codex run inside the ChatGPT desktop app *is* captured, via its `~/.codex` transcripts.)

## Contributing

Build from source, load the extension, and the verification checklist: **[CONTRIBUTING.md](CONTRIBUTING.md)**. Security posture and how to report a concern: **[SECURITY.md](SECURITY.md)**.

## Scope

The study's observation instrument, nothing more: for internal, consenting participants on their own machines. Not for monitoring end-users. Built by Rahul Biliyar (<rahul@memfold.ai>).

## License

[MIT](LICENSE).
