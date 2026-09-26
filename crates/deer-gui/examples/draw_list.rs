//! 功能示例：**拿到绘制命令列表**（绘制列表 / DrawList）。
//!
//! ```sh
//! cargo run -p deer-gui --example draw_list
//! ```
//!
//! 绘制列表是「布局」与「渲染」之间的中间表示：
//! 它是一串**与后端无关**的命令（画矩形 / 描边 / 圆角 / 文字 / 推拉裁剪区）。
//!
//! 为什么自己在后端之外还要能拿到它：
//! - **可断言**：你可以检查「这个节点到底产生了什么绘制」，不必渲染
//! - **可实现自己的后端**：CPU 与 Vulkan 后端都只是 `DrawList` 的消费者
//! - **可统计**：命令数就是性能预算的抓手

use deer_gui::prelude::*;

fn main() {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("绘制列表演示");
    app.container_opts(Kind::Column, "card", L::new().pad(10.0).gap(6.0).to_props(), |c| {
        c.field("输入框");
        c.container_opts(Kind::Row, "row", L::new().gap(8.0).to_props(), |r| {
            r.button("确定");
            r.button_opts("禁用", |n| n.props.disabled = true);
        });
    });
    let tree = app.build();

    let theme = Theme::default();
    let (w, h) = (320u32, 200u32);
    let geo = deer_gui::layout_tree(&tree, w, h, theme.clone());

    // 树 + 几何 + 主题 → 绘制列表
    let list = deer_gpu::build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);

    println!("绘制命令共 {} 条\n", list.len());
    for (i, cmd) in list.cmds.iter().enumerate() {
        let desc = match cmd {
            DrawCmd::FillRect { rect, color } => {
                format!("填充矩形   ({},{}) {}×{}  颜色 #{:02x}{:02x}{:02x}", rect.x, rect.y, rect.w, rect.h, color.r, color.g, color.b)
            }
            DrawCmd::FillRoundRect { rect, radius, color } => {
                format!("圆角填充   ({},{}) {}×{}  半径{radius}  颜色 #{:02x}{:02x}{:02x}", rect.x, rect.y, rect.w, rect.h, color.r, color.g, color.b)
            }
            DrawCmd::StrokeRect { rect, width, .. } => {
                format!("描边矩形   ({},{}) {}×{}  线宽{width}", rect.x, rect.y, rect.w, rect.h)
            }
            DrawCmd::Text { rect, text, .. } => {
                format!("文字       ({},{}) {}×{}  \"{text}\"", rect.x, rect.y, rect.w, rect.h)
            }
            DrawCmd::PushClip { rect } => format!("推入裁剪区 ({},{}) {}×{}", rect.x, rect.y, rect.w, rect.h),
            DrawCmd::PopClip => "弹出裁剪区".to_string(),
            DrawCmd::NodeHint { rect, .. } => format!("节点提示   ({},{}) {}×{}", rect.x, rect.y, rect.w, rect.h),
        };
        println!("  [{i:>2}] {desc}");
    }

    // 按类型统计 —— 这就是性能预算的抓手
    let c = list.counts();
    println!("\n按类型统计：");
    println!("  圆角填充 {} / 矩形填充 {} / 描边 {}", c.fill_round_rect, c.fill_rect, c.stroke_rect);
    println!("  文字 {} / 推裁剪 {} / 弹裁剪 {}", c.text, c.push_clip, c.pop_clip);

    // 不变式：裁剪栈必须平衡（后端的 scissor 状态靠它）
    assert!(list.clip_balanced(), "裁剪栈必须平衡，否则后端无法恢复状态");
    println!("\n裁剪栈平衡 ✅");

    // 不变式：禁用按钮必须用 border 色，而不是强调色
    let disabled = geo["button_2"];
    let uses_border = list.cmds.iter().any(|cmd| match cmd {
        DrawCmd::FillRoundRect { rect, color, .. } => {
            rect.x == disabled.x as i32 && *color == theme.border
        }
        _ => false,
    });
    assert!(uses_border, "禁用按钮必须用 border 色填充（不能用强调色冒充）");
    println!("禁用按钮用对了颜色 ✅");
}
