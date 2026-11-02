import { useEffect, useState } from 'react';
import {
  viewerTarget,
  type ManifestIndex,
  type OriginalPreview,
  type OriginalSources,
  type RecordRow,
} from '@bloodhound-rs/manifest';
import type { ManifestComponents } from './components.js';

interface Props {
  index: ManifestIndex;
  row: RecordRow;
  originals: OriginalSources;
  components: ManifestComponents;
  customLoader: boolean;
}

function Preview({
  load,
  components,
}: {
  load: (retry: boolean) => Promise<OriginalPreview>;
  components: ManifestComponents;
}) {
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
  if (result.status === 'loading') return <p role="status">Loading source…</p>;
  if (result.status === 'error')
    return (
      <div role="alert">
        <p>{result.error}</p>
        <Button onClick={() => setAttempt((value) => value + 1)}>Retry</Button>
      </div>
    );
  return (
    <section aria-label="Source preview">
      <p className="bh-path">
        {result.path}
        {result.location ? `:${result.location.line}:${result.location.column}` : ''}
        {result.origin ? ` · ${result.origin}` : ''}
      </p>
      <SourceCode text={result.text!} path={result.path} location={result.location} />
    </section>
  );
}

export function SourcePreview({ index, row, originals, components, customLoader }: Props) {
  const { Button, Input } = components;
  const source = row.source ? index.sources.get(row.source) : undefined;
  const location = 'location' in row.data ? row.data.location : undefined;
  const target = viewerTarget(index, row, originals.workspace, customLoader);
  const [link, setLink] = useState('');
  const [error, setError] = useState('');
  const [load, setLoad] = useState<(retry: boolean) => Promise<OriginalPreview>>();
  const [reference, setReference] = useState(() => source && originals.reference(source));
  const hasPosition = source && location;

  function showOriginal() {
    if (!source || !location) return;
    setLoad(() => async (retry: boolean) => {
      await originals.preview(row.id, source, location, retry);
      return originals.previews.get(row.id)!;
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
      <div className="bh-controls">
        {target ? (
          <Button
            onClick={() =>
              setLoad(() => async (): Promise<OriginalPreview> => {
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
              })
            }
          >
            Preview generated code
          </Button>
        ) : null}
        {reference && hasPosition ? (
          <Button onClick={showOriginal}>Preview original source</Button>
        ) : null}
      </div>
      {hasPosition ? (
        <details>
          <summary>Source map association</summary>
          {reference ? (
            <p className="bh-path">{reference.file ?? reference.url}</p>
          ) : (
            <p>No source map linked.</p>
          )}
          <div className="bh-controls">
            <Input
              value={link}
              onChange={setLink}
              placeholder="Source map URL or project path"
              aria-label="Source map URL or project path"
            />
            <Button onClick={associate} disabled={!link.trim()}>
              Link source map
            </Button>
            {reference ? (
              <Button
                onClick={() => {
                  originals.unlink(source);
                  setReference(undefined);
                  setLoad(undefined);
                  setError('');
                  setLink('');
                }}
              >
                Unlink
              </Button>
            ) : null}
          </div>
          {error ? <p role="alert">{error}</p> : null}
        </details>
      ) : null}
      {load ? <Preview load={load} components={components} /> : null}
    </div>
  );
}
