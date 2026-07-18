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
    setTimeout(() => {
      try {
        port.disconnect();
      } catch (e) {}
    }, 500);
  } catch (e) {
    console.warn("[aum] connectNative failed:", e);
  }
});
