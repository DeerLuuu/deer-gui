//! 功能示例：**命令行式界面（`.dui` 场景文件）**。
//!
//! ```sh
//! cargo run -p deer-gui --example scene_file
//! ```
//!
//! 与命令式 API 产出**结构相等**的树 —— 用哪套只看哪个更顺手：
//! 动态生成的界面用命令式，需要人手改的界面用场景文件。

use deer_gui::prelude::*;
use std::path::Path;

const SCENE: &str = r#"# 一个设置面板
[column name=app pad=14 gap=8]
  [text label=设置]
  [column name=card pad=10 gap=6]
    [text label=外观]
    [row name=themeRow gap=6]
      [button label=浅色]
      [button label=深色]
    [text label=通知]
    [row name=notifyRow gap=6]
      [button label=开]
      [button label=关 disabled]
  [row name=actions gap=8 main=end]
    [button label=取消]
    [button label=保存]
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ① 把场景文件写出来 —— 你之后可以直接改这个文件再重跑
    std::fs::create_dir_all("render_out")?;
    let path = Path::new("render_out/settings.dui");
    std::fs::write(path, SCENE)?;
    println!("写出场景文件：{}（可以直接编辑它再重跑）", path.display());

    // ② 解析成节点树。第二个参数是「文件名」，只用于报错时显示 `文件名:行号`
    let tree = parse_scene(SCENE, "settings.dui")?;
    let count = count_nodes(&tree);
    println!("解析成功：{count} 个节点");

    // ③ 编码回文本，验证往返稳定（parse ∘ encode ≡ 恒等）
    let roundtrip = encode_scene(&tree);
    let again = parse_scene(&roundtrip, "roundtrip.dui")?;
    assert!(tree.structurally_eq(&again), "场景文件往返必须结构相等");
    println!("往返一致 ✅");

    // ④ 渲染成图片
    let png = deer_gui::render_tree_to_png(&tree, 300, 260, Theme::default())?;
    let out = Path::new("render_out/scene_file.png");
    std::fs::write(out, &png)?;
    println!("渲染出图：{}（{} 字节）", out.display(), png.len());

    // ⑤ 故意写错，看报错是否可用（带行号）
    println!("\n故意制造一个错误看报错信息：");
    match parse_scene("[column name=a]\n  [button label=确定 nope=1]\n", "bad.dui") {
        Ok(_) => println!("（不该成功）"),
        Err(e) => println!("  {e}"),
    }
    Ok(())
}

fn count_nodes(n: &Node) -> usize {
    let mut c = 0;
    n.walk(&mut |_, _| c += 1, 0);
    c
}
