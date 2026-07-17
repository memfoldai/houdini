// MAIN-world content script (runs in the PAGE's context, at document_start).
//
// We wrap the page's own `window.fetch` to DETECT each conversation exchange and
// read the user's prompt from the request body (reliable, structured). For the
// assistant REPLY we read the completed message from the DOM after the stream
// ends, rather than parsing the provider's internal SSE — that internal format
// is undocumented and changes (it silently broke reply capture once), whereas the
// rendered message is stable and is what the user actually saw. The conversation
// id comes from the stable page URL. This works in background tabs because the
// page's fetch and DOM update regardless of tab focus. We never throw into the
// page and never disturb the response (the page's own fetch result is untouched).

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
      // ChatGPT puts the conversation id in the page path (/c/<id>).
      conversationId: () => {
        const m = location.pathname.match(/\/c\/([\w-]+)/);
        return m ? m[1] : null;
      },
      // The rendered assistant turns carry a stable author-role attribute.
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
      } catch (e) {
        /* never break the page */
      }
    }).catch(() => {});
    return p;
  };

  async function captureExchange(site, url, reqBody, respClone) {
    const prompt = site.extractPrompt(reqBody);
    const convId = site.conversationId(url) || PAGE_ID;

    // Drain the response clone to know when streaming has finished (we don't
    // parse it — the reply comes from the DOM once it's rendered).
    const reader = respClone.body && respClone.body.getReader();
    if (reader) {
      try {
        for (;;) {
          const { done } = await reader.read();
          if (done) break;
        }
      } catch (e) {
        /* ignore */
      }
    }
    // Let the final tokens render, then read the completed assistant message.
    await sleep(800);
    const reply = site.replyFromDom ? site.replyFromDom() : "";

    // Both turns in ONE message so their order is fixed (host appends in order).
    const turns = [];
    if (prompt && prompt.trim()) turns.push({ role: "user", text: prompt, ts_ms: Date.now() });
    if (reply && reply.trim()) turns.push({ role: "assistant", text: reply, ts_ms: Date.now() });
    if (turns.length) {
      window.postMessage({ __aum: true, payload: { tool: site.tool, external_id: convId, turns } }, location.origin);
    }
  }

  function asJson(body) {
    if (typeof body === "string") return tryJson(body);
    return null; // Blob/FormData/stream bodies aren't used by these endpoints.
  }
  function tryJson(s) {
    try {
      return JSON.parse(s);
    } catch (e) {
      return null;
    }
  }
})();
