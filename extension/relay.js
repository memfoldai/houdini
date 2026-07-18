window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  if (event.origin !== location.origin) return;
  const data = event.data;
  if (!data || data.__aum !== true || !data.payload) return;
  try {
    chrome.runtime.sendMessage(data.payload);
  } catch (e) {}
});
