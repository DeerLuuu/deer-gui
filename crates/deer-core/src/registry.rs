//! **属性注册表**（E1）：按 [`Kind`] 枚举「可编辑属性」的**纯数据表**。
//!
//! 出处：App 地基任务书 2026-10-02（`docs/superpowers/plans/2026-10-02-app-foundation.md`）
//! E 线第一项，编辑器最小闭环「选 → 看 → 改 → 存」的**上游**。
//!
//! # 为什么是纯数据表，而不是 Rust 反射
//!
//! 本项目有一条立场：**节点树里不含回调**（树是纯数据，`Node: Clone + PartialEq`）。
//! 注册表沿用同一条立场 —— 它只是一张 `&'static [PropSpec]`，没有 trait object、
//! 没有 `Any`、没有过程宏。Inspector 面板、「改一个属性」的 undo 粒度、`.dui` 2.0 的
//! 语法面、拖拽写回的映射规则，**四个消费者共用这一张表**，而不是各自维护一份。
//!
//! # 防漂移：**编译期**，不是靠自觉
//!
//! 注册表最大的风险是「给结构体加了字段，忘了登记」——那种漏**不会报错**，
//! 只会让编辑器少显示一个属性，很难查。这里的对策是 [`tests::registry_covers_every_struct_field`]
//! 的**穷尽解构**：
//!
//! ```ignore
//! let LayoutProps { width, height, /* … 一个都不许少 … */ } = LayoutProps::default();
//! ```
//!
//! 模式里**不写 `..`** ⇒ 只要给 [`crate::node::LayoutProps`] 或 [`crate::node::NodeProps`] 加一个字段，
//! **这个文件先编译失败**，逼作者回来登记。这比「写个测试断言字段数量」强：
//! 数量相同但换了字段名的改动骗不过解构。
//!
//! **它抓不到什么**（如实登记）：把字段**改名**却忘了同步 [`PropSpec::name`] 的字符串
//! —— 那属于字符串层面的漂移，编译期看不见。这类由 `.dui` 的语法测试兜
//! （[`crate::scene`] 的属性名与结构体字段名共用同一份字符串来源）。

use crate::node::Kind;

/// 属性的**取值类型** —— 只回答「编辑器该用什么控件改它」，不是 Rust 类型的镜像。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    /// 浮点数（`padding` / `gap` / `grow`）。
    F32,
    /// 布尔开关（`scroll` / `wrap` / `disabled`）。
    Bool,
    /// 可选尺寸：`px` 或 `pct`（`width` / `height`）。
    Size,
    /// 对齐方式（`main_axis` / `cross_axis`）。
    Align,
    /// **流外定位**（L1 的 `position`；L4 起 `Pos` 含 `Anchors` 变体）：偏移或锚点。
    Pos,
    /// 文本（`label`）。
    Text,
    /// **不透明**：往返保真的载体 —— **编辑器不该直接编辑它**（D8 的 `extra`）。
    ///
    /// 单列一类而不是塞进 `Text`：它的语义不是「一段文字」，而是
    /// 「本库不认识的属性的原样保留」。把它伪装成可编辑文本会诱导编辑器去改它，
    /// 而改它等于替用户编造未来版本的语义。
    Opaque,
}

/// 一条属性登记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropSpec {
    /// 属性名 —— 与结构体字段名**逐字一致**（防漂移测试钉住这一点）。
    pub name: &'static str,
    /// 编辑器该用什么控件改它。
    pub ty: PropType,
    /// 取值域（给人看；也供编辑器做输入校验）。
    pub domain: &'static str,
    /// 默认值（写成字面量，便于与 [`crate::node::LayoutProps`]::default 对照）。
    pub default: &'static str,
    /// 这个属性对哪些 [`Kind`] 有意义。**非空**是硬要求（有测试钉）。
    pub kinds: &'static [Kind],
}

// —— 适用面常量：把「哪些 Kind 有意义」写成具名集合，避免每条登记各写一遍数组 ——

