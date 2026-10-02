//! **属性注册表**（E1）：按 `Kind` 列出「可编辑属性」—— 编辑器 Inspector / undo 粒度 /
//! `.dui` 2.0 语法面 / 拖拽写回**四个消费者的共同上游**。
//!
//! 跑法：
//! ```text
//! cargo run -p deer-gui --example prop_registry
//! ```
//! 产物：**标准输出**（本示例不写文件）。结尾有自检断言，防止「跑成功但什么都没做」。
//!
//! 指南：[`docs/features/prop-registry.md`](../../../docs/features/prop-registry.md)

use deer_gui::layout::node::Kind;
use deer_gui::layout::registry;

fn main() {
    println!("=== 属性注册表（E1）===\n");

    // ① 全部登记项 —— 这就是编辑器「能改什么」的完整清单
    println!("① 全部 {} 条登记：\n", registry::all().len());
    println!("  名字       类型   取值域                              默认");
    println!("  {}", "-".repeat(78));
    for s in registry::all() {
        println!(
            "  {:<10} {:<6} {:<34} {}",
            s.name,
            type_name(s.ty),
            s.domain,
            s.default
        );
    }

    // ② 按 Kind 过滤 —— 五种节点各自能改什么
    println!("\n② 按节点类型过滤（编辑器选中的节点决定 Inspector 显示什么）：\n");
    let kinds = [
        Kind::Column,
        Kind::Row,
        Kind::Text,
        Kind::Button,
        Kind::Field,
    ];
    for k in kinds {
        let names: Vec<&str> = registry::for_kind(k).map(|s| s.name).collect();
        println!("  {:<7} ({:>2} 项) {}", kind_name(k), names.len(), names.join(", "));
    }

    // ③ 按名字查一条（编辑器「改一个属性」的入口）
    println!("\n③ 按名字查：");
    for name in ["padding", "wrap", "disabled"] {
        match registry::find(name) {
            Some(s) => println!("  {name:<9} → 类型 {:?}，默认 {}，适用 {:?}", s.ty, s.default, s.kinds),
            None => println!("  {name:<9} → 未登记"),
        }
    }
    println!(
        "  {:<9} → {}（未登记的名字必须返回 None，不能瞎猜一个默认项）",
        "no_such",
        if registry::find("no_such").is_none() { "未登记 ✅" } else { "❌ 竟然查到了" }
    );

    // ─────────────────────────────────────────────────────────────────────
    // 自检：跑成功不等于做对了 —— 下面每条都要真的成立
    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 自检 ===");

    // ① 表非空，且名字唯一
    let mut names: Vec<&str> = registry::all().iter().map(|s| s.name).collect();
    assert!(!names.is_empty(), "注册表不能为空");
    let before = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(before, names.len(), "属性名必须唯一");
    println!("  ① {} 条登记、名字唯一 ✅", before);

    // ② 每条都要有非空的适用面（否则编辑器永远不显示它 —— 等于没登记）
    for s in registry::all() {
        assert!(!s.kinds.is_empty(), "`{}` 的 kinds 为空", s.name);
        assert!(!s.domain.is_empty(), "`{}` 的 domain 为空", s.name);
    }
    println!("  ② 每条都有非空 shapes/取值域 ✅");

    // ③ 形状与语义的对照：容器专属属性不该出现在叶子上（不是「数量对得上」这种空话）
    let col: Vec<&str> = registry::for_kind(Kind::Column).map(|s| s.name).collect();
    let text: Vec<&str> = registry::for_kind(Kind::Text).map(|s| s.name).collect();
    assert!(col.contains(&"padding"), "Column 应有 padding");
    assert!(!text.contains(&"padding"), "Text 是叶子，不该有 padding");
    assert!(text.contains(&"wrap"), "Text 应有 wrap");
    assert!(!col.contains(&"wrap"), "Column 不该有 wrap");
    println!("  ③ 容器/叶子的属性面确实分开 ✅");

    // ④ 五种 Kind 都至少有一条可编辑属性（否则那种节点在编辑器里是死的）
    for k in kinds {
        let n = registry::for_kind(k).count();
        assert!(n > 0, "{k:?} 一条可编辑属性都没有");
    }
    println!("  ④ 五种 Kind 各有属性可改 ✅");

    println!("\n全部自检通过。这些数据就是 Inspector 面板要显示的东西。");
}

/// 给人看的类型名。
fn type_name(t: registry::PropType) -> &'static str {
    match t {
        registry::PropType::F32 => "f32",
        registry::PropType::Bool => "bool",
        registry::PropType::Size => "size",
        registry::PropType::Align => "align",
        registry::PropType::Text => "text",
        registry::PropType::Opaque => "opaque",
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Column => "Column",
        Kind::Row => "Row",
        Kind::Text => "Text",
        Kind::Button => "Button",
        Kind::Field => "Field",
    }
}
