//! # deer-layout
//!
//! **deer-gui 的语言无关核心**：节点树、布局代数、命中测试、场景文件解析。
//!
//! 这一层**完全不知道 GPU、窗口、事件循环的存在** —— 它是纯函数与纯数据。
//! 这样安排的理由：
//!
//! 1. **可测**：布局与节点树能在没有 GPU 驱动的 CI 里被完整断言；
//! 2. **可换后端**：DOM/GPU/软渲染后端都只消费同一棵树与同一张几何表；
//! 3. **可移植**：这段逻辑与语言无关（它从 deer-ui 的 TypeScript 验证原型搬来，
//!    V0 的 28 条断言在这里以 Rust 测试的形式重建）。
//!
//! ## 两条构筑路径，一棵树
//!
//! ```text
//!   builder::Builder（命令式，imgui 式手感）─┐
//!                                            ├─→ node::Node 树 ─→ layout::layout → 几何
//!   scene::parse_scene（.dui，.tscn 式）    ─┘                      └→ layout::hit_test
//! ```
//!
//! 两条路径产出**结构相等**的树（`Node::structurally_eq`），这是核心不变式。

#![forbid(unsafe_code)]
#![deny(clippy::all)]

pub mod builder;
pub mod layout;
pub mod node;
pub mod registry;
pub mod scene;

pub use builder::{Builder, L};
pub use layout::{ApproxMeasure, Geometry, Intrinsics, Measure, TextStyle, hit_test, layout, measure_tree, metrics};
pub use layout::{
    SCROLLBAR_INSET, SCROLLBAR_MIN_THUMB, SCROLLBAR_W, ScrollbarGeom, scrollbar_geom,
};
pub use node::{Align, IdGen, Kind, LayoutProps, Node, NodeProps, Rect, Size};
pub use registry::{PropSpec, PropType, SPECS};
pub use scene::{SceneError, encode_scene, parse_scene};

/// 本 crate 的语义版本（实验阶段）。
pub const CORE_VERSION: &str = "0.0.0";
