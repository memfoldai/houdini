# Set up web-chat capture (2 minutes, one time)

The app already records your **coding-tool** AI use (Claude Code, Codex, OpenClaw)
on its own. To *also* record **web chats** (ChatGPT and Claude in the browser),
add this browser extension. It's optional — skip it if you only use CLI tools.

Everything stays **on your Mac**: the extension reads the chat in your own tab and
hands it to the app locally. Nothing is uploaded.

---

## Step 1 — Install the app first

If you haven't already: open the `.dmg`, drag **AI Usage Monitor** to
**Applications**, then **right-click it → Open** (once — macOS asks the first time
because it's an internal build). A small ring appears in your menu bar. Leave it
running.

> The app must be installed and running for the extension to work — it wires up
> the local connection automatically the moment it launches. There is **no
> terminal command** to run.

## Step 2 — Load the extension

This works in any Chromium browser: **Chrome, Brave, Edge, Arc, Vivaldi**.

1. Copy the **Browser Extension** folder (next to this file) somewhere permanent —
   e.g. your **Documents** folder. *(Don't leave it on the mounted disk image;
   the browser needs it to stay put.)*
2. In your browser, go to the extensions page:
   - Chrome → `chrome://extensions`
   - Brave → `brave://extensions`
   - Edge → `edge://extensions`
3. Turn on **Developer mode** (toggle, top-right).
4. Click **Load unpacked** (top-left) and select the **Browser Extension** folder
   you copied in step 1.
5. Done — you'll see **AI Usage Monitor** in the list.

## Step 3 — Check it works

Open **chatgpt.com** or **claude.ai**, send one message, and watch the menu-bar
ring: it fills to a solid dot when a chat is recorded. That's it.

---

## Good to know

- **Multiple browsers?** Repeat step 2 in each one you use.
- **Privacy:** the extension only activates on `chatgpt.com` and `claude.ai`, only
  reads the conversation, and sends it to the local app — never to the network.
  Content is redacted (passwords, keys, emails, card numbers) before it's saved.
- **Pausing:** use **Take a break** in the menu; while paused, nothing is recorded,
  web chats included.
- **Updates:** when you get a new app version, also replace the **Browser
  Extension** folder with the new one and click the **↻ reload** icon on the
  extensions page. *(This build isn't on the Chrome Web Store, so it doesn't
  auto-update.)*
- **Remove it:** click **Remove** on the extensions page.
