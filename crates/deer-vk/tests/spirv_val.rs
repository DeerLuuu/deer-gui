//! **用官方 `spirv-val` 校验自研 SPIR-V 汇编器的全部产物**。
//!
//! ## 为什么这条测试是必要的（一次代价很大的教训）
//!
//! 本项目自研 SPIR-V 汇编器（不依赖 SDK 的 `glslc`）。M2a 期间遇到
//! 「`vkCmdDraw` 不产生任何像素」的缺陷，而**所有 Vulkan API 都返回成功**：
//!
//! | 环节 | 现象 |
//! |---|---|
//! | `vkCreateShaderModule` | ✅ 接受（它只存字节，**不编译**） |
//! | `vkCreateGraphicsPipelines` | ✅ 返回成功、句柄非空 |
//! | `vkCmdDraw` | ❌ **静默不产生任何片元** |
//!
//! 靠推理排查了很久（换着色器、换几何、换 viewport、换顶点来源都不行），
//! 最后装上 Vulkan SDK、用官方 `spirv-val` 一跑就看到：
//!
//! ```text
//!   error: EntryPoint is in an invalid layout section
//! ```
//!
//! **根因是 SPIR-V 的段序错误**（`OpEntryPoint` 排在类型之后；
//! `OpFunction` 掉进了类型段）。驱动的「不报错」让这个缺陷极难定位。
//!
//! 所以：**每次改汇编器都必须跑这条测试**。它是唯一能独立判断产物是否合法的关卡。
//!
//! ## 找不到 `spirv-val` 时的行为
//!
//! 打印清晰的跳过原因（并说明怎么装），**不伪装成通过**。
//! 路径可用环境变量 `SPIRV_VAL` 覆盖。

use deer_vk::spirv;
use std::path::{Path, PathBuf};
use std::process::Command;

/// M3a 新增的两支着色器（名字, 产物）。
///
/// 单列成函数是为了让 `all_shaders()`（喂给官方 `spirv-val`）与
/// `rect_attrs_shaders_validate`（纯字节自检 + 单独校验）用**同一份**定义，
/// 不会出现「测的是一个、跑的是另一个」。
fn rect_attrs_shaders() -> [(&'static str, Vec<u8>); 2] {
    [
        ("vs_rect_attrs", spirv::vertex_shader_rect_attrs()),
        ("fs_rect_shape", spirv::fragment_shader_rect_shape()),
    ]
}

/// 找一个可用的 `spirv-val`。
fn find_spirv_val() -> Option<PathBuf> {
    // ① 环境变量优先
    if let Ok(p) = std::env::var("SPIRV_VAL") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    // ② 常见的 Vulkan SDK 安装位置
    let mut cands: Vec<PathBuf> = Vec::new();
    for root in ["C:\\VulkanSDK", "Z:\\VulkanSDK", "/usr/bin", "/usr/local/bin"] {
        let r = Path::new(root);
        if !r.is_dir() {
            continue;
        }
        // SDK 的目录结构是 <root>/<version>/Bin/spirv-val.exe
        if let Ok(entries) = std::fs::read_dir(r) {
            for e in entries.flatten() {
                cands.push(e.path().join("Bin").join("spirv-val.exe"));
                cands.push(e.path().join("bin").join("spirv-val"));
            }
        }
        cands.push(r.join("spirv-val.exe"));
        cands.push(r.join("spirv-val"));
    }
    // ③ PATH 里
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            cands.push(dir.join("spirv-val.exe"));
            cands.push(dir.join("spirv-val"));
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

/// 待校验的全部着色器。
fn all_shaders() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("vs_empty", spirv::vertex_shader_empty()),
        ("fs_empty", spirv::fragment_shader_empty()),
        ("vs_const_position", spirv::vertex_shader_const_position()),
        (
            "vs_reads_vertex_index",
            spirv::vertex_shader_reads_vertex_index(),
        ),
        (
            "vs_triangle",
            spirv::vertex_shader_triangle([[-0.8, -0.8], [0.8, -0.8], [-0.8, 0.8]]),
        ),
        (
            "vs_hardcoded_position",
            spirv::vertex_shader_hardcoded_position(),
        ),
        (
            "vs_select_full_vec4",
            spirv::vertex_shader_select_full_vec4([
                [-0.8, -0.8, 0.0, 1.0],
                [0.8, -0.8, 0.0, 1.0],
                [-0.8, 0.8, 0.0, 1.0],
            ]),
        ),
        (
            "vs_from_vertex_buffer",
            spirv::vertex_shader_from_vertex_buffer(),
        ),
        (
            "vs_rect_pushconstant",
            spirv::vertex_shader_rect_pushconstant(),
        ),
        (
            "fs_solid",
            spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]),
        ),
    ]
    .into_iter()
    .chain(rect_attrs_shaders())
    .collect()
}

