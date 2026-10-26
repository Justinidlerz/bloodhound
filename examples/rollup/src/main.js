import { mountWidget } from './widget.js';

// Expected: the build stamps a version onto the page. Allow-listed in rollup.config.mjs.
window.__APP_VERSION__ = '1.0.0';

// Unexpected: a shared config object leaks onto the global scope.
globalThis.sharedConfig = { endpoint: '/api' };

mountWidget(document.getElementById('app'));

// A lazily loaded chunk is followed through the emitted bundle.
if (location.search.includes('analytics')) {
  import('./analytics.js').then((module) => module.start());
}