/// 任何节点都可以设（尺寸与生长权重对容器/叶子都成立）。
const ANY: [Kind; 8] = [
    Kind::Column,
    Kind::Row,
    Kind::Text,
    Kind::Button,
    Kind::Field,
    Kind::Segmented,
    Kind::ChipGroup,
    Kind::TabBar,
];
/// 只有容器排得下子节点 ⇒ 内边距 / 间距 / 主轴对齐。
/// 选择类三种组是容器（M6 5c）——它们的孩子就是选项，同样吃这套布局参数。
const CONTAINERS: [Kind; 5] = [
    Kind::Column,
    Kind::Row,
    Kind::Segmented,
    Kind::ChipGroup,
    Kind::TabBar,
];
/// 只有容器有「子节点整体位移」这回事。
const SCROLLABLE: [Kind; 1] = [Kind::Column];
/// 有文本内容的节点。
const TEXTUAL: [Kind; 3] = [Kind::Text, Kind::Button, Kind::Field];
/// 换行只对纯文本有意义（按钮/输入框的换行是另一件事，本期没做）。
const TEXT_ONLY: [Kind; 1] = [Kind::Text];

/// **全部**登记项（声明的顺序 = 编辑器里显示的顺序）。
pub const SPECS: &[PropSpec] = &[
    PropSpec {
        name: "width",
        ty: PropType::Size,
        domain: "px(>=0) 或 pct(0..=100)",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "height",
        ty: PropType::Size,
        domain: "px(>=0) 或 pct(0..=100)",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "min_w",
        ty: PropType::Size,
        domain: "px(>=0) 或 pct(0..=100)；下限，min > max ⇒ min 赢",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "max_w",
        ty: PropType::Size,
        domain: "px(>=0) 或 pct(0..=100)；上限，grow 分配结果也被封顶",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "min_h",
        ty: PropType::Size,
        domain: "px(>=0) 或 pct(0..=100)；下限，min > max ⇒ min 赢",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "max_h",
        ty: PropType::Size,
        domain: "px(>=0) 或 pct(0..=100)；上限，grow 分配结果也被封顶",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "grow",
        ty: PropType::F32,
        domain: ">= 0（0 = 不生长）",
        default: "0",
        kinds: &ANY,
    },
    PropSpec {
        name: "padding",
        ty: PropType::F32,
        domain: ">= 0",
        default: "0",
        kinds: &CONTAINERS,
    },
    PropSpec {
        name: "gap",
        ty: PropType::F32,
        domain: ">= 0",
        default: "0",
        kinds: &CONTAINERS,
    },
    PropSpec {
        name: "main_axis",
        ty: PropType::Align,
        domain: "start | center | end | stretch",
        default: "none",
        kinds: &CONTAINERS,
    },
    PropSpec {
        name: "cross_axis",
        ty: PropType::Align,
        domain: "start | center | end | stretch",
        default: "none",
        kinds: &CONTAINERS,
    },
    PropSpec {
        name: "cross_self",
        ty: PropType::Align,
        domain: "start | center | end | stretch（覆盖容器级 cross_axis，仅该流内子节点）",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "scroll",
        ty: PropType::Bool,
        domain: "true | false（只对 Column 有意义）",
        default: "false",
        kinds: &SCROLLABLE,
    },
    PropSpec {
        name: "wrap",
        ty: PropType::Bool,
        domain: "true | false（需 `width: px` 才有换行宽度）",
        default: "false",
        kinds: &TEXT_ONLY,
    },
    PropSpec {
        name: "position",
        ty: PropType::Pos,
        domain: "Offset：x,y（整数像素，可为负）｜Anchors：anchor-l/t/r/b=比例（可缺省）+ \
                 anchor-ox/oy=整数像素（正=向内）；任一形式 ⇒ 脱离流内",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "label",
        ty: PropType::Text,
        domain: "任意文本",
        default: "none",
        kinds: &TEXTUAL,
    },
    PropSpec {
        name: "extra",
        ty: PropType::Opaque,
        domain: "未知属性的原样保留（**不要编辑**；它只为往返保真）",
        default: "none",
        kinds: &ANY,
    },
    PropSpec {
        name: "disabled",
        ty: PropType::Bool,
        domain: "true | false（**整棵子树**都不响应输入）",
        default: "false",
        kinds: &ANY,
    },
];

