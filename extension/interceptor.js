// MAIN-world content script (runs in the PAGE's context, at document_start).
//
// This is the reliable way to read a web AI chat: wrap the page's own
// `window.fetch` so we see the conversation API request (your prompt) and the
// streamed SSE response (the reply) — the same structured data the site's own
// code consumes, before React renders it, and in background tabs too (the page's
// fetch runs regardless of tab focus). We read a CLONE of the response, so the
// page is never affected, and never throw into the page.
//
// It posts captured turns via window.postMessage to the ISOLATED-world relay
// (MAIN world can't call chrome.* APIs). Per-site parsing lives in SITES; the
// endpoint/SSE shapes are reverse-engineered (not official contracts), so each
// parser is defensive and easy to adjust when a site changes.

(function () {
  "use strict";

  // Stable-per-page-load fallback id, so an exchange whose real conversation id
  // we can't read still groups its prompt with its reply.
  const PAGE_ID = "page-" + Math.random().toString(36).slice(2);

  const SITES = {
    "chatgpt-web": {
      hosts: ["chatgpt.com", "chat.openai.com"],
      isConversation: (url, method) =>
        method === "POST" && /\/backend-api\/(f\/)?conversation(\?|$)/.test(url),
      conversationIdFromUrl: () => null,
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
      // ChatGPT streams the CUMULATIVE assistant text in each event, plus the
      // conversation_id — so take the latest cumulative, not a concatenation.
      cumulative: true,
      parseEvent: (line) => {
        if (!line.startsWith("data:")) return null;
        const data = line.slice(5).trim();
        if (!data || data === "[DONE]") return null;
        const j = tryJson(data);
        if (!j) return null;
        const out = {};
        if (j.conversation_id) out.convId = j.conversation_id;
        const msg = j.message;
        if (msg && msg.author && msg.author.role === "assistant" && msg.content && Array.isArray(msg.content.parts)) {
          out.cumulative = msg.content.parts.filter((p) => typeof p === "string").join("");
        }
        return out;
      },
    },

    "claude-web": {
      hosts: ["claude.ai"],
      isConversation: (url, method) =>
        method === "POST" && /\/chat_conversations\/[^/]+\/completion/.test(url),
      conversationIdFromUrl: (url) => {
        const m = url.match(/chat_conversations\/([^/]+)\/completion/);
        return m ? m[1] : null;
      },
      extractPrompt: (body) => {
        const j = asJson(body);
        return j && typeof j.prompt === "string" ? j.prompt : null;
      },
      // Claude streams DELTAS (concatenate).
      cumulative: false,
      parseEvent: (line) => {
        if (!line.startsWith("data:")) return null;
        const data = line.slice(5).trim();
        if (!data) return null;
        const j = tryJson(data);
        if (!j) return null;
        if (typeof j.completion === "string") return { delta: j.completion };
        if (j.type === "content_block_delta" && j.delta && typeof j.delta.text === "string") {
          return { delta: j.delta.text };
        }
        return null;
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
    let convId = site.conversationIdFromUrl(url);
    let assistant = "";

    const reader = respClone.body && respClone.body.getReader();
    if (reader) {
      const dec = new TextDecoder();
      let buf = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += dec.decode(value, { stream: true });
        let nl;
        while ((nl = buf.indexOf("\n")) >= 0) {
          const line = buf.slice(0, nl);
          buf = buf.slice(nl + 1);
          const ev = site.parseEvent(line.trim());
          if (!ev) continue;
          if (ev.convId) convId = ev.convId;
          if (site.cumulative) {
            if (ev.cumulative != null) assistant = ev.cumulative;
          } else if (ev.delta) {
            assistant += ev.delta;
          }
        }
      }
    }

    // Send both turns in ONE message so their order is fixed (the host appends
    // in array order); two separate messages could be delivered out of order.
    const id = convId || PAGE_ID;
    const turns = [];
    if (prompt && prompt.trim()) turns.push({ role: "user", text: prompt, ts_ms: Date.now() });
    if (assistant && assistant.trim()) turns.push({ role: "assistant", text: assistant, ts_ms: Date.now() });
    if (turns.length) {
      window.postMessage({ __aum: true, payload: { tool: site.tool, external_id: id, turns } }, location.origin);
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
