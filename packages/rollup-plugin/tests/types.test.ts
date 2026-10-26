import type { Plugin as RolldownPlugin } from 'rolldown';
import type { Plugin as RollupPlugin } from 'rollup';
import { bloodhound } from '../src/index.js';

// The plugin must be usable in both bundlers without casts.
const rollupPlugin: RollupPlugin = bloodhound();
const rolldownPlugin: RolldownPlugin = bloodhound({ globals: 'error' });
export { rolldownPlugin, rollupPlugin };
