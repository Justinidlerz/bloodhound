# Changesets

需要发布的改动在仓库根目录运行 `pnpm changeset`，选择 `@bloodhound-rs/core` 的 patch、minor 或 major，并填写面向使用者的变更说明。生成的 Markdown 文件随代码进入 PR。

仅文档或示例改动可不添加 changeset。没有待发布版本变更时，工作流只构建、检查和打包。

PR beta 不回写版本文件。正式发布成功后，工作流提交版本文件、changelog 和已消费的 changeset，并推送版本标签。完整配置见 [发布流程](../docs/releasing.md)。
