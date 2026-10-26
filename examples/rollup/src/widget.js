export function mountWidget(root) {
  if (!root) return;
  root.innerHTML = '<button type="button">Open</button>';
  // DOM property writes are reported as findings.
  document.title = 'Widget ready';
}