/// M3a（矩形属性着色器）的专项校验。
///
/// 分两层：
/// 1. **纯字节自检**（任何机器都能跑）：魔数、头部 `bound > 0`、指令流用满整份模块
///    —— `bound` 合法是那个「驱动不报错也不画」缺陷的第一道防线
///    （见 `spirv.rs` 模块文档：头部 `bound` 必须 **>** 所有用到的 Id）
/// 2. **官方 `spirv-val`**：把这两支着色器交给与 `all_shaders()` 同一条流程校验
///    （`vkCreateShaderModule` 极宽容，只有独立校验器能判合法性）
#[test]
fn rect_attrs_shaders_validate() {
    for (name, code) in rect_attrs_shaders() {
        // ① 纯字节自检（不依赖 SDK）
        assert_eq!(code.len() % 4, 0, "{name}: SPIR-V 必须 4 字节对齐");
        let words: Vec<u32> = code
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(words[0], 0x0723_0203, "{name}: magic");
        // 头部第 4 个字是 bound，必须 > 0 且大于所有用到的 Id（spirv.rs 模块文档 L23-25）
        assert!(words[3] > 0, "{name}: 头部 bound 必须 > 0");

        // 指令流必须正好用满整份模块（词数自洽）
        let mut i = 5usize;
        while i < words.len() {
            let wc = (words[i] >> 16) as usize;
            assert!(wc >= 1, "{name}: 词 {i} 声明词数 0");
            assert!(i + wc <= words.len(), "{name}: 词 {i} 越界（wc={wc}）");
            i += wc;
        }
        assert_eq!(i, words.len(), "{name}: 指令流必须正好用完整份模块");
    }

    // ①b 结构专项：用了 GLSL.std.450 扩展指令集的着色器，其 `OpExtInstImport`
    //     必须落在「扩展指令集导入段」（规范序：Capability → Extension →
    //     ExtInstImport → MemoryModel → …）。
    //
    // 为什么单独查这一条：`OpFloor` **不是** core opcode —— 它来自
    // `GLSL.std.450` 扩展指令集（编号 8），必须 `OpExtInstImport` 一个
    // `GLSL.std.450` 串再 `OpExtInst`。这段是 M3a 新增的段序风险点，
    // 而段序错的表现恰恰是「驱动/校验器报别的错」（见本文件开头的教训）。
    {
        let bytes = spirv::fragment_shader_rect_shape();
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        const OP_EXT_INST_IMPORT: u16 = 11;
        const OP_EXT_INST: u16 = 12;
        const OP_MEMORY_MODEL: u16 = 14;
        let mut import_at = None;
        let mut memory_model_at = None;
        let mut ext_inst_count = 0usize;
        let mut i = 5usize;
        while i < words.len() {
            let wc = (words[i] >> 16) as usize;
            let op = (words[i] & 0xffff) as u16;
            match op {
                OP_EXT_INST_IMPORT => import_at = import_at.or(Some(i)),
                OP_MEMORY_MODEL => memory_model_at = memory_model_at.or(Some(i)),
                OP_EXT_INST => ext_inst_count += 1,
                _ => {}
            }
            i += wc;
        }
        let imp = import_at.expect("fs_rect_shape: 用了 GLSL.std.450 就必须有 OpExtInstImport");
        let mm = memory_model_at.expect("fs_rect_shape: 缺少 OpMemoryModel");
        assert!(
            imp < mm,
            "fs_rect_shape: OpExtInstImport（词 {imp}）必须在 OpMemoryModel（词 {mm}）之前 —— 段序错误"
        );
        assert!(
            ext_inst_count >= 2,
            "fs_rect_shape: 至少要两条 OpExtInst（px / py 各一次 OpFloor），实得 {ext_inst_count}"
        );
    }

    // ② 官方 spirv-val（与 all_shaders 同一条流程、同一个 find_spirv_val）
    let Some(val) = find_spirv_val() else {
        println!(
            "跳过官方 spirv-val 部分：本机没有 spirv-val。\n\
             装 Vulkan SDK（含 Shader Toolchain）即可用，或设环境变量 SPIRV_VAL 指向它。\n\
             ⚠️ 纯字节自检**不能**替代 spirv-val —— 它只查词数与 bound，不查语义合法性。"
        );
        return;
    };

    let dir = std::env::temp_dir().join("deer_spirv_val_rect_attrs");
    std::fs::create_dir_all(&dir).expect("建临时目录");
    for (name, bytes) in rect_attrs_shaders() {
        let p = dir.join(format!("{name}.spv"));
        std::fs::write(&p, &bytes).unwrap_or_else(|e| panic!("写 {} 失败：{e}", p.display()));
        let out = Command::new(&val)
            .arg(&p)
            .output()
            .unwrap_or_else(|e| panic!("执行 spirv-val 失败：{e}"));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "{name} 未通过官方 spirv-val（{}）：\n{stderr}",
            val.display()
        );
        println!("  ✅ {name}（{} 字节）", bytes.len());
    }
    println!("M3a 矩形属性着色器（顶点 + 片元）通过官方 spirv-val 校验 ✅");
}