/// 全部登记项（[`SPECS`] 的函数形式，便于 `.iter()` 链式使用）。
pub fn all() -> &'static [PropSpec] {
    SPECS
}

/// 某个 [`Kind`] 上有意义的属性（**保持 [`SPECS`] 的声明序**）。
pub fn for_kind(kind: Kind) -> impl Iterator<Item = &'static PropSpec> {
    SPECS.iter().filter(move |s| s.kinds.contains(&kind))
}

/// 按名字查一条登记（名字是结构体字段名，大小写敏感）。
pub fn find(name: &str) -> Option<&'static PropSpec> {
    SPECS.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{LayoutProps, NodeProps};

    /// **防漂移（编译期 + 测试期双保险）**
    ///
    /// ① `let` 模式**不写 `..`** ⇒ 结构体加字段时本函数**编译失败**（这是主要防线）；
    /// ② 把解构出的**每个绑定都碰一次**（`let _probe`）⇒ 不会退化成 unused 警告而被人顺手删掉；
    /// ③ 断言「登记表的属性名集合」与「结构体字段名集合」**完全相等**（改名也会红，
    ///    只要名字表同步改了；只改结构体不改名字表则由 ① 之外的情形兜 —— 见模块文档的「抓不到什么」）。
    #[test]
    fn registry_covers_every_struct_field() {
        // ① 穷尽解构（**不加 `..`**：少写一个字段就编译不过）
        let LayoutProps {
            width,
            height,
            padding,
            gap,
            main_axis,
            cross_axis,
            grow,
            scroll,
            wrap,
            position,
            cross_self,
            min_w,
            max_w,
            min_h,
            max_h,
        } = LayoutProps::default();
        let NodeProps {
            label,
            disabled,
            extra,
        } = NodeProps::default();

        // ② 触碰每个绑定（否则编译器会警告 unused，后来者容易一把删掉）
        let _probe = (
            &width, &height, &padding, &gap, &main_axis, &cross_axis, &grow, &scroll, &wrap,
            &position, &cross_self, &min_w, &max_w, &min_h, &max_h, &label, &disabled, &extra,
        );

        // ③ 名字集合必须相等（两边都排序后比较，避免顺序敏感）
        let mut from_struct = vec![
            "width",
            "height",
            "padding",
            "gap",
            "main_axis",
            "cross_axis",
            "grow",
            "scroll",
            "wrap",
            "position",
            "cross_self",
            "min_w",
            "max_w",
            "min_h",
            "max_h",
            "label",
            "disabled",
            "extra",
        ];
        let mut from_registry: Vec<&str> = SPECS.iter().map(|s| s.name).collect();
        from_struct.sort_unstable();
        from_registry.sort_unstable();
        assert_eq!(
            from_registry, from_struct,
            "属性注册表与 `LayoutProps` + `NodeProps` 的字段集必须完全一致\
             （加字段请同步登记；本断言失败说明两边漂了）"
        );
    }

    /// 默认值字面量必须与 `Default::default()` 的实际取值一致 ——
    /// 否则编辑器会「显示一个值、实际是另一个」，那是最难查的一类错。
    ///
    /// 判据来源是**结构体的 Default**，不是再抄一遍常量。
    #[test]
    fn registry_defaults_match_the_struct_default() {
        let l = LayoutProps::default();
        let n = NodeProps::default();
        let show = |s: &PropSpec| -> String {
            match s.name {
                "width" => format!("{:?}", l.width).to_lowercase(),
                "height" => format!("{:?}", l.height).to_lowercase(),
                "min_w" => format!("{:?}", l.min_w).to_lowercase(),
                "max_w" => format!("{:?}", l.max_w).to_lowercase(),
                "min_h" => format!("{:?}", l.min_h).to_lowercase(),
                "max_h" => format!("{:?}", l.max_h).to_lowercase(),
                "padding" => show_f32(l.padding),
                "gap" => show_f32(l.gap),
                "main_axis" => format!("{:?}", l.main_axis).to_lowercase(),
                "cross_axis" => format!("{:?}", l.cross_axis).to_lowercase(),
                "cross_self" => format!("{:?}", l.cross_self).to_lowercase(),
                "grow" => show_f32(l.grow),
                "scroll" => l.scroll.to_string(),
                "wrap" => l.wrap.to_string(),
                "position" => format!("{:?}", l.position).to_lowercase(),
                "label" => format!("{:?}", n.label).to_lowercase(),
                "disabled" => n.disabled.to_string(),
                "extra" => "none".to_string(), // 空表 ⇒ 登记里写 none
                other => panic!("登记表里有 `{other}`，但本测试不知道它该映到哪个字段 —— 请补上"),
            }
        };
        for s in SPECS {
            let actual = show(s);
            assert_eq!(
                s.default, actual,
                "`{}` 的登记默认值 `{}` 与结构体实际默认 `{}` 不一致",
                s.name, s.default, actual
            );
        }
    }

    fn show_f32(v: f32) -> String {
        // 0.0 与 0 视为同一个值（登记表写 "0" 更易读）
        if v == 0.0 {
            "0".to_string()
        } else {
            v.to_string()
        }
    }

    /// 每条登记的适用面必须非空，且登记的名字唯一。
    #[test]
    fn registry_entries_are_sane() {
        let mut seen = Vec::new();
        for s in SPECS {
            assert!(
                !s.kinds.is_empty(),
                "`{}` 的 kinds 为空 ⇒ 它在任何 Kind 上都不可达（编辑器永远不显示它）",
                s.name
            );
            assert!(
                !seen.contains(&s.name),
                "`{}` 登记了两次 —— 名字必须唯一",
                s.name
            );
            seen.push(s.name);
            assert!(
                !s.domain.is_empty() && !s.default.is_empty(),
                "`{}` 的 domain/default 不能为空（编辑器要用前者做校验、后者做「重置」）",
                s.name
            );
        }
    }

    /// 形状与语义的**具体**对照（不是「数量对得上」这种空话）：
    /// 容器专属属性不该出现在叶子上，反之亦然。
    #[test]
    fn for_kind_splits_container_and_leaf_props() {
        let names = |k: Kind| -> Vec<&'static str> { for_kind(k).map(|s| s.name).collect() };

        let col = names(Kind::Column);
        for expected in ["padding", "gap", "main_axis", "cross_axis", "scroll"] {
            assert!(col.contains(&expected), "Column 应有 `{expected}`，实际 {col:?}");
        }
        assert!(!col.contains(&"wrap"), "`wrap` 只对 Text 有意义：{col:?}");

        let row = names(Kind::Row);
        assert!(
            !row.contains(&"scroll"),
            "`scroll` 本期只做垂直滚动、只对 Column 有意义：{row:?}"
        );
        assert!(row.contains(&"padding"), "Row 是容器，应有 padding：{row:?}");

        let text = names(Kind::Text);
        assert!(text.contains(&"wrap"), "Text 应有 `wrap`：{text:?}");
        assert!(!text.contains(&"padding"), "`padding` 只对容器有意义：{text:?}");
        assert!(text.contains(&"label"), "Text 有 label：{text:?}");

        // 叶子共同点：都能设尺寸/grow/禁用/流外定位（`disabled` 对整棵子树生效 ⇒ 叶子也适用）
        // L2/L3：`cross_self`（叶子也能是别人家的子节点）与 min/max（与 width/height 同面）也是 ANY
        for leaf in [Kind::Text, Kind::Button, Kind::Field] {
            let ns = names(leaf);
            for expected in [
                "width", "height", "min_w", "max_w", "min_h", "max_h", "grow", "cross_self",
                "disabled", "position", "label",
            ] {
                assert!(ns.contains(&expected), "{leaf:?} 应有 `{expected}`：{ns:?}");
            }
        }
    }

    /// `find` 的行为（含未登记名字必须返回 `None`，不能瞎猜一个默认项）。
    #[test]
    fn find_is_exact_and_rejects_unknown_names() {
        assert_eq!(find("padding").map(|s| s.ty), Some(PropType::F32));
        assert_eq!(find("wrap").map(|s| s.ty), Some(PropType::Bool));
        assert!(find("Padding").is_none(), "名字大小写敏感");
        assert!(find("").is_none());
        assert!(find("no_such_prop").is_none());
    }
}
