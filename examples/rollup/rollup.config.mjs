import { defineConfig } from 'rollup';
import { bloodhound } from '@bloodhound-rs/rollup-plugin';
import { bloodhoundOptions, emitHtml, input, output } from './shared.mjs';

export default defineConfig({
  input,
  output: output('rollup'),
  plugins: [emitHtml(), bloodhound(bloodhoundOptions)],
});
