import { useCallback, useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent, ReactNode } from 'react';
import { useMessages } from './messages.js';

type Side = 'groups' | 'detail';
interface Range {
  min: number;
  max: number;
  now: number;
}
interface Props {
  groups: ReactNode;
  results: ReactNode;
  detail: ReactNode;
  selected?: string;
}

/** Layout and resize state stay local to each embedded viewer. */
export function ViewerLayout({ groups, results, detail, selected }: Props) {
  const { text: t } = useMessages();
  const id = useId();
  const layoutRef = useRef<HTMLDivElement>(null);
  const groupsRef = useRef<HTMLElement>(null);
  const resultsRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLElement>(null);
  const drag = useRef<{ side: Side; pointerId: number; x: number; width: number }>();
  const returnFocus = useRef<HTMLElement>();
  const [ranges, setRanges] = useState<Partial<Record<Side, Range>>>({});

  const measure = useCallback(() => {
    const layout = layoutRef.current;
    const panels = { groups: groupsRef.current, detail: detailRef.current };
    const records = resultsRef.current;
    if (!layout || !panels.groups || !panels.detail || !records) return;
    const splitters = Array.from(layout.querySelectorAll<HTMLElement>(':scope > .bh-splitter'));
    // The container's compact layout has no horizontal splitters.
    if (!splitters[0] || getComputedStyle(splitters[0]).display === 'none') return;
    const handles = splitters.reduce((width, el) => width + el.getBoundingClientRect().width, 0);
    const recordMin = parseFloat(getComputedStyle(records).minWidth);
    const next = {} as Record<Side, Range>;
    for (const side of ['groups', 'detail'] as const) {
      const panel = panels[side]!;
      const other = panels[side === 'groups' ? 'detail' : 'groups']!;
      const min = parseFloat(getComputedStyle(panel).minWidth);
      next[side] = {
        min,
        max: Math.max(
          min,
          layout.clientWidth - handles - recordMin - other.getBoundingClientRect().width,
        ),
        now: panel.getBoundingClientRect().width,
      };
    }
    return next;
  }, []);

  const sync = useCallback(() => {
    const layout = layoutRef.current;
    if (!layout) return;
    for (const side of ['groups', 'detail'] as const) {
      const range = measure()?.[side];
      const property = `--bh-${side}-width`;
      if (range && layout.style.getPropertyValue(property)) {
        layout.style.setProperty(
          property,
          `${Math.min(range.max, Math.max(range.min, range.now))}px`,
        );
      }
    }
    const next = measure();
    if (next) setRanges(next);
  }, [measure]);

  useEffect(() => {
    const layout = layoutRef.current;
    if (!layout) return;
    const observer = new ResizeObserver(sync);
    observer.observe(layout);
    sync();
    return () => observer.disconnect();
  }, [sync]);

  useEffect(() => {
    const inspector = detailRef.current;
    if (!inspector) return;
    inspector.scrollTop = 0;
    if (selected) {
      const active = document.activeElement;
      if (
        active instanceof HTMLElement &&
        layoutRef.current?.contains(active) &&
        !inspector.contains(active)
      ) {
        returnFocus.current = active;
      }
      if (!measure()) inspector.focus();
    } else if (returnFocus.current?.isConnected) {
      returnFocus.current.focus({ preventScroll: true });
      returnFocus.current = undefined;
    }
  }, [selected, measure]);

  function resize(side: Side, width: number) {
    const measured = measure();
    const layout = layoutRef.current;
    if (!measured || !layout) return;
    const range = measured[side];
    const other = side === 'groups' ? 'detail' : 'groups';
    // Keep the opposite divider still while resizing this panel.
    layout.style.setProperty(`--bh-${other}-width`, `${measured[other].now}px`);
    layout.style.setProperty(
      `--bh-${side}-width`,
      `${Math.min(range.max, Math.max(range.min, width))}px`,
    );
    sync();
  }

  function start(event: PointerEvent<HTMLDivElement>, side: Side) {
    if (event.button !== 0 || !event.isPrimary) return;
    const range = measure()?.[side];
    if (!range) return;
    event.preventDefault();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { side, pointerId: event.pointerId, x: event.clientX, width: range.now };
    layoutRef.current?.setAttribute('data-resizing', '');
  }

  function move(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current;
    if (!current || current.pointerId !== event.pointerId) return;
    const delta = (event.clientX - current.x) * (current.side === 'groups' ? 1 : -1);
    resize(current.side, current.width + delta);
  }

  function stop(event: PointerEvent<HTMLDivElement>) {
    if (drag.current?.pointerId !== event.pointerId) return;
    drag.current = undefined;
    layoutRef.current?.removeAttribute('data-resizing');
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function reset() {
    layoutRef.current?.style.removeProperty('--bh-groups-width');
    layoutRef.current?.style.removeProperty('--bh-detail-width');
    sync();
  }

  function keydown(event: KeyboardEvent<HTMLDivElement>, side: Side) {
    const range = measure()?.[side];
    if (!range) return;
    let width: number;
    if (event.key === 'Home') width = range.min;
    else if (event.key === 'End') width = range.max;
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      const direction = (event.key === 'ArrowRight' ? 1 : -1) * (side === 'groups' ? 1 : -1);
      width = range.now + direction * (event.shiftKey ? 32 : 8);
    } else return;
    event.preventDefault();
    resize(side, width);
  }

  function splitter(side: Side, label: string) {
    const range = ranges[side];
    return (
      <div
        className="bh-splitter"
        role="separator"
        tabIndex={0}
        aria-label={label}
        aria-orientation="vertical"
        aria-controls={`${id}-${side}`}
        aria-valuemin={range && Math.round(range.min)}
        aria-valuemax={range && Math.round(range.max)}
        aria-valuenow={range && Math.round(range.now)}
        onPointerDown={(event) => start(event, side)}
        onPointerMove={move}
        onPointerUp={stop}
        onPointerCancel={stop}
        onLostPointerCapture={stop}
        onDoubleClick={reset}
        onKeyDown={(event) => keydown(event, side)}
      />
    );
  }

  return (
    <div className="bh-layout" ref={layoutRef} data-detail-open={Boolean(selected)}>
      <aside
        className="bh-groups"
        ref={groupsRef}
        id={`${id}-groups`}
        aria-label={t('layout.groups')}
      >
        {groups}
      </aside>
      {splitter('groups', t('layout.resizeGroups'))}
      <div className="bh-results" ref={resultsRef}>
        {results}
      </div>
      {splitter('detail', t('layout.resizeDetails'))}
      <aside
        className="bh-inspector"
        ref={detailRef}
        id={`${id}-detail`}
        aria-label={t('layout.details')}
        tabIndex={-1}
      >
        {detail ?? <p className="bh-muted">{t('layout.empty')}</p>}
      </aside>
    </div>
  );
}
