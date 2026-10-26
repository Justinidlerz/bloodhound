# Build and publish releases

This guide covers Changesets versioning and automatic npm publishing through `.github/workflows/release.yml`.

## Workflow

| Trigger                                                                                                                                  | Behavior                                                                                                                                                               |
| ---------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PR opened, updated, reopened, or marked ready                                                                                            | Check formatting, run script tests, build five native targets, run Rust and SDK tests, check types, run JS / HTML examples, and assemble all public workspace packages |
| Non-draft PR from the same repository, with both author and triggering actor matching `NPM_RELEASE_USER`, and a changeset version change | Publish a beta version without writing version files back to the PR branch                                                                                             |
| Push to `main` by `NPM_RELEASE_USER`, with a changeset version change                                                                    | Publish a stable version, then commit version metadata, consume Changesets, and create a release tag                                                                   |
| Manual run with an empty `publish_run_id`                                                                                                | Check, build, and pack without publishing                                                                                                                              |
| Manual run with `publish_run_id`                                                                                                         | Recover missing packages from the original release archives                                                                                                            |

Native targets are defined in `.github/native-targets.json`: Linux glibc x64 / arm64, macOS x64 / arm64, and Windows x64. Keep `package.json` → `napi.targets` aligned with this matrix. `pnpm platforms:sync` checks that they match and uses napi-rs to generate `npm/<platform>/package.json`.

Core contains the loader, declarations, and documentation. Each of the five platform packages contains one binary and restricts installation with `os`, `cpu`, and `libc`. Core uses `workspace:*` optional dependencies in the repository. Packing converts these to exact versions.

Assembly checks that all five binaries exist, core contains no binaries, and each platform package contains exactly one binary. The Rollup plugin is built against the collected SDK and packed separately with no native binary. All public workspace packages are discovered from pnpm metadata, yielding seven archives for the current workspace. The release manifest includes SHA-512 integrity values.

The publisher verifies all archives before contacting npm, then publishes the versions selected by Changesets in runtime dependency order using `npm publish`: platform packages, core, then the plugin when those packages changed. After each publish, it checks public registry integrity every 15 seconds for up to 10 minutes. It stops on a timeout, mismatched content, or a failed command. Core is published only after all platform versions are verified. No npm approval click is required for new direct releases.

## Use Changesets

Run this command from the repository root:

```sh
pnpm changeset
```

Select a patch, minor, or major release for the affected packages and describe the change for consumers. Core and its five platform packages form a fixed group. `@bloodhound-rs/rollup-plugin` is versioned independently; a plugin-only changeset also triggers publication. Include the generated `.changeset/*.md` file in the PR. Documentation-only or example-only changes do not require a changeset. Without a version change, the workflow does not publish packages.

| Stage                                 | Version handling                                                                                                                                                                               |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PR                                    | Calculate the next version, append `-beta.<PR-number>.<run-ID>.<attempt>`, and publish with the `beta` tag                                                                                     |
| Main branch                           | Update versions and changelogs, consume changeset files, and publish with the `latest` tag                                                                                                     |
| Before native builds                  | Synchronize the npm version to `native/Cargo.toml` and `Cargo.lock`. Reports use the compiled Rust version                                                                                     |
| After verified main-branch publishing | Commit metadata with `chore: release <version> [skip ci]`, create `v<version>` for core or `<package>@<version>` for independently versioned packages, and push the commit and tags atomically |

Preparation saves changed package names, versions, tags and metadata paths in `artifacts/release-plan.json`. The release-state archive carries this plan and every workspace manifest/changelog into build, packaging and metadata recording jobs. Beta suffixes apply only to changed packages; workspace dependencies resolve to the corresponding prepared versions. Unchanged package versions are not republished when only the plugin changes.

To preview version changes locally, run `pnpm version:packages`. It modifies the working tree without committing, pushing, or publishing. `.changeset/config.json` keeps `commit` set to `false`.

## Configure GitHub and npm

Configure these settings for every public package, including `@bloodhound-rs/rollup-plugin`:

| Location                                    | Configuration                                                                                                                   |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| GitHub repository Actions variables         | Set `NPM_RELEASE_USER` to the GitHub username authorized to publish. Without it, the workflow only builds                       |
| npm package Settings → Trusted publishing   | Configure owner `Justinidlerz`, repository `bloodhound`, workflow `release.yml`, no environment, and allow direct `npm publish` |
| GitHub Actions permissions and branch rules | Allow publishing jobs to request an OIDC token and the metadata job to write to `main` and create release tags                  |

Publishing uses Node.js 24 and checks for npm 11.15.0 or later. Authentication uses OIDC without injecting `NPM_TOKEN`. Preparation generates `repository.url` from the GitHub repository. See [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/).

Versions already staged before enabling direct publishing remain pending. A maintainer must approve those versions, or reject them before retrying direct publication of the same version. Changing publisher permissions does not approve pending packages. The workflow does not approve or reject staged versions. See [npm staged publishing](https://docs.npmjs.com/staged-publishing/).

## Retry and inspect results

The `npm-publish-results-<attempt>` artifact contains `publish-results.json`, including archive integrity and each attempted package's `published`, `already-published`, or `failed` status. Results are saved after each package, including before a failure exits the job. `bloodhound-packages` contains the archives, their release manifest, and the publishing script. `example-reports-*` contains the platform example reports.

After a partial failure, correct the cause and retry with the original archives. Public versions with identical integrity are skipped. Different content, duplicate staged versions, and authorization failures remain errors. Public package versions cannot be overwritten: use the original archives or advance to a fresh version before publishing rebuilt archives.

If publication succeeded but metadata recording failed, rerun only `record-release` after resolving branch permissions or an advancing branch. A full PR rerun produces a different beta version because its version includes the run ID and attempt number.

Rerunning an old Actions run uses that run's workflow and packaged script. Workflow edits take effect in runs created from a commit containing those edits.

## Recover original release archives

Run `Release` manually on `main` with `publish_run_id` set to the original completed main-branch Release push run and `publish_tag` set to its intended npm tag. Only `NPM_RELEASE_USER` can publish these artifacts. Recovery checks the source run, downloads `bloodhound-packages`, applies the current publishing script, and verifies archive integrity. Matching public versions are skipped and missing versions are published and verified. No binaries are rebuilt.

Recovery does not consume current Changesets or update version files from an older source revision. Reconcile the recovered release version before preparing a later release with new artifacts.
