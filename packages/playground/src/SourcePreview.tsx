import { useEffect, useState } from 'react';
import {
  viewerTarget,
  type ManifestIndex,
  type OriginalPreview,
  type OriginalSources,
  type RecordRow,
} from '@bloodhound-rs/manifest';
import type { ManifestComponents } from './components.js';
import { useMessages } from './messages.js';

interface Props {
  index: ManifestIndex;
  row: RecordRow;
  originals: OriginalSources;
  components: ManifestComponents;
  customLoader: boolean;
}

interface PreviewRequest {
  kind: 'generated' | 'original';
  load: (retry: boolean) => Promise<OriginalPreview>;
}

function Preview({
  load,
  components,
}: {
  load: (retry: boolean) => Promise<OriginalPreview>;
  components: ManifestComponents;
}) {
  const { text: t, label } = useMessages();
  const [result, setResult] = useState<OriginalPreview>({ status: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const { Button, SourceCode } = components;
  useEffect(() => {
    let active = true;
    setResult({ status: 'loading' });
    void load(attempt > 0).then(
      (value) => {
        if (active) setResult(value);
      },
      (error: unknown) => {
        if (active)
          setResult({
            status: 'error',
            error: error instanceof Error ? error.message : String(error),
          });
      },
    );
    return () => {
      active = false;
    };
  }, [load, attempt]);
  if (result.status === 'loading') return <p role="status">{t('source.loading')}</p>;
  if (result.status === 'error')
    return (
      <div role="alert">
        <p>{t('source.error', { message: result.error ?? '' })}</p>
        <Button onClick={() => setAttempt((value) => value + 1)}>{t('source.retry')}</Button>
      </div>
    );
  return (
    <section aria-label={t('source.label')}>
      <p className="bh-path">
        {result.path}
        {result.location ? `:${result.location.line}:${result.location.column}` : ''}
        {result.origin ? ` · ${t('source.origin', { value: label(result.origin) })}` : ''}
      </p>
      <SourceCode text={result.text!} path={result.path} location={result.location} />
    </section>
  );
}

export function SourcePreview({ index, row, originals, components, customLoader }: Props) {
  const { text: t } = useMessages();
  const { Button, Input } = components;
  const source = row.source ? index.sources.get(row.source) : undefined;
  const location = 'location' in row.data ? row.data.location : undefined;
  const target = viewerTarget(index, row, originals.workspace, customLoader);
  const [link, setLink] = useState('');
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<PreviewRequest>();
  const [reference, setReference] = useState(() => source && originals.reference(source));
  const hasPosition = source && location;

  function showOriginal() {
    if (!source || !location) return;
    setPreview({
      kind: 'original',
      load: async (retry) => {
        await originals.preview(row.id, source, location, retry);
        return originals.previews.get(row.id)!;
      },
    });
  }

  function associate() {
    if (!source) return;
    try {
      originals.link(source, link);
      setReference(originals.reference(source));
      setError('');
      showOriginal();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <div className="bh-source-preview">
      <div className="bh-controls bh-preview-switch" aria-label={t('source.mode')}>
        {target ? (
          <Button
            variant="toggle"
            aria-pressed={preview?.kind === 'generated'}
            onClick={() =>
              setPreview({
                kind: 'generated',
                load: async (): Promise<OriginalPreview> => {
                  const data = await originals.loadText(target.url, {
                    kind: 'source',
                    file: target.source.file,
                    limit: 8 * 1024 * 1024,
                  });
                  return {
                    status: 'ready',
                    text: data.text,
                    path: target.source.file ?? data.url,
                    location: target.location,
                  };
                },
              })
            }
          >
            {t('source.generated')}
          </Button>
        ) : null}
        {reference && hasPosition ? (
          <Button
            variant="toggle"
            aria-pressed={preview?.kind === 'original'}
            onClick={showOriginal}
          >
            {t('source.original')}
          </Button>
        ) : null}
      </div>
      {hasPosition ? (
        <details>
          <summary>{t('source.association')}</summary>
          {reference ? (
            <p className="bh-path">{reference.file ?? reference.url}</p>
          ) : (
            <p>{t('source.noMap')}</p>
          )}
          <div className="bh-controls">
            <Input
              value={link}
              onChange={setLink}
              placeholder={t('source.mapAddress')}
              aria-label={t('source.mapAddress')}
            />
            <Button onClick={associate} disabled={!link.trim()}>
              {t('source.link')}
            </Button>
            {reference ? (
              <Button
                onClick={() => {
                  originals.unlink(source);
                  setReference(undefined);
                  setPreview(undefined);
                  setError('');
                  setLink('');
                }}
              >
                {t('source.unlink')}
              </Button>
            ) : null}
          </div>
          {error ? <p role="alert">{t('source.error', { message: error })}</p> : null}
        </details>
      ) : null}
      {preview ? <Preview load={preview.load} components={components} /> : null}
    </div>
  );
}
