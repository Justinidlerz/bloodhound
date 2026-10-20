# Changesets

For changes that require a release, run `pnpm changeset` from the repository root. Select a patch, minor, or major release for `@bloodhound-rs/core` and write a description for SDK consumers. Include the generated Markdown file in the PR.

Documentation-only or example-only changes do not require a changeset. Without a pending version change, the workflow only builds, checks, and packs.

PR beta releases do not write version files back to the branch. After a stable release succeeds, the workflow commits version files, changelogs, and consumed changesets, then pushes a version tag. See [Build and release](../docs/releasing.md) for the full configuration.
