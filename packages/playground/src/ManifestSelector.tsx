import { useEffect, useRef, useState } from 'react';
import { fetchText, parseManifest, safeUrl, type Manifest } from '@bloodhound-rs/manifest';
import { defaultComponents, type ManifestComponents } from './components.js';

export interface ManifestResource {
  manifest: Manifest;
  name: string;
  url?: string;
  notices: string[];
}
export interface ManifestSelectorProps {
  onChange: (resource: ManifestResource) => void;
  components?: Partial<ManifestComponents>;
  /** Host-controlled URL fetch, including authentication when needed. */
  loadManifest?: (url: string) => Promise<{ text: string; url: string }>;
  /** Supply a worker-backed parser for large reports. */
  parse?: (
    text: string,
  ) => ReturnType<typeof parseManifest> | Promise<ReturnType<typeof parseManifest>>;
  maxBytes?: number;
  className?: string;
}
const defaultLoadManifest = (url: string) => fetchText(url, 64 * 1024 * 1024);

/** Resource input only. It does not render a report or load any source code. */
export function ManifestSelector({
  onChange,
  components,
  loadManifest = defaultLoadManifest,
  parse = parseManifest,
  maxBytes = 64 * 1024 * 1024,
  className = '',
}: ManifestSelectorProps) {
  const { Button, Input } = { ...defaultComponents, ...components };
  const [url, setUrl] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const fileInput = useRef<HTMLInputElement>(null);
  const request = useRef(0);
  useEffect(
    () => () => {
      request.current++;
    },
    [],
  );

  async function open(file?: File) {
    const id = ++request.current;
    setLoading(true);
    setError('');
    try {
      if (file && file.size > maxBytes) throw new Error('Manifest exceeds the size limit.');
      const address = file ? undefined : safeUrl(url.trim());
      if (!file && !address) throw new Error('Enter an HTTP(S) manifest URL.');
      const data = file
        ? { text: await file.text(), url: undefined }
        : await loadManifest(address!);
      if (new TextEncoder().encode(data.text).length > maxBytes)
        throw new Error('Manifest exceeds the size limit.');
      const parsed = await parse(data.text);
      if (id === request.current)
        onChange({ ...parsed, name: file?.name ?? address!, url: data.url });
    } catch (error) {
      if (id === request.current) setError(error instanceof Error ? error.message : String(error));
    } finally {
      if (id === request.current) setLoading(false);
    }
  }

  return (
    <section
      className={`bh-selector ${className}`}
      aria-label="Select manifest"
      aria-busy={loading}
    >
      <div className="bh-controls">
        <input
          ref={fileInput}
          type="file"
          accept=".json,application/json"
          hidden
          onChange={(event) => {
            const file = event.currentTarget.files?.[0];
            event.currentTarget.value = '';
            if (file) void open(file);
          }}
        />
        <Button onClick={() => fileInput.current?.click()}>Open manifest file</Button>
        <Input
          value={url}
          onChange={setUrl}
          placeholder="https://…/bloodhound.manifest.json"
          aria-label="Manifest URL"
        />
        <Button disabled={!url.trim()} onClick={() => void open()}>
          Load URL
        </Button>
      </div>
      {loading ? <p role="status">Loading manifest…</p> : null}
      {error ? <p role="alert">{error}</p> : null}
    </section>
  );
}
