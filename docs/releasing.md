# 构建与发布

工作流为 `.github/workflows/release.yml`，主分支为 `main`。

| 触发方式                                                                        | 行为                                                                           |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| PR 创建、更新、重新打开或转为 ready                                             | 源码格式检查、五个平台原生构建、类型检查、运行已有 JS / HTML 示例、汇总 npm 包 |
| 同仓库非 draft PR，作者与触发者均为 `NPM_RELEASE_USER`，且有 changeset 版本变更 | 检查通过后发布 beta，不回写 PR 分支                                            |
| `main` 收到 `NPM_RELEASE_USER` 推送，且有 changeset 版本变更                    | 检查通过后发布 latest，提交版本元数据并推送版本标签                            |
| 手动运行                                                                        | 只检查、构建和打包，不发布                                                     |

原生目标集中配置于 `.github/native-targets.json`：Linux glibc x64 / arm64、macOS x64 / arm64、Windows x64。各平台在相同的版本元数据上编译，汇总任务检查二进制齐全后打包。发布直接使用该 tarball，不重新编译。

## Changesets 用法

需要发布的代码改动运行：

```sh
pnpm changeset
```

选择 `@bloodhound-rs/core` 的 patch、minor 或 major，并填写变更说明。生成的 `.changeset/*.md` 随代码进入 PR。仅文档或示例改动可不添加 changeset，没有版本变更时不发布。

| 阶段           | 版本处理                                                                                     |
| -------------- | -------------------------------------------------------------------------------------------- |
| PR             | Changesets 计算下一正式版本，再追加 `-beta.<PR号>.<运行ID>.<重试次数>`，使用 npm `beta` 标签 |
| 主分支         | Changesets 更新版本和 `CHANGELOG.md`、消费变更文件，使用 npm `latest` 标签                   |
| 原生构建前     | 将 npm 版本同步到 `native/Cargo.toml` 和 `Cargo.lock`；报告版本从 Rust 编译版本取得          |
| 正式发布成功后 | 提交版本、changelog 和 Changesets 文件变更，原子推送提交与 `v<版本>` 标签                    |

本地预览版本文件变更可运行 `pnpm version:packages`。该命令修改工作区，不提交、不推送、不发布。`.changeset/config.json` 的 `commit` 固定为 `false`。

## GitHub 与 npm 配置

| 位置                                 | 配置                                                                                         |
| ------------------------------------ | -------------------------------------------------------------------------------------------- |
| GitHub 仓库 Actions variables        | 设置 `NPM_RELEASE_USER` 为获准发布的 GitHub 用户名。未设置时仅构建，不发布                   |
| GitHub 仓库 Actions secrets          | 首次发布或尚未配置 Trusted Publisher 时，设置有目标 scope 发布权限的 `NPM_TOKEN`             |
| npm 包 Settings → Trusted publishing | GitHub owner、仓库名填写实际仓库，workflow filename 填 `release.yml`，允许直接 `npm publish` |
| GitHub Actions 权限及分支规则        | 允许发布后的元数据任务写入 `main` 和版本标签；不需要 PR 评论权限                             |

发布采用 Node.js 24，检查 npm 版本至少为 11.5.1。npm 优先使用可用的 OIDC 可信发布授权，并支持通过 `NPM_TOKEN` 完成首次发布。Token 仅注入发布步骤，不传入构建任务。首次发布完成后，在 npm 包设置中绑定 Trusted Publisher，确认 OIDC 发布通过后即可移除 `NPM_TOKEN`。包的 `repository.url` 在准备阶段根据当前 GitHub 仓库生成。配置步骤见 [npm Trusted Publishing 文档](https://docs.npmjs.com/trusted-publishers/)。

正式发布成功而元数据回写失败时，先处理分支或标签权限、分支前进冲突，再恢复 `record-release` 任务。工作流不强制推送、不删除 npm 版本。PR beta 版本号包含运行 ID 和重试次数，重复运行不会复用同一 beta 版本。

构建产物在 Actions 页面下载：`bloodhound-core-package` 是汇总后的 npm 包，`example-reports-*` 是各平台示例报告。发布版本与产物信息写入 job summary。
