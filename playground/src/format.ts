/** Every interpolated value is escaped unless it was built with this template. */
export class Markup {
  constructor(readonly value: string) {}
  toString() {
    return this.value;
  }
}
function escape(value: unknown): string {
  if (value instanceof Markup) return value.value;
  if (Array.isArray(value)) return value.map(escape).join('');
  if (value === null || value === undefined) return '';
  return String(value).replace(
    /[&<>"']/g,
    (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]!,
  );
}
export function html(parts: TemplateStringsArray, ...values: unknown[]): Markup {
  return new Markup(
    parts.reduce(
      (result, part, index) => result + part + (index < values.length ? escape(values[index]) : ''),
      '',
    ),
  );
}
export const count = (value: number) => new Intl.NumberFormat('en-US').format(value);
export function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 ** 2).toFixed(2)} MiB`;
}
export const time = (value: number) =>
  value < 1000 ? `${Math.round(value)} ms` : `${(value / 1000).toFixed(2)} s`;
export const human = (value: string) =>
  value.replaceAll('-', ' ').replace(/^./, (char) => char.toUpperCase());
export function badge(value: string): Markup {
  const color = ['error', 'failed', 'unresolved'].includes(value)
    ? 'danger'
    : ['warning', 'partial', 'gap', 'medium'].includes(value)
      ? 'amber'
      : ['completed', 'resolved', 'high'].includes(value)
        ? 'green'
        : 'neutral';
  return html`<span class="badge ${color}">${human(value)}</span>`;
}
export function safeUrl(value: string | undefined): string | undefined {
  if (!value) return undefined;
  try {
    const url = new URL(value);
    return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}
export function externalLink(url: string | undefined): Markup {
  const href = safeUrl(url);
  return href
    ? html`<a class="text-link" href="${href}" target="_blank" rel="noopener noreferrer"
        >Open in new tab ↗</a
      >`
    : html``;
}
export function empty(title: string, description: string): Markup {
  return html`<div class="empty-panel">
    <div class="empty-mark">⌕</div>
    <h3>${title}</h3>
    <p>${description}</p>
  </div>`;
}
