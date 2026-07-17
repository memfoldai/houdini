// Service worker: relays captured web-chat turns to the local native host over
// native messaging. The host name must match the installed host manifest
// (`ai.memfold.usage_monitor`).
//
// MV3 service workers are ephemeral (terminated when idle), so we do NOT hold a
// long-lived port. We open a fresh native-messaging connection per message,
// post, and let it close — the host is invoked per connection, reads the message
// from stdin, stores it, and exits when the port closes. This is robust to the
// worker being torn down between messages.

const HOST_NAME = "ai.memfold.usage_monitor";

chrome.runtime.onMessage.addListener((payload, _sender) => {
  if (!payload || !payload.tool || !Array.isArray(payload.turns)) return;
  try {
    const port = chrome.runtime.connectNative(HOST_NAME);
    port.onDisconnect.addListener(() => {
      if (chrome.runtime.lastError) {
        console.warn("[aum] native host disconnected:", chrome.runtime.lastError.message);
      }
    });
    port.postMessage(payload);
    // Close shortly after posting so the host flushes and exits.
    setTimeout(() => {
      try {
        port.disconnect();
      } catch (e) {}
    }, 500);
  } catch (e) {
    console.warn("[aum] connectNative failed:", e);
  }
});
