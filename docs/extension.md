# Set up web-chat capture (2 minutes, one time)

Houdini already records your command-line AI use (Claude Code, Codex, OpenClaw) on
its own. To also record web chats (ChatGPT, Claude, and Gemini in the browser), add
this browser extension. It is optional, so skip it if you only use command-line
tools.

Everything stays on your Mac. The extension reads the chat in your own tab and hands
it to the app locally. Nothing is uploaded.

## Step 1: install the app first

If you haven't already, open the `.dmg`, drag **Houdini** to Applications, and open
it. The first time, macOS blocks a self-signed build, so go to **System Settings ->
Privacy & Security** and click **Open Anyway** next to Houdini. A small ring appears
in your menu bar. Leave it running.

The app must be installed and running for the extension to work. It wires up the
local connection automatically the moment it launches, so there is no terminal
command to run.

## Step 2: load the extension

This works in any Chromium browser (Chrome, Brave, Edge, Arc, Vivaldi).

1. Copy the **Browser Extension** folder (next to this file) somewhere permanent,
   such as your Documents folder. Don't leave it on the mounted disk image; the
   browser needs it to stay put.
2. In your browser, open the extensions page:
   - Chrome: `chrome://extensions`
   - Brave: `brave://extensions`
   - Edge: `edge://extensions`
3. Turn on **Developer mode** (toggle, top-right).
4. Click **Load unpacked** (top-left) and select the **Browser Extension** folder
   you copied in step 1.
5. You'll see **Houdini** in the list.

## Step 3: check it works

Open chatgpt.com, claude.ai, or gemini.google.com, send one message, and watch the
menu-bar ring. It fills to a solid dot when a chat is recorded.

## Good to know

- **Multiple browsers.** Repeat step 2 in each one you use.
- **Privacy.** The extension activates on two groups of sites: the AI chats
  (chatgpt.com, claude.ai, gemini.google.com), where it records the conversation;
  and Google Workspace (mail.google.com, drive.google.com, docs.google.com,
  sheets.google.com, slides.google.com, calendar.google.com), where it records
  **which actions you take** (send, archive, delete, …) for agent-vs-human
  attribution. It stores the normalized action verb, not raw control labels, so
  emails, files, or documents named in UI labels are not persisted. Everything is
  sent to the local app, never to the network, and is redacted (passwords, keys,
  emails, card numbers) before it is saved.
- **Pausing.** Use **Take a break** in the menu. While paused nothing is recorded,
  web chats included.
- **Updates.** When you get a new app version, replace the **Browser Extension**
  folder with the new one and click the reload icon on the extensions page. This
  build is not on the Chrome Web Store, so it does not auto-update.
- **Remove it.** Click **Remove** on the extensions page.
