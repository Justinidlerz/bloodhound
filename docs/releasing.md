# Build and release

This guide describes versioning and publishing for maintainers. The workflow is `.github/workflows/release.yml`, and the main branch is `main`.

| Trigger                                                                                                                                  | Behavior                                                                                                                        |
| ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| PR opened, updated, reopened, or marked ready                                                                                            | Check source formatting, build five native targets, check types, run the existing JS / HTML examples, and assemble npm packages |
| Non-draft PR from the same repository, with both author and triggering actor matching `NPM_RELEASE_USER`, and a changeset version change | Publish beta after checks pass, without writing version files back to the PR branch                                             |
| Push to `main` by `NPM_RELEASE_USER`, with a changeset version change                                                                    | Publish latest after checks pass, commit version metadata, and push a version tag                                               |
| Manual run                                                                                                                               | Check, build, and pack without publishing                                                                                       |

Native targets are defined in `.github/native-targets.json`: Linux glibc x64 / arm64, macOS x64 / arm64, and Windows x64. Keep `package.json` → `napi.targets` aligned with this matrix. `pnpm platforms:sync` checks that they match and uses napi-rs to generate `npm/<platform>/package.json`.

Core contains the loader, declarations, schemas, and documentation. Each of the five platform packages contains its own binary and restricts installation with `os`, `cpu`, and `libc`. Core uses `workspace:*` optional dependencies in the repository. `pnpm pack` converts these to exact versions.

The assembly job runs `pnpm pack:release`. It checks that all five binaries exist, core contains no binaries, and each platform package contains exactly one binary. It produces six archives and a release manifest with SHA-512 integrity values. The publishing job publishes platform packages first, confirms their version integrity values in npm, and then publishes core without rebuilding. Retrying skips already published versions with matching content and stops if content differs.

## Use Changesets

For a change that requires a release, run this command from the repository root:

```sh
pnpm changeset
```

Select a patch, minor, or major release for `@bloodhound-rs/core` and write a description for SDK consumers. All six packages belong to one Changesets fixed group and receive the same version. Include the generated `.changeset/*.md` file in the PR. Documentation-only or example-only changes do not require a changeset. Without a version change, the workflow does not publish.

| Stage                  | Version handling                                                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| PR                     | Changesets calculates the next stable version, then appends `-beta.<PR-number>.<run-ID>.<attempt>` and uses the npm `beta` tag |
| Main branch            | Changesets updates versions and `CHANGELOG.md`, consumes changeset files, and uses the npm `latest` tag                        |
| Before native builds   | Synchronize the npm version to `native/Cargo.toml` and `Cargo.lock`. Reports use the compiled Rust version                     |
| After a stable release | Commit version files, changelogs, and changeset updates, then atomically push the commit and `v<version>` tag                  |

To preview version changes locally, run `pnpm version:packages`. It modifies the working tree without committing, pushing, or publishing. `.changeset/config.json` keeps `commit` set to `false`.

## Configure GitHub and npm

Configure these settings before publishing:

| Location                                    | Configuration                                                                                                              |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| GitHub repository Actions variables         | Set `NPM_RELEASE_USER` to the GitHub username authorized to publish. Without it, the workflow only builds                  |
| GitHub repository Actions secrets           | Set `NPM_TOKEN` with publishing access to the target scope for initial publication or packages without a Trusted Publisher |
| npm package Settings → Trusted publishing   | Enter the GitHub owner and repository, use `release.yml` as the workflow filename, and permit direct `npm publish`         |
| GitHub Actions permissions and branch rules | Permit the release metadata job to write to `main` and version tags. PR comment permissions are not required               |

Publishing uses Node.js 24 and checks for npm 11.5.1 or later. npm uses available OIDC trusted publishing authorization and supports `NPM_TOKEN` for initial publication. The token is injected only into the publishing step, not into build jobs. After initial publication, bind a Trusted Publisher in each package's npm settings. Remove `NPM_TOKEN` after confirming that OIDC publication succeeds. Preparation generates `repository.url` from the GitHub repository. See [npm Trusted Publishing](https://docs.npmjs.com/trusted-publishers/) for setup details.

If stable publication succeeds but metadata recording fails, resolve branch or tag permissions, or conflicts from an advancing branch, then resume the `record-release` job. The workflow does not force-push or delete npm versions. PR beta versions include the run ID and attempt number, so reruns do not reuse the same beta version.

`NPM_TOKEN` must cover core and all five platform packages. When using Trusted Publishers, bind each package to the same `release.yml` workflow.

Download build artifacts from the Actions run: `bloodhound-packages` contains six npm archives, the release manifest, and the publishing script. `example-reports-*` contains reports from each platform's examples. Check the job summary for published versions and artifact details.
