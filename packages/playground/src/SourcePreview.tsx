import { useEffect, useState } from 'react';
import {
  declarationRow,
  writeRows,
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
  point: 'write' | 'declaration';
  write: number;
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
  const declaration = 'declaration' in row.data ? row.data.declaration : undefined;
  const [link, setLink] = useState('');
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<PreviewRequest>();
  const [writeIndex, setWriteIndex] = useState(0);
  const [reference, setReference] = useState(() => source && originals.reference(source));
  const writes = writeRows(row);
  const moduleState = row.type === 'module-state-write';
  const writeRow = writes[writeIndex]!;
  const location = 'location' in writeRow.data ? writeRow.data.location : undefined;
  const hasPosition = source && location;
  const point = preview?.point ?? 'write';
  const activeRow = point === 'declaration' ? (declarationRow(writeRow) ?? writeRow) : writeRow;
  const target = viewerTarget(index, activeRow, originals.workspace, customLoader);
  const declarationPosition = declaration?.location.original ?? declaration?.location;

  function showOriginal(
    point: PreviewRequest['point'] = preview?.point ?? 'write',
    write = writeIndex,
  ) {
    const selected = writes[write]!;
    const location = 'location' in selected.data ? selected.data.location : undefined;
    if (!source || !location) return;
    setPreview({
      kind: 'original',
      point,
      write,
      load: async (retry) => {
        if (point === 'declaration' && declaration) {
          await originals.previewDeclaration(selected.id, source, location, declaration, retry);
          return originals.previews.get(`${selected.id}:declaration`)!;
        }
        await originals.preview(selected.id, source, location, retry);
        return originals.previews.get(selected.id)!;
      },
    });
  }

  function showGenerated(
    point: PreviewRequest['point'] = preview?.point ?? 'write',
    write = writeIndex,
  ) {
    const selected = writes[write]!;
    const selectedRow = point === 'declaration' ? (declarationRow(selected) ?? selected) : selected;
    const target = viewerTarget(index, selectedRow, originals.workspace, customLoader);
    if (!target) return;
    setPreview({
      kind: 'generated',
      point,
      write,
      load: async () => {
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
    });
  }

  function selectPoint(point: PreviewRequest['point'], write = writeIndex) {
    setWriteIndex(write);
    setPreview(undefined);
    if (preview?.kind === 'generated' || !reference) showGenerated(point, write);
    else showOriginal(point, write);
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
      {writes.length > 1 ? (
        <p>{t(moduleState ? 'record.writes' : 'record.occurrences', { count: writes.length })}</p>
      ) : null}
      {location && (declaration || writes.length > 1) ? (
        <div className="bh-controls bh-preview-switch" aria-label={t('source.positions')}>
          {writes.map((write, index) => {
            const position = 'location' in write.data ? write.data.location : undefined;
            const at = position?.original ?? position;
            return at ? (
              <Button
                key={write.id}
                variant="toggle"
                aria-pressed={point === 'write' && writeIndex === index}
                onClick={() => selectPoint('write', index)}
              >
                {t(
                  writes.length > 1
                    ? moduleState
                      ? 'source.writeOccurrence'
                      : 'source.occurrence'
                    : 'source.writePosition',
                  {
                    index: index + 1,
                    line: at.line,
                    column: at.column,
                  },
                )}
              </Button>
            ) : null;
          })}
          {declaration && declarationPosition ? (
            <Button
              variant="toggle"
              aria-pressed={point === 'declaration'}
              disabled={!target && !reference}
              onClick={() => selectPoint('declaration')}
            >
              {t('source.declarationPosition', {
                line: declarationPosition.line,
                column: declarationPosition.column,
                name: declaration.name,
              })}
            </Button>
          ) : null}
        </div>
      ) : null}
      {writes.length > 1 && 'snippet' in writeRow.data ? (
        <>
          <p>{writeRow.data.target}</p>
          {writeRow.data.detail ? <p>{writeRow.data.detail}</p> : null}
          {writeRow.data.polyfill ? (
            <p>
              {t('polyfill.detail', {
                target: writeRow.data.polyfill.guardTarget ?? writeRow.data.target,
              })}
            </p>
          ) : null}
          <pre className="bh-snippet">{writeRow.data.snippet}</pre>
        </>
      ) : null}
      <div className="bh-controls bh-preview-switch" aria-label={t('source.mode')}>
        {target ? (
          <Button
            variant="toggle"
            aria-pressed={preview?.kind === 'generated'}
            onClick={() => showGenerated()}
          >
            {t('source.generated')}
          </Button>
        ) : null}
        {reference && hasPosition ? (
          <Button
            variant="toggle"
            aria-pressed={preview?.kind === 'original'}
            onClick={() => showOriginal()}
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
      {preview ? (
        <Preview
          key={`${preview.write}:${preview.point}:${preview.kind}`}
          load={preview.load}
          components={components}
        />
      ) : null}
    </div>
  );
}
