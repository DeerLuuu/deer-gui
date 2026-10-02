//! 平台无关的绘制数据：颜色、绘制命令、绘制列表。
//!
//! 后端只认这一份数据 —— 这样「渲染器写一次、后端换着接」才成立。

/// 线性空间不做转换的直通 RGBA 颜色（0–255 + alpha 0.0–1.0）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 1.0 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color { r, g, b, a }
    }
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0.0 };
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    /// 打包成 `0xRRGGBBAA`（便于断言与着色器 uniform）。
    pub const fn packed(self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | ((self.a * 255.0) as u32)
    }

    /// 向白色混合 `t`（`0.0` = 原色、`1.0` = 白）。**alpha 不变**。
    ///
    /// 用途：交互状态色（hover 提亮）。刻意**不动 alpha** —— 把「提亮」做成不透明叠加，
    /// 是为了让 CPU / GPU 的逐字节对照在不透明语料上仍然成立（半透明只保证 ≤1 LSB）。
    pub fn lighten(self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let mix = |c: u8| (c as f32 + (255.0 - c as f32) * t).round().clamp(0.0, 255.0) as u8;
        Color {
            r: mix(self.r),
            g: mix(self.g),
            b: mix(self.b),
            a: self.a,
        }
    }

    /// 向黑色混合 `t`（`0.0` = 原色、`1.0` = 黑）。**alpha 不变**。
    ///
    /// 用途：交互状态色（pressed 加深）。见 [`Color::lighten`] 关于 alpha 的说明。
    pub fn darken(self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let mix = |c: u8| (c as f32 * (1.0 - t)).round().clamp(0.0, 255.0) as u8;
        Color {
            r: mix(self.r),
            g: mix(self.g),
            b: mix(self.b),
            a: self.a,
        }
    }
}

/// 整数矩形（几何已在布局阶段取整 ⇒ 绘制也用整数，避免半像素模糊）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RectI {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> RectI {
        RectI { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }
}

/// 纹理句柄（字形图集、图片）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u32);

/// 绘制命令。
///
/// 设计原则：**命令是「结果」不是「控件」**。控件（Button/Field…）在
/// `Renderer` 里被翻译成矩形/文字/图标；后端只负责把矩形与字形画出来。
/// 这样新增控件类型**不需要动任何后端**。
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    /// 填充矩形。
    FillRect { rect: RectI, color: Color },
    /// 描边矩形（1px 或指定线宽）。
    StrokeRect { rect: RectI, color: Color, width: i32 },
    /// 圆角填充（半径实际由后端做圆角化；CPU 后端用简单掩码）。
    FillRoundRect { rect: RectI, radius: i32, color: Color },
    /// 一段文字（字形从图集取样）。`size` 为像素字号。
    Text {
        rect: RectI,
        text: String,
        color: Color,
        size: f32,
        /// 水平对齐（0=左，1=中，2=右）。
        align: u8,
    },
    /// 推送裁剪区（与 `PopClip` 配对）。
    PushClip { rect: RectI },
    PopClip,
    /// 引用节点（**绑定校验用**；后端可忽略）。
    ///
    /// 后端的语义是「诊断信息，安静忽略」—— 它**不产生任何像素**。但
    /// `deer_gui::interaction::ClipSnapshot::from_draw_list` 依赖它把「第 k 条提示」
    /// 绑到「前序里第 k 个有几何的节点」上，所以这两个字段是**校验和**，不是给人看的：
    ///
    /// - `node_id_len`：id 的**字节长度**（便宜的前置检查）；
    /// - `node_id_fp`：id 的**确定性指纹**（[`node_id_fp`]）。
    ///
    /// 为什么必须有指纹：只有长度时，**等长 id 互换**（例如同一层里两个 8 字节的 id
    /// 换了位置）会让长度校验和逐项相同 ⇒ 绑定静默错位、快照悄悄张冠李戴。
    /// 两个字段都由 [`DrawCmd::node_hint`] 一处产出，调用方不要手写字面量。
    NodeHint {
        rect: RectI,
        node_id_len: u32,
        node_id_fp: u64,
    },
}

/// 节点 id 的**确定性指纹**（FNV-1a 64 位）。
///
/// 用途见 [`DrawCmd::NodeHint`]：让 id **真的参与**「提示 ⇄ 节点」的绑定校验。
///
/// 三条刻意的性质：
/// 1. **确定性**：纯函数、无随机种子（`DefaultHasher` 不能用在这里 —— 它只承诺
///    「同一次运行内一致」，而这里的护栏要跨进程/跨平台复现同样的结论）；
/// 2. **无依赖、无分配**：`const fn`，热路径上每个节点每帧算一次也不产生堆分配；
/// 3. **够用就好**：这里要拦的是「顺序错位」这类**结构性**错误，不是对抗性碰撞，
///    所以不引密码学哈希（那会让 `DrawCmd` 与构建图都变重）。
pub const fn node_id_fp(id: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let bytes = id.as_bytes();
    let mut h = OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(PRIME);
        i += 1;
    }
    h
}

impl DrawCmd {
    /// 一条节点提示（**唯一**构造点：id 指纹只在这里算，免得各产出点各写一份）。
    pub fn node_hint(rect: RectI, node_id: &str) -> DrawCmd {
        DrawCmd::NodeHint {
            rect,
            node_id_len: node_id.len() as u32,
            node_id_fp: node_id_fp(node_id),
        }
    }
}

/// 一帧的绘制列表。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
    /// 裁剪栈深度必须归零（结构不变式）。
    clip_balance: i32,
}

impl DrawList {
    pub fn new() -> DrawList {
        DrawList::default()
    }

    /// 从已有命令重建（后端内部用）。
    ///
    /// **会重新计算裁剪平衡**，所以它不会让 `clip_balanced()` 失去意义 ——
    /// 传入的命令序列与 `push()` 逐个添加等价。
    pub fn from_cmds(cmds: Vec<DrawCmd>) -> DrawList {
        let mut list = DrawList::new();
        for c in cmds {
            list.push(c);
        }
        list
    }

    pub fn push(&mut self, cmd: DrawCmd) {
        match cmd {
            DrawCmd::PushClip { .. } => self.clip_balance += 1,
            DrawCmd::PopClip => self.clip_balance -= 1,
            _ => {}
        }
        self.cmds.push(cmd);
    }

    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// 裁剪栈必须平衡 —— 这是绘制列表的结构不变式（后端可据此省掉运行时检查）。
    pub fn clip_balanced(&self) -> bool {
        self.clip_balance == 0
    }

    /// 统计各命令数量（测试与性能诊断用）。
    pub fn counts(&self) -> DrawCounts {
        let mut c = DrawCounts::default();
        for cmd in &self.cmds {
            match cmd {
                DrawCmd::FillRect { .. } => c.fill_rect += 1,
                DrawCmd::StrokeRect { .. } => c.stroke_rect += 1,
                DrawCmd::FillRoundRect { .. } => c.fill_round_rect += 1,
                DrawCmd::Text { .. } => c.text += 1,
                DrawCmd::PushClip { .. } => c.push_clip += 1,
                DrawCmd::PopClip => c.pop_clip += 1,
                DrawCmd::NodeHint { .. } => c.node_hint += 1,
            }
        }
        c
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawCounts {
    pub fill_rect: usize,
    pub stroke_rect: usize,
    pub fill_round_rect: usize,
    pub text: usize,
    pub push_clip: usize,
    pub pop_clip: usize,
    pub node_hint: usize,
}