#[test]
fn all_shaders_pass_official_spirv_val() {
    let Some(val) = find_spirv_val() else {
        println!(
            "跳过：本机没有 spirv-val。\n\
             装 Vulkan SDK（含 Shader Toolchain）即可用，或设环境变量 SPIRV_VAL 指向它。\n\
             ⚠️ 这条测试是自研 SPIR-V 汇编器**唯一独立**的合法性判据 —— \
             长期跳过等于没有验证。"
        );
        return;
    };
    println!("用 {} 校验", val.display());

    let dir = std::env::temp_dir().join("deer_spirv_val");
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let shaders = all_shaders();
    assert!(shaders.len() >= 8, "待校验的着色器数量异常");

    let mut failures = Vec::new();
    for (name, bytes) in &shaders {
        let p = dir.join(format!("{name}.spv"));
        std::fs::write(&p, bytes).unwrap_or_else(|e| panic!("写 {} 失败：{e}", p.display()));

        let out = Command::new(&val)
            .arg(&p)
            .output()
            .unwrap_or_else(|e| panic!("执行 spirv-val 失败：{e}"));

        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.success() {
            println!("  ✅ {name}（{} 字节）", bytes.len());
        } else {
            println!("  ❌ {name}");
            for line in stderr.lines().take(6) {
                println!("       {line}");
            }
            failures.push((name.to_string(), stderr.to_string()));
        }
    }

    assert!(
        failures.is_empty(),
        "{} 个着色器未通过官方 spirv-val 校验：\n{}",
        failures.len(),
        failures
            .iter()
            .map(|(n, e)| format!("--- {n}\n{e}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!("全部 {} 支着色器通过官方 spirv-val 校验 ✅", shaders.len());
}

/// 段序专项回归：`OpEntryPoint` 必须出现在类型/常量**之前**。
///
/// 这是那个「驱动不报错也不画」缺陷的直接判据 —— 即使没有 `spirv-val`
/// 也能跑（纯字节检查），所以它在任何机器上都能保护这段逻辑。
#[test]
fn entry_point_precedes_types_in_wire_format() {
    const OP_ENTRY_POINT: u16 = 15;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_FLOAT: u16 = 22;
    const OP_FUNCTION: u16 = 54;
    const OP_VARIABLE: u16 = 59;

    for (name, bytes) in all_shaders() {
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(words[0], 0x0723_0203, "{name}: magic");
        assert_eq!(words[1], spirv::SPIRV_VERSION_1_0, "{name}: version");

        // 扫一遍，记住各关键指令第一次出现的**词位置**
        let mut first: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        let mut i = 5usize;
        while i < words.len() {
            let wc = (words[i] >> 16) as usize;
            let op = (words[i] & 0xffff) as u16;
            if wc == 0 || i + wc > words.len() {
                panic!("{name}: 词流在 {i} 处损坏（wc={wc}）");
            }
            first.entry(op).or_insert(i);
            i += wc;
        }

        let ep = *first
            .get(&OP_ENTRY_POINT)
            .unwrap_or_else(|| panic!("{name}: 缺少 OpEntryPoint"));

        // OpEntryPoint 必须在**所有**类型声明之前
        for op in [OP_TYPE_VOID, OP_TYPE_FLOAT] {
            if let Some(&pos) = first.get(&op) {
                assert!(
                    ep < pos,
                    "{name}: OpEntryPoint（词 {ep}）必须早于类型指令（词 {pos}）—— \
                     段序错误会让驱动静默不画"
                );
            }
        }
        // 全局变量与所有类型/常量都必须在第一条 OpFunction 之前
        if let Some(&func) = first.get(&OP_FUNCTION) {
            if let Some(&var) = first.get(&OP_VARIABLE) {
                assert!(
                    var < func,
                    "{name}: 全局 OpVariable（词 {var}）必须在 OpFunction（词 {func}）之前"
                );
            }
        }
    }
    println!("全部着色器的段序正确（OpEntryPoint 早于类型、全局变量早于函数）✅");
}
