//! 文档与示例的一致性检查（**让「每个功能都要教」成为可执行的规则**）。
//!
//! 背景：M1 交付后出现过「库能跑，但使用者不知道如何渲染任何东西」——
//! 渲染链中间缺了一段（`Renderer` 只有 trait、没有实现），而且没有面向使用者的文档。
//! 结论：**文档与示例必须被测试强制**，否则一定会漂。
//!
//! 本测试检查四件事：
//! 1. `FEATURES.md` 里每个指向指南的链接，目标文件**存在**；
//! 2. `FEATURES.md` 里每条 `--example <名字>`，对应源码**存在**；
//! 3. `FEATURES.md` 里每个 ✅ 行，**同时有指南链接和示例**（不允许「只登记不交付」）；
//! 4. 每份已完成的指南里提到的示例，源码**存在**。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 仓库根（= 本 crate 的上两级）。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/deer-gui 的上两级应是仓库根")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("读不到 {}：{e}", path.display()))
}

/// 从文本里抽出所有 `--example <name>`。
fn examples_in(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for part in text.split("--example ").skip(1) {
        let name: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

/// 从 markdown 里抽出所有 `](path)` 形式的链接目标（只取 .md）。
fn md_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in text.split("](").skip(1) {
        let target: String = part.chars().take_while(|c| *c != ')').collect();
        let target = target.split('#').next().unwrap_or("").to_string();
        if target.ends_with(".md") {
            out.push(target);
        }
    }
    out
}

fn example_path(root: &Path, name: &str) -> PathBuf {
    root.join("crates/deer-gui/examples").join(format!("{name}.rs"))
}

#[test]
fn features_manifest_guides_exist() {
    let root = repo_root();
    let manifest = read(&root.join("FEATURES.md"));
    let mut missing = Vec::new();
    for link in md_links(&manifest) {
        // 相对仓库根解析（FEATURES.md 在根目录）
        let target = root.join(&link);
        if !target.exists() {
            missing.push(link);
        }
    }
    assert!(
        missing.is_empty(),
        "FEATURES.md 里有指向不存在的指南的链接：{missing:#?}"
    );
}

#[test]
fn features_manifest_examples_exist() {
    let root = repo_root();
    let manifest = read(&root.join("FEATURES.md"));
    let names = examples_in(&manifest);
    assert!(
        names.len() >= 5,
        "清单里应当提到至少 5 个示例，实际 {} —— 若你删了示例，也要同步清单",
        names.len()
    );
    let mut missing = Vec::new();
    for name in &names {
        if !example_path(&root, name).exists() {
            missing.push(name.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "FEATURES.md 提到的示例没有源码文件：{missing:#?}（应在 crates/deer-gui/examples/<名字>.rs）"
    );
}

#[test]
fn every_completed_feature_has_guide_and_example() {
    let root = repo_root();
    let manifest = read(&root.join("FEATURES.md"));

    // 只看「功能表格」里的行：以 `|` 开头、含 ✅、且不是表头/分隔行
    let mut checked_rows = 0;
    let mut problems = Vec::new();

    for line in manifest.lines() {
        if !line.starts_with('|') || !line.contains('✅') {
            continue;
        }
        // 排除图例行（「完成，有示例 + 指南，可放心用」）
        if line.contains("完成，有示例") {
            continue;
        }
        checked_rows += 1;

        // 必须有一个 .md 链接（指南）
        if md_links(line).is_empty() {
            problems.push(format!("缺少指南链接：{}", first_cell(line)));
        }
        // 必须有示例命令
        if examples_in(line).is_empty() {
            problems.push(format!("缺少可运行示例：{}", first_cell(line)));
        }
    }

    assert!(
        checked_rows >= 8,
        "应当有至少 8 行已完成功能，实际 {checked_rows} —— 若这是真的，说明清单被删减了"
    );
    assert!(
        problems.is_empty(),
        "以下 ✅ 功能没有「指南 + 示例」齐备（规则见 FEATURES.md）：\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn guide_files_mention_existing_examples_and_are_complete() {
    let root = repo_root();
    let dir = root.join("docs/features");
    let mut guides = 0;
    let mut problems = Vec::new();

    let entries = std::fs::read_dir(&dir).expect("docs/features 目录应存在");
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name == "TEMPLATE.md" {
            continue;
        }
        let text = read(&path);
        guides += 1;

        // ① 指南提到的示例必须存在
        for ex in examples_in(&text) {
            if !example_path(&root, &ex).exists() {
                problems.push(format!("{name}: 提到示例 `{ex}`，但没有 examples/{ex}.rs"));
            }
        }
        // ② 指南必须有「做不到什么」这一节（每份指南都要求写边界）
        if !text.contains("做不到") {
            problems.push(format!("{name}: 缺少「做不到什么」（见 TEMPLATE.md 第 6 节）"));
        }
        // ③ 指南不能还留着未打勾的检查清单（说明没走完流程）
        if text.contains("- [ ]") {
            problems.push(format!("{name}: 检查清单里还有未勾选项（- [ ]）"));
        }
    }

    assert!(guides >= 6, "应当有至少 6 份指南，实际 {guides}");
    assert!(problems.is_empty(), "指南检查失败：\n  {}", problems.join("\n  "));
}

#[test]
fn tutorial_and_manifest_are_linked_from_readme() {
    let root = repo_root();
    let readme = read(&root.join("README.md"));
    assert!(
        readme.contains("docs/TUTORIAL.md"),
        "README 必须链接教程（否则新手找不到入口）"
    );
    assert!(
        readme.contains("FEATURES.md"),
        "README 必须链接功能清单"
    );
    assert!(
        readme.contains("docs/features/"),
        "README 必须链接逐功能指南目录"
    );
}

/// 取表格行的第一个单元格（去掉 `|` 与加粗标记），用于报错时指明是哪个功能。
fn first_cell(line: &str) -> String {
    line.trim_start_matches('|')
        .split('|')
        .next()
        .unwrap_or("")
        .replace("**", "")
        .trim()
        .to_string()
}
