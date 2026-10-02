# 功能指南：属性注册表（prop-registry）

> 跑 `cargo run -p deer-gui --example prop_registry` 看全部登记项 ·
> 代码在 `crates/deer-layout/src/registry.rs` ·
> 出处：App 地基任务书 2026-10-02 的 **E1**（编辑器地基第一项）

## 1. 这是什么 / 什么时候用它

**一张纯数据表**：按 `Kind` 枚举「这个节点有哪些属性是可以被编辑的」—— 属性名、取值类型、
取值域、默认值、以及它对哪些 `Kind` 有意义。

什么时候用它：

- **你要做 Inspector 面板**（编辑器里「选中节点 → 显示可改的属性」）—— 这就是数据源，
  不用自己维护一份「哪个控件有哪些属性」的清单；
- **你要做「改一个属性」的撤销粒度**（E3）—— 粒度定义在属性名上，而属性名的权威列表在这里；
- **你要扩 `.dui` 语法**（E2）—— 语法面必须与这张表对齐，否则又变成两份真值；
- **你要做拖拽写回**（E6）—— 「拖手柄改的是哪个属性」要能查到它的取值域（px 还是 pct）。

**什么时候不要用它**：运行时渲染 / 布局**不读这张表**——布局只读 `LayoutProps` 结构体本身。
注册表是**工具层**的东西，不参与每一帧。

## 2. 最小示例

```rust
use deer_gui::layout::node::Kind;
use deer_gui::layout::registry;

// 某节点能改什么（保持声明序，编辑器可照序显示）
for spec in registry::for_kind(Kind::Column) {
    println!("{} : {:?}  默认 {}  取值域 {}", spec.name, spec.ty, spec.default, spec.domain);
}

// 某属性是什么（编辑器「改一个属性」的入口）
let padding = registry::find("padding").expect("已登记");
assert_eq!(padding.ty, registry::PropType::F32);
```

实测输出（`--example prop_registry`）：

```text
Column  ( 9 项) width, height, grow, padding, gap, main_axis, cross_axis, scroll, disabled
Row     ( 8 项) width, height, grow, padding, gap, main_axis, cross_axis, disabled
Text    ( 6 项) width, height, grow, wrap, label, disabled
Button  ( 5 项) width, height, grow, label, disabled
Field   ( 5 项) width, height, grow, label, disabled
```

## 3. 完整 API

```rust
pub enum PropType { F32, Bool, Size, Align, Text }

pub struct PropSpec {
    pub name: &'static str,           // 与结构体字段名逐字一致
    pub ty: PropType,                 // 编辑器该用什么控件改它
    pub domain: &'static str,         // 取值域（给人看 + 供校验）
    pub default: &'static str,        // 默认值字面量（与 Default 实测一致）
    pub kinds: &'static [Kind],       // 对哪些 Kind 有意义（非空）
}

pub const SPECS: &[PropSpec];                       // 全部登记项
pub fn all() -> &'static [PropSpec];                // SPECS 的函数形式
pub fn for_kind(kind: Kind) -> impl Iterator<Item = &'static PropSpec>;
pub fn find(name: &str) -> Option<&'static PropSpec>;
```

`PropType` 只回答「编辑器用什么控件」，**不是** Rust 类型系统的镜像 ——
`Size` 是个好例子：它对应 `Option<Size>`（`px` 或 `pct`），编辑器该给的是一个
「数值 + 单位下拉」，而不是一个数。

## 4. 自检（怎么确认你真的用对了）

`--example prop_registry` 结尾有四条自检，跑一次就全过：

1. 表非空、属性名唯一；
2. 每条都有非空的 `kinds` / `domain`（否则编辑器永远不显示它 —— 等于没登记）；
3. **形状与语义的对照**：`Column` 有 `padding` 而 `Text` 没有、`Text` 有 `wrap` 而 `Column` 没有
   （不是「数量对得上」这种空话）；
4. 五种 `Kind` 各至少有一条可编辑属性。

**防漂移**（这是本模块最关键的一环）：`registry.rs` 的
`registry_covers_every_struct_field` 用**穷尽解构**把 `LayoutProps`(11 字段) 与
`NodeProps`(2 字段) 全部列出来、**不写 `..`** ⇒ 只要给结构体加一个字段，
**这个文件先编译失败**，逼人回来登记。

## 5. 常见坑

- **加字段忘了登记** —— 编译期就会挡住（见上）。但**改名**同时忘了改 `PropSpec::name`
  的字符串，编译期看不见：那要靠 `.dui` 语法测试兜（`scene.rs` 与本表共用同一批名字）。
- **`default` 写成「我以为的默认」** —— 有测试拿 `LayoutProps::default()` 的**实际值**对照，
  不是比对另一份手写常量。改默认值而忘了改登记，测试会红。
- **以为渲染会读它** —— 不会。布局只读结构体；注册表是工具层数据。往表里加「运行时也想用」
  的东西之前先想清楚：那多半属于结构体。
- **`find` 对未登记的名字返回 `None`** —— 别在上面 `unwrap_or_default` 编一个条目出来，
  那会把「拼错属性名」变成静默无效。

## 6. 相关

- 上游任务书：[`docs/superpowers/plans/2026-10-02-app-foundation.md`](../superpowers/plans/2026-10-02-app-foundation.md) E 线
- 数据结构：[`node-tree.md`](node-tree.md)（`Node` / `LayoutProps` / `NodeProps`）
- `.dui` 语法：[`scene-file.md`](scene-file.md)
- 布局语义：[`layout.md`](layout.md)

### 做不到什么

- **不做「改属性的入口」**：本表只描述「有哪些属性」，`set_property` 这类**写**操作属 E3（树编辑 API）；
- **不含回调 / 校验器**：表里没有函数指针，取值域的校验由消费者自己实现（表只给字符串描述）；
- **不覆盖 `Node` 的结构字段**：`id` / `children` / `kind` 不在表里 —— 前两者是结构操作（E3），
  后者不可改（改 `Kind` 等于换一个节点）；
- **不保证顺序就是编辑器该显示的顺序**：声明序是一个合理默认，但编辑器有权分组/排序；
- **不做国际化**：`domain` 是给开发者看的中文字符串，没有 i18n。

## 7. 检查清单（发布前过一遍）

- [x] `--example prop_registry` 真的跑过，`exit = 0`
- [x] 示例结尾有自检断言（四条），不是「跑成功就算」
- [x] 每条登记项的 `kinds` / `domain` / `default` 都非空
- [x] 穷尽解构的防漂移测试在（加字段会编译失败）
- [x] `default` 与 `LayoutProps::default()` 实测值一致的测试在
- [x] 本指南含「做不到什么」一节
- [x] 登记进 `FEATURES.md` 且指南链接 + 示例命令都对
