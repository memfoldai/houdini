// Human-action capture for Google Workspace apps.
//
// Sibling to capture.js. Where capture.js records AI *chat* turns, this records
// the *actions* the human performs in Gmail/Drive/etc. (send, archive, delete,
// …) so they can be attributed against the agent's own actions. It only reads
// the current tab and hands each action to the local app; nothing is uploaded.
//
// Detection is intentionally conservative: it classifies a click by the
// accessible label (aria-label / data-tooltip) of the control, then stores
// only the normalized action verb so page-specific names do not persist.
(function () {
  "use strict";

  const APPS = [
    { app: "mail.google.com" },
    { app: "drive.google.com" },
    { app: "docs.google.com" },
    { app: "sheets.google.com" },
    { app: "slides.google.com" },
    { app: "calendar.google.com" },
  ];

  // Map an accessible label (aria-label / data-tooltip) to a normalized action
  // verb, or null if unrecognized. Order matters: more specific first so e.g.
  // "Send & Archive" is a send and "Move to trash" is a delete. Labels are the
  // real ones Gmail/Drive/Docs expose on their toolbar and compose controls.
  function matchVerb(label) {
    const l = (label || "").toLowerCase();
    if (!l) return null;
    if (l.includes("send")) return "send"; // "Send ‪(⌘Enter)‬", "Send & Archive"
    if (l.includes("reply")) return "reply"; // "Reply", "Reply all"
    if (l.includes("forward")) return "forward";
    if (l.includes("archive")) return "archive";
    if (l.includes("spam")) return "spam"; // "Report spam"
    if (l.includes("snooze")) return "snooze";
    if (l.includes("trash") || l.includes("delete") || l.includes("remove")) return "delete";
    if (l.includes("move")) return "move"; // Drive "Move to…"
    if (l.includes("share")) return "share";
    if (l.includes("rename")) return "rename";
    if (l.includes("download")) return "download";
    if (l.includes("upload")) return "upload";
    if (l.includes("compose")) return "compose";
    return null;
  }

  // Expose the pure classifier for unit tests without affecting the page.
  if (typeof globalThis !== "undefined") {
    globalThis.__bbMatchVerb = matchVerb;
  }

  const site = APPS.find((a) => location.hostname === a.app);
  if (!site) return;

  // Per-tab nonce so two tabs of the same app can't mint the same id when their
  // first recognized click lands in the same millisecond (the store dedupes on
  // (source, ext_id), which would otherwise silently drop one).
  const PAGE_ID = Math.random().toString(36).slice(2);
  let counter = 0;

  document.addEventListener(
    "click",
    (ev) => {
      const info = classify(ev.target);
      if (info) emit(info);
    },
    true,
  );

  // Walk up a few ancestors so a click on an icon inside a button still resolves.
  function classify(target) {
    let node = target;
    for (let i = 0; node && i < 5; i++) {
      const label =
        (node.getAttribute &&
          (node.getAttribute("aria-label") || node.getAttribute("data-tooltip"))) ||
        "";
      const verb = matchVerb(label);
      if (verb) return { action: verb };
      node = node.parentElement;
    }
    return null;
  }

  function emit(info) {
    counter += 1;
    const action = {
      ext_id: `${site.app}:${PAGE_ID}:${Date.now()}:${counter}`,
      app: site.app,
      action: info.action,
      kind: "mutating",
      ts_ms: Date.now(),
    };
    try {
      chrome.runtime.sendMessage({ actions: [action] });
    } catch (e) {}
  }
})();
