import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ManifestPlayground, type ManifestResource } from '@bloodhound-rs/playground';
import { fetchText, parseManifest, type ProjectWorkspace } from '@bloodhound-rs/manifest';
import { resolveManifestParam } from './autoload.js';

function App({
  initial,
  workspace,
  error,
}: {
  initial?: ManifestResource;
  workspace?: ProjectWorkspace;
  error?: string;
}) {
  const [resource, setResource] = useState(initial);
  return (
    <main className="react-playground">
      <header>
        <h1>Bloodhound React Playground</h1>
        <a href={`./${window.location.search}`}>Open classic Playground</a>
      </header>
      <ManifestPlayground initialResource={initial} onChange={setResource} workspace={workspace} />
      {error && !resource ? <p role="alert">{error}</p> : null}
      {!resource ? (
        <p>Open a Manifest v2 report to browse findings, dependencies and original source.</p>
      ) : null}
    </main>
  );
}

const workspaceReady: Promise<ProjectWorkspace | undefined> = fetch('/__workspace')
  .then((response) => (response.ok ? response.json() : undefined))
  .catch(() => undefined);
const params = new URL(window.location.href).searchParams;
let initial: ManifestResource | undefined;
let error: string | undefined;
const value = params.get('manifest');
if (value) {
  try {
    const target = resolveManifestParam(value, window.location.href, params.get('name'));
    if (target.kind !== 'fetch')
      throw new Error('Use pnpm playground --manifest <path> for local files.');
    const data = await fetchText(target.url, 64 * 1024 * 1024);
    initial = { ...parseManifest(data.text), name: target.name, url: data.url };
  } catch (caught) {
    error = caught instanceof Error ? caught.message : String(caught);
  }
}
createRoot(document.getElementById('root')!).render(
  <App initial={initial} error={error} workspace={await workspaceReady} />,
);
