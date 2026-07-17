// ISOLATED-world content script: the bridge between the MAIN-world interceptor
// (which can't call chrome.* APIs) and the extension service worker.
//
// It only accepts messages from THIS window's own interceptor (checks the origin
// and the __aum marker), then forwards the captured turn to the background
// service worker, which relays it to the local native host.

window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  if (event.origin !== location.origin) return;
  const data = event.data;
  if (!data || data.__aum !== true || !data.payload) return;
  try {
    chrome.runtime.sendMessage(data.payload);
  } catch (e) {
    // Service worker asleep/reloading; the next message reconnects it.
  }
});
