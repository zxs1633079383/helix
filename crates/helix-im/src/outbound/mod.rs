//! 出站命令编排（HTTP outbound build）。
//!
//! 现网前端 service 层的 wire body 组装编排下沉到此（接管：前端退化为纯渲染 + 语义 invoke）。
//! P1 落地 `send_build`（posts/create body）；后续 Phase（P2 registry 化 + posts 写族等）
//! 会在此目录补各命令注册文件。`mod.rs` 只 re-export。

pub mod send_build;
