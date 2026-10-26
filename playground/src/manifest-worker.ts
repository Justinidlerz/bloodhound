import { parseManifest } from './manifest.js';

// The JSON parser runs off the UI thread. Uploaded content is only treated as data.
self.onmessage = async (event: MessageEvent<{ file: File }>) => {
  try {
    const parsed = parseManifest(await event.data.file.text());
    self.postMessage({ ok: true, ...parsed });
  } catch (error) {
    self.postMessage({
      ok: false,
      error: error instanceof Error ? error.message : 'Unable to read this file.',
    });
  }
};
