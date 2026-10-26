export function start() {
  // Third-party style global queue, written from a dynamic chunk.
  window.dataLayer = window.dataLayer || [];
  window.dataLayer.push({ event: 'start' });
  delete window.__legacyTracker;
}
