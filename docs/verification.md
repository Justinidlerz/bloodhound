# 本机验证

环境：macOS arm64、Node.js 22.23.1、Rust 1.99.0-nightly。Oxc 组件锁定为 0.154.0，最低 Rust 版本为 1.97。

| 检查                                                     | 结果                                                                         |
| -------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `cargo check --locked`                                   | 通过                                                                         |
| Rust release 与 napi 原生构建                            | 通过                                                                         |
| `pnpm typecheck` 与 TypeScript 声明生成                  | 通过                                                                         |
| `pnpm pack`                                              | 生成 `bloodhound-core-0.1.0.tgz`                                             |
| 包外解包、按 `@bloodhound/core` 导入、原生扫描与索引调用 | 通过                                                                         |
| 包外 TypeScript 消费者类型检查                           | 通过                                                                         |
| napi-rs 自动生成入口、Rust 直接导出扫描和索引            | 包外 ESM 导入、JS/CSS 扫描、索引查询及恢复通过                               |
| Rust 原生取消监听                                        | 扫描前及扫描中取消通过，保留 onabort，完成后移除监听                         |
| 设计中的 sharedConfig 示例                               | completed，保留 write / read / write，返回冲突候选和入口路径                 |
| 断点保存后恢复                                           | completed，Rust 重新校验输入                                                 |
| HTML + base + Import Map + 内联样式 + 模块摘要           | completed，4 个资源、3 条引用，识别 sharedConfig 写入                        |
| iframe 独立 scope 与 JSONL 回读                          | 6 个资源、5 条引用、两个独立 sharedConfig 目标，JSONL 实体数量与标准报告一致 |
| 公开 css-tree 3.2.1 压缩产物                             | partial，202655 字节、360 条访问、1 条候选问题                               |

公开产物的 partial 包含可变捕获别名、switch / 异常控制流和未知对象操作等覆盖缺口。结果保存在 `outputs/csstree-rust-report.json` 与 `outputs/csstree-rust.jsonl`。该次运行不是精确率、召回率或性能基准。

TypeSafe 仅核对公开的 JavaScript、原生扩展及 HTML 分词技术事实，没有上传内部设计、仓库源码或扫描报告。实际编译、包加载和扫描结果由本机命令产生。

没有新增或改写单元测试。没有使用浏览器，没有提交、推送、发布 npm 或创建 PR。其他平台的二进制构建与完整标注集验收尚未执行。
