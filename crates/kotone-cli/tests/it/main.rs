//! kotone-cli 的集成测试：一个测试程序，每个文件是一个子模块。
//!
//! `tests/` 下每个 `.rs` 都是单独的测试程序，都要把带 sherpa-onnx 的整套依赖链接一次，链接比跑测试慢得多。
//! 所以照 matklad 的 “Delete Cargo Integration Tests”（<https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html>，
//! “one integration test binary”）合成 `tests/it/main.rs` 加子模块。新的集成测试写成这里的子模块，不要在 `tests/` 下另起文件。
//!
//! 只跑一个文件的：`cargo test -p kotone-cli --test it tools_cli_m2::`。

mod agent_cli;
mod tools_cli;
mod tools_cli_m2;
