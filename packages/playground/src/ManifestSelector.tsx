import { useEffect, useRef, useState } from 'react';
import { fetchText, parseManifest, safeUrl, type Manifest } from '@bloodhound-rs/manifest';
import { defaultComponents, type ManifestComponents } from './components.js';
import { englishMessages } from './locales/en.js';
import {
  MessagesProvider,
  useMessages,
  type PlaygroundMessages,
  type PlaygroundMessageKey,
  type PlaygroundMessageParams,
} from './messages.js';

export interface ManifestResource {
  manifest: Manifest;
  name: string;
  url?: string;
  notices: string[];
}
export interface ManifestSelectorProps {
  onChange: (resource: ManifestResource) => void;
  components?: Partial<ManifestComponents>;
  messages?: Partial<PlaygroundMessages>;
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
export function ManifestSelector({ messages = englishMessages, ...props }: ManifestSelectorProps) {
  return (
    <MessagesProvider messages={messages}>
      <Selector {...props} />
    </MessagesProvider>
  );
}

function Selector({
  onChange,
  components,
  loadManifest = defaultLoadManifest,
  parse = parseManifest,
  maxBytes = 64 * 1024 * 1024,
  className = '',
}: ManifestSelectorProps) {
  const { text: t } = useMessages();
  const { Button, Input } = { ...defaultComponents, ...components };
  const [url, setUrl] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<{
    key: PlaygroundMessageKey;
    params?: PlaygroundMessageParams;
  }>();
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
    setError(undefined);
    try {
      if (file && file.size > maxBytes) {
        setError({ key: 'selector.tooLarge' });
        return;
      }
      const address = file ? undefined : safeUrl(url.trim());
      if (!file && !address) {
        setError({ key: 'selector.invalidUrl' });
        return;
      }
      const data = file
        ? { text: await file.text(), url: undefined }
        : await loadManifest(address!);
      if (new TextEncoder().encode(data.text).length > maxBytes) {
        if (id === request.current) setError({ key: 'selector.tooLarge' });
        return;
      }
      const parsed = await parse(data.text);
      if (id === request.current)
        onChange({ ...parsed, name: file?.name ?? address!, url: data.url });
    } catch (error) {
      if (id === request.current)
        setError({
          key: 'selector.error',
          params: { message: error instanceof Error ? error.message : String(error) },
        });
    } finally {
      if (id === request.current) setLoading(false);
    }
  }

  return (
    <section
      className={`bh-selector ${className}`}
      aria-label={t('selector.label')}
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
        <Button onClick={() => fileInput.current?.click()}>{t('selector.openFile')}</Button>
        <Input
          value={url}
          onChange={setUrl}
          placeholder={t('selector.urlPlaceholder')}
          aria-label={t('selector.url')}
        />
        <Button disabled={!url.trim()} onClick={() => void open()}>
          {t('selector.loadUrl')}
        </Button>
      </div>
      {loading ? <p role="status">{t('selector.loading')}</p> : null}
      {error ? <p role="alert">{t(error.key, error.params)}</p> : null}
    </section>
  );
}
