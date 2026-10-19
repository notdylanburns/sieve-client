const tabUrls = new Map();

// Fires before the navigation request is even sent — this is what closes the gap
chrome.webNavigation.onBeforeNavigate.addListener((details) => {
  if (details.frameId === 0) {
    tabUrls.set(details.tabId, details.url);
  }
});

// Confirms/updates after commit (handles server-side redirects changing the final URL)
chrome.webNavigation.onCommitted.addListener((details) => {
  if (details.frameId === 0) {
    tabUrls.set(details.tabId, details.url);
  }
});

// SPA (pushState) navigations
chrome.webNavigation.onHistoryStateUpdated.addListener((details) => {
  if (details.frameId === 0) {
    tabUrls.set(details.tabId, details.url);
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  tabUrls.delete(tabId);
});

// Chromium has no flag for proxy credentials, so the device's come from managed policy storage
const proxyAuthAttempts = new Set();

chrome.webRequest.onAuthRequired.addListener(
  (details, callback) => {
    if (!details.isProxy) {
      callback({});
      return;
    }
    // A second challenge for the same request means the credentials were refused; stop there
    // rather than retrying forever.
    if (proxyAuthAttempts.has(details.requestId)) {
      callback({ cancel: true });
      return;
    }
    proxyAuthAttempts.add(details.requestId);

    chrome.storage.managed.get(["proxyUsername", "proxyPassword"]).then(
      ({ proxyUsername, proxyPassword }) =>
        callback(
          proxyUsername && proxyPassword
            ? { authCredentials: { username: proxyUsername, password: proxyPassword } }
            : { cancel: true },
        ),
      () => callback({ cancel: true }),
    );
  },
  { urls: ["<all_urls>"] },
  ["asyncBlocking"],
);

for (const event of [chrome.webRequest.onCompleted, chrome.webRequest.onErrorOccurred]) {
  event.addListener((details) => proxyAuthAttempts.delete(details.requestId), { urls: ["<all_urls>"] });
}

chrome.webRequest.onBeforeSendHeaders.addListener(
  (details) => {
    const headers = (details.requestHeaders || []).filter(
      (header) => header.name.toLowerCase() !== "x-proxy-origin-url"
    );
    const isDocument = details.type === "main_frame" || details.type === "sub_frame";
    const pageUrl = details.tabId >= 0 && !isDocument && tabUrls.get(details.tabId);
    if (pageUrl) {
      headers.push({ name: "X-Proxy-Origin-Url", value: pageUrl });
    }
    return { requestHeaders: headers };
  },
  { urls: ["<all_urls>"] },
  ["blocking", "requestHeaders"]
);
