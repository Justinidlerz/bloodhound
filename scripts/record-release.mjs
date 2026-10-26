import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { releasePlanPath } from './workspace-packages.mjs';

const plan = JSON.parse(readFileSync(releasePlanPath, 'utf8'));
if (plan.mode !== 'latest' || plan.packages.length === 0 || plan.tags.length === 0) {
  throw new Error('A completed stable release plan is required');
}
const branch = process.env.RELEASE_BRANCH;
if (!branch) throw new Error('RELEASE_BRANCH is required');
const git = (args) => execFileSync('git', args, { stdio: 'inherit' });
git(['check-ref-format', `refs/heads/${branch}`]);
git(['add', '-A', '--', '.changeset', ...plan.metadataFiles]);
git(['commit', '-m', `chore: release ${plan.tags.join(', ')} [skip ci]`]);
for (const tag of plan.tags) git(['tag', '-a', tag, '-m', `Release ${tag}`]);
git([
  'push',
  '--atomic',
  'origin',
  `HEAD:${branch}`,
  ...plan.tags.map((tag) => `refs/tags/${tag}`),
]);
