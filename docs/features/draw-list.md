# 功能指南：绘制列表（draw-list）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example draw_list` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**布局与渲染之间的中间表示**：一串**与后端无关**的绘制命令
（画矩形 / 圆角 / 描边 / 文字 / 推拉裁剪区）。

什么时候用它：
- **可断言**：想检查「这个节点到底产生了什么绘制」，不必真渲染；
- **统计**：命令数就是性能预算的抓手（`counts()`）；
- **自己写后端**：CPU 与 Vulkan 后端都只是 `DrawList` 的消费者 —— 你也能写一个；
- **诊断**：画的颜色/位置不对时，看命令列表比看像素快。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let theme = Theme::default();
let geo = deer_gui::layout_tree(&tree, 320, 200, theme.clone());
let list = deer_gpu::build_draw_list(&tree, &geo, theme, &ApproxMeasure);

for cmd in &list.cmds {
    match cmd {
        DrawCmd::Text { rect, text, .. } => println!("文字 \"{text}\" @ ({},{})", rect.x, rect.y),
        DrawCmd::FillRoundRect { rect, color, .. } => println!("圆角块 @ ({},{}) {}×{}", rect.x, rect.y, rect.w, rect.h),
        _ => {}
    }
}
```

跑完整版：`cargo run -p deer-gui --example draw_list`

## 3. 完整 API

### 构造

| 函数 | 说明 |
|---|---|
| `deer_gpu::build_draw_list(&tree, &geo, theme, &measure)` | 用默认渲染器生成 |
| `deer_gpu::render::DefaultRenderer::new(theme, &measure).build(&tree, &geo)` | 同上（显式版） |
| `deer_gpu::render::NullRenderer::build(&tree, &geo)` | 只出 `NodeHint`（诊断/对照用） |

### `DrawCmd` 的全部变体

| 变体 | 字段 | 含义 |
|---|---|---|
| `FillRect` | `rect`, `color` | 实心矩形 |
| `StrokeRect` | `rect`, `color`, `width` | 矩形描边（`width` 像素宽） |
| `FillRoundRect` | `rect`, `radius`, `color` | 圆角实心（CPU 后端用四角圆心近似，无抗锯齿） |
| `Text` | `rect`, `text`, `color`, `size`, `align` | 一段文字。`align`：0=左 1=中 2=右 |
| `PushClip` / `PopClip` | `rect` | 裁剪区（必须配对） |
| `NodeHint` | `rect`, `node_id_len` | 节点占位提示（后端可忽略） |

`RectI` 的字段是 `i32`（几何在布局阶段已经取整）。

### 列表上的方法

| 方法 | 作用 |
|---|---|
| `.len()` / `.is_empty()` | 命令数 |
| `.counts()` | 按类型统计 → `DrawCounts` |
| `.clip_balanced()` | 裁剪栈是否平衡（**后端依赖它**，不平衡会返回错误而不是画出奇怪结果） |
| `.cmds` | `Vec<DrawCmd>`，可直接遍历 |

## 4. 自检

```rust
// ① 裁剪栈必须平衡
assert!(list.clip_balanced(), "PushClip/PopClip 未配对");

// ② 禁用按钮必须用 border 色，而不是强调色
let disabled = geo["button_2"];
assert!(list.cmds.iter().any(|c| matches!(c,
    DrawCmd::FillRoundRect { rect, color, .. }
        if rect.x == disabled.x as i32 && *color == theme.border
)), "禁用按钮用错了颜色");

// ③ 命令数在预算内（性能抓手）
let n = list.counts();
assert!(n.fill_round_rect + n.text < 100, "命令太多：{n:?}");
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| `clip_balanced()` 为 false | 自己构造 `DrawList` 时 `PushClip`/`PopClip` 没配对 | 用 `DrawList::from_cmds(cmds)` 重建（它会重新计算平衡） |
| 拿到了 `DrawCmd` 但不知道单位 | 几何是 `i32` 像素，颜色是 `Color`（0–255 + alpha 0.0–1.0） | — |
| 想改某个控件的外观 | 命令由 `DefaultRenderer` 生成，**当前写死** | 自己写渲染器（实现 `Renderer` trait 或直接遍历几何） |
| 命令数比预期多 | 每个可见节点至少一条命令；按钮是「圆角块 + 文字」两条 | 正常 |

## 6. 相关

- 渲染：[`rendering.md`](rendering.md)
- GPU HAL（后端契约）：[`gpu-hal.md`](gpu-hal.md)
- **做不到**：自定义着色（渲染器现在是固定的）；**批处理的其它高级形态**（间接绘制 / 多批次提交 / 通用纹理 —— **统一管线与跨帧复用缓冲已落地**，见 [`gpu-geometry.md`](gpu-geometry.md) 第 7 节；原「合段」函数已随统一管线删除）

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example draw_list` → `exit=0`
- [x] 示例有自检断言（裁剪平衡 + 禁用色正确）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
