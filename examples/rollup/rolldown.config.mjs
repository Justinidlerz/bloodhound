import { defineConfig } from 'rolldown';
import { bloodhound } from '@bloodhound-rs/rollup-plugin';
import { bloodhoundOptions, emitHtml, input, output } from './shared.mjs';

// Same plugin instance and options as rollup.config.mjs; only the bundler differs.
export default defineConfig({
  input,
  output: output('rolldown'),
  plugins: [emitHtml(), bloodhound(bloodhoundOptions)],
});
