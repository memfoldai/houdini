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
  function matchVerb(label) {
    const l = (label || "").toLowerCase();
    if (!l) return null;
    if (l.includes("send")) return "send";
    if (l.includes("reply")) return "reply";
    if (l.includes("forward")) return "forward";
    if (l.includes("archive")) return "archive";
    if (l.includes("spam")) return "spam";
    if (l.includes("snooze")) return "snooze";
    if (l.includes("trash") || l.includes("delete") || l.includes("remove")) return "delete";
    if (l.includes("move")) return "move";
    if (l.includes("share")) return "share";
    if (l.includes("rename")) return "rename";
    if (l.includes("download")) return "download";
    if (l.includes("upload")) return "upload";
    if (l.includes("compose")) return "compose";
    return null;
  }
  if (typeof globalThis !== "undefined") {
    globalThis.__bbMatchVerb = matchVerb;
  }

  const site = APPS.find((a) => location.hostname === a.app);
  if (!site) return;
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
