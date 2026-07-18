(function () {
  "use strict";

  const PAGE_ID = "page-" + Math.random().toString(36).slice(2);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  const SITES = {
    "chatgpt-web": {
      hosts: ["chatgpt.com", "chat.openai.com"],
      isConversation: (url, method) =>
        method === "POST" && /\/backend-api\/(f\/)?conversation(\?|$)/.test(url),
      extractPrompt: (body) => {
        const j = asJson(body);
        if (!j || !Array.isArray(j.messages)) return null;
        for (let i = j.messages.length - 1; i >= 0; i--) {
          const m = j.messages[i];
          const role = (m.author && m.author.role) || m.role;
          if (role === "user") {
            const parts = m.content && m.content.parts;
            if (Array.isArray(parts)) return parts.filter((p) => typeof p === "string").join("\n");
          }
        }
        return null;
      },
      conversationId: () => {
        const m = location.pathname.match(/\/c\/([\w-]+)/);
        return m ? m[1] : null;
      },
      replyFromDom: () => {
        const els = document.querySelectorAll('[data-message-author-role="assistant"]');
        const last = els[els.length - 1];
        return last ? (last.innerText || "").trim() : "";
      },
    },

    "claude-web": {
      hosts: ["claude.ai"],
      isConversation: (url, method) =>
        method === "POST" && /\/chat_conversations\/[^/]+\/completion/.test(url),
      extractPrompt: (body) => {
        const j = asJson(body);
        return j && typeof j.prompt === "string" ? j.prompt : null;
      },
      conversationId: (url) => {
        const m = (url + " " + location.pathname).match(/(?:chat_conversations\/|\/chat\/)([\w-]+)/);
        return m ? m[1] : null;
      },
      replyFromDom: () => {
        const els = document.querySelectorAll('.font-claude-message, [data-testid="assistant-message"]');
        const last = els[els.length - 1];
        return last ? (last.innerText || "").trim() : "";
      },
    },
  };

  const site = Object.values(SITES).find((s) => s.hosts.includes(location.hostname));
  if (!site) return;
  site.tool = Object.keys(SITES).find((k) => SITES[k] === site);

  const origFetch = window.fetch;
  window.fetch = function (input, init) {
    const url = typeof input === "string" || input instanceof URL ? String(input) : (input && input.url) || "";
    const method = ((init && init.method) || (input && input.method) || "GET").toUpperCase();
    const reqBody = init && init.body;
    const p = origFetch.apply(this, arguments);
    p.then((resp) => {
      try {
        if (site.isConversation(url, method)) captureExchange(site, url, reqBody, resp.clone());
      } catch (e) {}
    }).catch(() => {});
    return p;
  };

  async function captureExchange(site, url, reqBody, respClone) {
    const prompt = site.extractPrompt(reqBody);

    const reader = respClone.body && respClone.body.getReader();
    if (reader) {
      try {
        for (;;) {
          const { done } = await reader.read();
          if (done) break;
        }
      } catch (e) {}
    }

    const reply = await waitForStableReply(site);
    const convId = site.conversationId(url) || PAGE_ID;

    if (!reply) {
      console.warn("[aum] captured a", site.tool, "prompt but no assistant reply; the DOM selector may need updating");
    }

    const turns = [];
    if (prompt && prompt.trim()) turns.push({ role: "user", text: prompt, ts_ms: Date.now() });
    if (reply && reply.trim()) turns.push({ role: "assistant", text: reply, ts_ms: Date.now() });
    if (turns.length) {
      window.postMessage({ __aum: true, payload: { tool: site.tool, external_id: convId, turns } }, location.origin);
    }
  }

  async function waitForStableReply(site) {
    if (!site.replyFromDom) return "";
    let last = "";
    let stableFor = 0;
    for (let i = 0; i < 24; i++) {
      await sleep(500);
      const text = site.replyFromDom();
      if (text && text === last) {
        if (++stableFor >= 2) return text;
      } else {
        stableFor = 0;
        last = text;
      }
    }
    return last;
  }

  function asJson(body) {
    if (typeof body === "string") return tryJson(body);
    return null;
  }
  function tryJson(s) {
    try {
      return JSON.parse(s);
    } catch (e) {
      return null;
    }
  }
})();
