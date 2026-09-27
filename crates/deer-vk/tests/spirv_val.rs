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

/// SPIR-V `StorageClass` 值（解接口用）：`Input = 1`、`Output = 3`。
const STORAGE_CLASS_INPUT: u32 = 1;
const STORAGE_CLASS_OUTPUT: u32 = 3;

fn word_vec(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// 把 SPIR-V 拆成指令流：返回 `(opcode, 起始词下标)`，并断言词数自洽。
fn instrs(bytes: &[u8], what: &str) -> Vec<(u16, usize)> {
    let w = word_vec(bytes);
    assert_eq!(w[0], 0x0723_0203, "{what}: 魔数必须是 0x07230203");
    let mut out = Vec::new();
    let mut i = 5usize;
    while i < w.len() {
        let wc = (w[i] >> 16) as usize;
        assert!(wc >= 1, "{what}: 词 {i} 声明词数为 0");
        assert!(i + wc <= w.len(), "{what}: 词 {i} 声明 {wc} 词但越界");
        out.push(((w[i] & 0xffff) as u16, i));
        i += wc;
    }
    assert_eq!(i, w.len(), "{what}: 指令流必须正好用完整份模块");
    out
}

/// 找 `OpCompositeExtract(source, index)` 的 result-id。
fn extract_of(bytes: &[u8], what: &str, source: u32, index: u32) -> Option<u32> {
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    let w = word_vec(bytes);
    for &(op, i) in &instrs(bytes, what) {
        if op == OP_COMPOSITE_EXTRACT && w[i + 3] == source && w[i + 4] == index {
            return Some(w[i + 2]);
        }
    }
    None
}

/// 取「某个存储类的变量被装饰的 location 集合」（升序去重）。
fn locations_of(bytes: &[u8], what: &str, storage_class: u32) -> Vec<u32> {
    const OP_VARIABLE: u16 = 59;
    const OP_DECORATE: u16 = 71;
    const DECORATION_LOCATION: u32 = 30;
    let w = word_vec(bytes);
    let mut vars: std::collections::BTreeSet<u32> = Default::default();
    for &(op, i) in &instrs(bytes, what) {
        if op == OP_VARIABLE && w[i + 3] == storage_class {
            vars.insert(w[i + 2]);
        }
    }
    let mut locs: std::collections::BTreeSet<u32> = Default::default();
    for &(op, i) in &instrs(bytes, what) {
        if op == OP_DECORATE && w[i + 2] == DECORATION_LOCATION && vars.contains(&w[i + 1]) {
            locs.insert(w[i + 3]);
        }
    }
    locs.into_iter().collect()
}

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

/// M3b 新增的两支**采样**着色器（文本/字形管线）。
///
/// 与 [`rect_attrs_shaders`] 分开列，是为了让「采样指令 + 描述符」这条新路径
/// 有**独立**的专项校验（见 `text_shaders_validate`），而不是混在通用流程里
/// 只看到一个「没通过」。
fn text_shaders() -> [(&'static str, Vec<u8>); 2] {
    [
        ("vs_text", spirv::vertex_shader_text()),
        ("fs_text", spirv::fragment_shader_text()),
    ]
}

/// B5-1 新增的两支**统一管线**着色器（形状与文本合成一条管线）。
fn unified_shaders() -> [(&'static str, Vec<u8>); 2] {
    [
        ("vs_unified", spirv::vertex_shader_unified()),
        ("fs_unified", spirv::fragment_shader_unified()),
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
    .chain(text_shaders())
    .chain(unified_shaders())
    .collect()
}

/// M3b（采样着色器）的专项校验。
///
/// 自研汇编器**第一次**接触采样指令 + 描述符，所以这里除了「喂给官方
/// `spirv-val`」之外，还逐条钉住**描述符接口**的正确性 —— 因为这几条
/// 校验器**不一定**会报（它不知道 Rust 侧的 `VkDescriptorSetLayout` 长什么样）：
///
/// | 检查 | 为什么非查不可 |
/// |---|---|
/// | `OpDecorate %tex DescriptorSet 0` | 缺了它，`vkCreateGraphicsPipelines` 会说「着色器用了 set 0 但没有布局」或直接采到空描述符 |
/// | `OpDecorate %tex Binding 0` | 与 `DescriptorSetLayoutBinding.binding = 0` 必须一致；不一致是**静默采样到未定义内容** |
/// | `OpTypeImage ... Sampled 1` | `Sampled = 1` 表示「只采样不同步读写」；写成 2 会让布局校验失败 |
/// | `OpImageSampleImplicitLod` **不带** `ImageOperands` | 本机实测：误加 `Lod 0` 会被 `spirv-val` 拒绝（`Lod` 只许配 `*ExplicitLod`） |
#[test]
fn text_shaders_validate() {
    const OP_TYPE_IMAGE: u16 = 25;
    const OP_TYPE_SAMPLED_IMAGE: u16 = 27;
    const OP_IMAGE_SAMPLE_IMPLICIT_LOD: u16 = 87;
    const OP_DECORATE: u16 = 71;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    // ImageOperands 里的任何一位都不该出现（下面是几个常见位的并）
    const ANY_IMAGE_OPERAND_BITS: u32 = 0x3ff;

    // ① 纯字节自检（任何机器都能跑）
    for (name, code) in text_shaders() {
        assert_eq!(code.len() % 4, 0, "{name}: SPIR-V 必须 4 字节对齐");
        let words: Vec<u32> = code
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(words[0], 0x0723_0203, "{name}: magic");
        assert!(words[3] > 0, "{name}: 头部 bound 必须 > 0");
        let mut i = 5usize;
        while i < words.len() {
            let wc = (words[i] >> 16) as usize;
            assert!(wc >= 1, "{name}: 词 {i} 声明词数 0");
            assert!(i + wc <= words.len(), "{name}: 词 {i} 越界（wc={wc}）");
            i += wc;
        }
        assert_eq!(i, words.len(), "{name}: 指令流必须正好用完整份模块");
    }

    // ② 片元着色器的「采样 + 描述符」结构
    let fs: Vec<u32> = spirv::fragment_shader_text()
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let mut image_ty_count = 0usize;
    let mut sampled_ty_count = 0usize;
    let mut sample_count = 0usize;
    let mut samples_with_image_operands = 0usize;
    let mut binding0 = false;
    let mut set0 = false;
    let mut i = 5usize;
    while i < fs.len() {
        let wc = (fs[i] >> 16) as usize;
        let op = (fs[i] & 0xffff) as u16;
        match op {
            OP_TYPE_IMAGE => {
                image_ty_count += 1;
                // OpTypeImage result sampledType dim depth arrayed ms sampled format [access]
                // 操作数表：i+0 首字, i+1 result, i+2 sampledType, i+3 dim, i+4 depth,
                //           i+5 arrayed, i+6 ms, i+7 sampled, i+8 format
                assert!(
                    wc >= 9,
                    "OpTypeImage 至少要 9 个词（含 access qualifier 可省），实得 {wc}"
                );
                assert_eq!(fs[i + 3], 1, "OpTypeImage 的 dim 必须是 1（2D）");
                assert_eq!(fs[i + 7], 1, "OpTypeImage 的 sampled 必须是 1（只采样）");
                // image format = Unknown(0)：Vulkan 要求采样图像用 Unknown
                assert_eq!(fs[i + 8], 0, "OpTypeImage 的 format 必须是 Unknown(0)");
            }
            OP_TYPE_SAMPLED_IMAGE => sampled_ty_count += 1,
            OP_IMAGE_SAMPLE_IMPLICIT_LOD => {
                sample_count += 1;
                // OpImageSampleImplicitLod resultType result sampledImage coordinate [imageOperands...]
                // 无 imageOperands ⇒ 恰好 5 个词。多出来就说明有人加了 Lod/Bias 之类
                // ——「Lod 只许配 ExplicitLod」（实测被 spirv-val 拒），别加。
                if wc > 5 && fs[i + 5] & ANY_IMAGE_OPERAND_BITS != 0 {
                    samples_with_image_operands += 1;
                }
            }
            OP_DECORATE => {
                if fs[i + 2] == DECORATION_BINDING && fs[i + 3] == 0 {
                    binding0 = true;
                }
                if fs[i + 2] == DECORATION_DESCRIPTOR_SET && fs[i + 3] == 0 {
                    set0 = true;
                }
            }
            _ => {}
        }
        i += wc;
    }
    assert_eq!(image_ty_count, 1, "片元着色器应当有**恰好一个** OpTypeImage");
    assert_eq!(sampled_ty_count, 1, "片元着色器应当有**恰好一个** OpTypeSampledImage");
    assert_eq!(sample_count, 1, "片元着色器应当**恰好一次** OpImageSampleImplicitLod");
    assert_eq!(
        samples_with_image_operands, 0,
        "隐式采样**不能**带 ImageOperands —— `Lod` 只许配 `*ExplicitLod`/`OpImageFetch`，\
         误加会被 spirv-val 拒绝（本机实测）。单层纹理的隐式 LOD 天然就是第 0 层。"
    );
    assert!(binding0, "描述符变量必须装饰 Binding 0（与 DescriptorSetLayoutBinding 一致）");
    assert!(set0, "描述符变量必须装饰 DescriptorSet 0");

    // ③ 交给官方 spirv-val（与 all_shaders 同一条流程、同一个 find_spirv_val）
    let Some(val) = find_spirv_val() else {
        println!(
            "跳过官方 spirv-val 部分：本机没有 spirv-val。\n\
             装 Vulkan SDK（含 Shader Toolchain）即可用，或设环境变量 SPIRV_VAL 指向它。\n\
             ⚠️ 纯字节自检**不能**替代 spirv-val —— 采样指令的正确性只有校验器能判。"
        );
        return;
    };
    let dir = std::env::temp_dir().join("deer_spirv_val_text");
    std::fs::create_dir_all(&dir).expect("建临时目录");
    for (name, bytes) in text_shaders() {
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
    println!("M3b 采样着色器（顶点 + 片元）通过官方 spirv-val 校验 ✅");
}

/// **回归锁**：`fs_text` 必须输出「**非预乘**」颜色（覆盖率只乘进 alpha）。
///
/// ## 为什么这条必须有
///
/// M3b 计划初稿给的是预乘版 `vec4(rgb*cov, a*cov)`。它与本管线已冻结的混合状态
/// （`SRC_ALPHA / ONE_MINUS_SRC_ALPHA`）**不兼容**：代入
/// `src_color*src_alpha + dst*(1-src_alpha)` 后 RGB 会被乘**两次** alpha，
/// 文本边缘肉眼可见地偏暗。
///
/// 而 `spirv-val` **查不出这个错** —— 两支写法都是合法 SPIR-V。所以这里从字节码
/// 层面把契约钉死：`out_color` 的 **alpha** 操作数必须是「alpha × 输入」，
/// **RGB** 操作数必须是输入颜色的分量本身。
///
/// 判据刻意写得「窄」：只认 `OpCompositeConstruct` + 恰好一条
/// `OpVectorTimesScalar` 都不许出现 —— 预乘版必然引入 `OpVectorTimesScalar`
/// （或等价的三次 `OpFMul`），一旦有人改回去这条就红。
#[test]
fn text_fragment_shader_is_not_premultiplied() {
    const OP_STORE: u16 = 62;
    const OP_COMPOSITE_CONSTRUCT: u16 = 80;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_IMAGE_SAMPLE_IMPLICIT_LOD: u16 = 87;
    const OP_F_MUL: u16 = 133;
    const OP_VECTOR_TIMES_SCALAR: u16 = 142;

    let fs: Vec<u32> = spirv::fragment_shader_text()
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    // 收集需要的指令（按 result Id 索引）
    // extracts: result -> (源 composite, 分量下标)
    let mut extracts: std::collections::BTreeMap<u32, (u32, u32)> = Default::default();
    let mut fmul_results: std::collections::BTreeSet<u32> = Default::default();
    let mut construct: Option<(u32, Vec<u32>)> = None;
    let mut sample_result: Option<u32> = None;
    let mut vector_times_scalar = 0usize;
    let mut stored: Option<(u32, u32)> = None;

    let mut i = 5usize;
    while i < fs.len() {
        let wc = (fs[i] >> 16) as usize;
        let op = (fs[i] & 0xffff) as u16;
        match op {
            OP_VECTOR_TIMES_SCALAR => vector_times_scalar += 1,
            OP_F_MUL => {
                fmul_results.insert(fs[i + 2]);
            }
            OP_COMPOSITE_EXTRACT => {
                if wc >= 5 {
                    extracts.insert(fs[i + 2], (fs[i + 3], fs[i + 4]));
                }
            }
            OP_COMPOSITE_CONSTRUCT => {
                let parts: Vec<u32> = fs[i + 3..i + wc].to_vec();
                assert_eq!(parts.len(), 4, "只认 4 分量构造（vec4 输出）");
                construct = Some((fs[i + 2], parts));
            }
            OP_IMAGE_SAMPLE_IMPLICIT_LOD => sample_result = Some(fs[i + 2]),
            OP_STORE => stored = Some((fs[i + 1], fs[i + 2])),
            _ => {}
        }
        i += wc;
    }

    // ① 绝不允许 `OpVectorTimesScalar`：它是「预乘」写法的标志
    assert_eq!(
        vector_times_scalar, 0,
        "fs_text 里不允许出现 OpVectorTimesScalar —— 它是「预乘」写法的标志，\
         而本管线的混合是 SRC_ALPHA/ONE_MINUS_SRC_ALPHA，预乘会让 RGB 被乘两次 alpha"
    );

    let sampled = sample_result.expect("fs_text 必须有 OpImageSampleImplicitLod");
    let (construct_id, parts) = construct.expect("必须有一条 4 分量 OpCompositeConstruct");
    let (_, store_val) = stored.expect("fs_text 必须有 OpStore");
    assert_eq!(
        store_val, construct_id,
        "写入 out_color 的值必须就是那条 OpCompositeConstruct 的结果"
    );

    // ② alpha（第 4 分量）必须是 `color.a * cov`：
    //    - 由 OpFMul 产出；
    //    - 一侧操作数追溯到「某个 OpLoad 出来的 vec4 的分量 3」（= 顶点颜色 alpha）；
    //    - 另一侧追溯到「采样结果的分量 0」（= cov）。
    let a_out = parts[3];
    assert!(
        fmul_results.contains(&a_out),
        "out_color 的 alpha（Id {a_out}）必须由 OpFMul 产出（= color.a * cov）；\
         直接来自别处 ⇒ 覆盖率没有乘进 alpha"
    );
    let find_mul_operands = |target: u32| -> (u32, u32) {
        let mut i = 5usize;
        while i < fs.len() {
            let wc = (fs[i] >> 16) as usize;
            if (fs[i] & 0xffff) as u16 == OP_F_MUL && fs[i + 2] == target {
                return (fs[i + 3], fs[i + 4]);
            }
            i += wc;
        }
        panic!("找不到产出 Id {target} 的 OpFMul");
    };
    let (m_a, m_b) = find_mul_operands(a_out);

    // cov 侧：必须是「采样结果的分量 0」的抽取
    let is_cov = |id: u32| extracts.get(&id) == Some(&(sampled, 0));
    assert!(
        is_cov(m_a) || is_cov(m_b),
        "OpFMul 的一侧必须是采样结果的 .r（cov）：Id({m_a},{m_b}) 均不是 \
         「%sample 的 0 号分量」的抽取结果（extracts 里有这些：{:?}）",
        extracts
            .iter()
            .filter(|(_, (c, _))| *c == sampled)
            .collect::<Vec<_>>()
    );

    // alpha 侧：必须是「某个 vec4 的分量 3」的抽取（即顶点颜色的 alpha）
    let alpha_src = if is_cov(m_a) { m_b } else { m_a };
    match extracts.get(&alpha_src) {
        Some((_, 3)) => {}
        other => panic!(
            "OpFMul 的另一侧（Id {alpha_src}）必须是「某个 vec4 的 3 号分量」的抽取\
             （顶点颜色 alpha）；实得 {other:?}"
        ),
    }

    // ③ rgb 三个分量必须**不是**乘法结果（非预乘的核心）
    for (n, part) in parts[..3].iter().enumerate() {
        assert!(
            !fmul_results.contains(part),
            "out_color 的 rgb 分量 #{n}（Id {part}）是乘法结果 —— \
             非预乘要求 rgb 原样输出，覆盖率只乘进 alpha"
        );
    }

    println!(
        "fs_text 输出为非预乘颜色（rgb 原值 + alpha×cov，零 OpVectorTimesScalar）✅ \
         —— 与管线的 SRC_ALPHA/ONE_MINUS_SRC_ALPHA 混合相容"
    );
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

/// **B5-1 统一管线着色器的专项校验**（接口一致性 + 非预乘 + 判别符 + 无控制流）。
///
/// 统一着色器是本项目**第一次**把两条路径的判据合进同一支着色器，而且是
/// **5 个 location**（分开时是 3 + 2）—— 属性越多，「接口对不上」的概率越高，
/// 而这类缺陷**驱动不会报**：
///
/// | 检查 | 为什么非查不可 |
/// |---|---|
/// | ② **接口一致性** | M3c 抓到过一次：**驱动建管线成功、照样出像素，但 FS 的输入在上一阶段没有对应输出** |
/// | ③ **非预乘** | M3b 已实证：**`spirv-val` 会放行预乘写法** —— 只有字节码级的锁能抓 |
/// | ④ **判别符** | 判别符写死或没被用上，「统一」就只是名字统一，而驱动同样不报错 |
#[test]
fn unified_shaders_validate() {
    const OP_COMPOSITE_CONSTRUCT: u16 = 80;
    const OP_COMPOSITE_EXTRACT: u16 = 81;
    const OP_IMAGE_SAMPLE_IMPLICIT_LOD: u16 = 87;
    const OP_F_MUL: u16 = 133;
    const OP_VECTOR_TIMES_SCALAR: u16 = 142;
    const OP_SELECT: u16 = 169;
    const OP_F_ORD_LESS_THAN: u16 = 184;
    const OP_CONSTANT: u16 = 43;
    const OP_CONSTANT_NULL: u16 = 46;
    const OP_LOAD: u16 = 61;
    const OP_DECORATE: u16 = 71;
    const OP_BRANCH: u16 = 249;
    const OP_BRANCH_CONDITIONAL: u16 = 250;
    const OP_LOOP_MERGE: u16 = 246;
    const OP_PHI: u16 = 245;
    const OP_SWITCH: u16 = 247;
    const OP_LABEL: u16 = 248;
    const DECORATION_LOCATION: u32 = 30;
    /// uv 在**片元输入**里的 location（冻结接口：0=rect, 1=rk, 2=color, 3=uv）
    const UV_LOCATION: u32 = 3;
    const N_VS: &str = "vs_unified";
    const N_FS: &str = "fs_unified";

    let vs = spirv::vertex_shader_unified();
    let fs_bytes = spirv::fragment_shader_unified();
    let w = word_vec(&fs_bytes);
    let list = instrs(&fs_bytes, N_FS);

    // ── ① 纯字节自检（任何机器都能跑） ───────────────────────────────────────
    for (name, code) in unified_shaders() {
        assert_eq!(code.len() % 4, 0, "{name}: SPIR-V 必须 4 字节对齐");
        let hdr = word_vec(&code);
        assert_eq!(hdr[0], 0x0723_0203, "{name}: magic");
        assert!(hdr[3] > 0, "{name}: 头部 bound 必须 > 0");
        assert!(!instrs(&code, name).is_empty(), "{name}: 必须有指令");
    }

    // ── ② 接口一致性：各类 location 集合**恰好**是那样，且 VS 输出 ⊇ FS 输入 ──
    //
    // 「5 个 location」在两个**不同作用域**上各有含义，别混：
    //   · 顶点阶段 **输入** location 0..4 = 顶点属性（顶点缓冲 stride 52）
    //   · 顶点阶段 **输出** location 0..3 = 传给片元的 4 个属性
    //   · 片元阶段 **输入** location 0..3 = 必须与上一阶段输出**逐一对上**
    // `gl_Position` / `gl_FragCoord` 是内建，走 `BuiltIn` 装饰、**不占 location**。
    {
        let vs_in = locations_of(&vs, N_VS, STORAGE_CLASS_INPUT);
        let vs_out = locations_of(&vs, N_VS, STORAGE_CLASS_OUTPUT);
        let fs_in = locations_of(&fs_bytes, N_FS, STORAGE_CLASS_INPUT);

        assert_eq!(
            vs_in,
            vec![0, 1, 2, 3, 4],
            "{N_VS}: 顶点**输入**必须**恰好 5 个** location 0..4（顶点属性）—— \
             少一个就是「某个属性没接上」，多一个就是接口与 UnifiedVertex 不一致"
        );
        assert_eq!(
            vs_out,
            vec![0, 1, 2, 3],
            "{N_VS}: 顶点**输出**必须是 4 个 location 0..3（gl_Position 走 BuiltIn 不占 location）"
        );
        assert_eq!(fs_in, vec![0, 1, 2, 3], "{N_FS}: 片元**输入**必须是 4 个 location 0..3");
        for loc in &fs_in {
            assert!(
                vs_out.contains(loc),
                "{N_FS} 在 location {loc} 有输入，但 {N_VS} 没有对应输出（接口不匹配）—— \
                 这正是 M3c 那个「驱动建管线成功、照样出像素、但上一阶段没有对应输出」的\
                 静默缺陷形态；VS 输出 {vs_out:?}、FS 输入 {fs_in:?}"
            );
        }

        // 反向自检：判据必须**真的能**发现不匹配。用一支输出 location 不足的既有
        // 着色器当反例：`vs_text` 输出 [0,1]，拿它去对 `fs_in`（含 3）必须不通过。
        let vs_text_out =
            locations_of(&spirv::vertex_shader_text(), "vs_text", STORAGE_CLASS_OUTPUT);
        assert_eq!(vs_text_out, vec![0, 1], "vs_text 的输出集合（反例素材）");
        assert!(
            !fs_in.iter().all(|l| vs_text_out.contains(l)),
            "反向自检失败：拿输出 location 不足的 vs_text（{vs_text_out:?}）去对 {N_FS} 的\
             输入 {fs_in:?}，判据竟然**通过**了 ⇒ 这条护栏是空转的"
        );
    }

    // ── ②b 兄弟着色器同样过一遍接口检查（防「只给新管线查、旧的漂移」） ──────
    for (vs_name, vs_bytes, fs_name, fs_bytes, fs_expect) in [
        (
            "vs_rect_attrs",
            spirv::vertex_shader_rect_attrs(),
            "fs_rect_shape",
            spirv::fragment_shader_rect_shape(),
            vec![0, 1, 2],
        ),
        (
            "vs_text",
            spirv::vertex_shader_text(),
            "fs_text",
            spirv::fragment_shader_text(),
            vec![0, 1],
        ),
    ] {
        let vs_out = locations_of(&vs_bytes, vs_name, STORAGE_CLASS_OUTPUT);
        let fs_in = locations_of(&fs_bytes, fs_name, STORAGE_CLASS_INPUT);
        assert_eq!(fs_in, fs_expect, "{fs_name}: 输入 location 集合");
        for loc in &fs_in {
            assert!(
                vs_out.contains(loc),
                "{fs_name} 在 location {loc} 有输入，但 {vs_name} 没有对应输出；VS 输出 {vs_out:?}"
            );
        }
    }

    // ── ③ 非预乘回归锁（字节码级） ───────────────────────────────────────────
    // 语义：`text_out = vec4(color.rgb, color.a * cov)`。M3b 已实证
    // **`spirv-val` 会放行预乘** ⇒ 只有这把锁能抓。
    let text_out_alpha;
    {
        let mut extract_map: std::collections::BTreeMap<u32, (u32, u32)> = Default::default();
        let mut fmul_of: std::collections::BTreeMap<u32, (u32, u32)> = Default::default();
        let mut fmul_results: std::collections::BTreeSet<u32> = Default::default();
        let mut constructs: Vec<(u32, Vec<u32>)> = Vec::new();
        let mut sample_result: Option<u32> = None;
        let mut vector_times_scalar = 0usize;

        for &(op, i) in &list {
            match op {
                OP_VECTOR_TIMES_SCALAR => vector_times_scalar += 1,
                OP_F_MUL => {
                    fmul_results.insert(w[i + 2]);
                    fmul_of.insert(w[i + 2], (w[i + 3], w[i + 4]));
                }
                OP_COMPOSITE_EXTRACT => {
                    let wc = (w[i] >> 16) as usize;
                    if wc >= 5 {
                        extract_map.insert(w[i + 2], (w[i + 3], w[i + 4]));
                    }
                }
                OP_COMPOSITE_CONSTRUCT => {
                    let wc = (w[i] >> 16) as usize;
                    constructs.push((w[i + 2], w[i + 3..i + wc].to_vec()));
                }
                OP_IMAGE_SAMPLE_IMPLICIT_LOD => sample_result = Some(w[i + 2]),
                _ => {}
            }
        }

        assert_eq!(
            vector_times_scalar, 0,
            "{N_FS} 里不允许出现 OpVectorTimesScalar —— 它是「预乘」写法的标志，\
             而本管线混合是 SRC_ALPHA/ONE_MINUS_SRC_ALPHA（预乘会让 RGB 被乘两次 alpha）"
        );

        let sampled = sample_result.expect("fs_unified 必须有一次纹理采样（文本那一支）");
        let is_cov = |id: u32| extract_map.get(&id) == Some(&(sampled, 0));

        let (cov_mul_id, (ma, mb)) = fmul_of
            .iter()
            .find(|(_, (a, b))| is_cov(*a) || is_cov(*b))
            .map(|(r, ab)| (*r, *ab))
            .expect(
                "fs_unified 必须有一条 OpFMul 把**采样覆盖率**（%sample 的 0 号分量）乘进去 \
                 —— 否则覆盖率没参与运算（文本边缘会是硬边）",
            );
        let alpha_side = if is_cov(ma) { mb } else { ma };
        match extract_map.get(&alpha_side) {
            Some((_, 3)) => {}
            other => panic!(
                "{N_FS}: 覆盖率乘法的另一侧（Id {alpha_side}）必须是「某个 vec4 的 3 号分量」\
                 的抽取（= color.a）；实得 {other:?}"
            ),
        }
        text_out_alpha = cov_mul_id;

        let text_out = constructs
            .iter()
            .find(|(_, parts)| parts.len() == 4 && parts[3] == cov_mul_id)
            .unwrap_or_else(|| {
                panic!(
                    "{N_FS}: 找不到「以 alpha = cov 的乘法结果(Id {cov_mul_id}) 为第 4 分量」的 \
                     4 分量 `OpCompositeConstruct`（= text_out `vec4(color.rgb, a*cov)`）；\
                     现有 construct：{constructs:?}"
                )
            });

        // **非预乘的核心**：该 vec4 的 rgb 不得是任何乘法结果，且必须原样来自 color
        for (n, part) in text_out.1[..3].iter().enumerate() {
            assert!(
                !fmul_results.contains(part),
                "{N_FS}: text_out 的 rgb 分量 #{n}（Id {part}）是**乘法结果** —— \
                 非预乘要求 rgb 原样输出，覆盖率只乘进 alpha。\
                 （改成预乘时 `spirv-val` 会照样放行：M3b 已实证 —— 只有这把锁能抓）"
            );
            match extract_map.get(part) {
                Some((_, c)) if *c == n as u32 => {}
                other => panic!(
                    "{N_FS}: text_out 的 rgb 分量 #{n}（Id {part}）应当是 color 的第 {n} 号分量的\
                     抽取（非预乘 = rgb 原样透传）；实得 {other:?}"
                ),
            }
        }
    }

    // ── ④ 判别符：`uv < 0.0`，且其结果**真的驱动**最终二选一 ─────────────────
    //
    // ## 这条护栏改了四版，最终版由**实测字节码**定形（留档：它是同型陷阱的标本）
    //
    // | 版本 | 判据 | 实测结果 |
    // |---|---|---|
    // | 1 | 「存在某条 `OpFOrdLessThan`，一侧是某个 vec 的 0 号分量抽取」 | **空转**：匹配到形状判据里的 `dl < 0`（`dl = px - rx`）⇒ 判别符写死成 `OpConstantTrue` 后**照样绿** |
    // | 2/3 | 拿「采样坐标 Id」或「uv 的 `OpLoad` 结果」直接匹配 | **少穿透一层抽取**：比较用的是 `OpLoad` 结果的 `.x` 抽取 ⇒ 实测 0 条命中（红） |
    // | 4（本版） | **语义锚点 + 穿透一层抽取**：`Location=3` 的输入 = uv → 它的 `OpLoad` 结果及其 `.x` 抽取 → 找「它与 **0.0** 常量比较」的 `OpFOrdLessThan` → 并要求其结果驱动 **≥4 条 `OpSelect`** | 绿（正确）/ 红（写死或阈值改过）✅ |
    //
    // **实测事实**（981 词）：
    // ```text
    //   var 20 = 输入，Location = 3          ⇒ uv
    //   29     = OpLoad var20                ⇒ uv 的值（**不被比较直接用**）
    //   165    = OpCompositeExtract(29, 0)   ⇒ uv.x
    //   166    = OpFOrdLessThan(165, 24)     ⇒ 判别符（24 = OpConstant 0.0）
    //   …4 条 OpSelect(166, shape_i, text_i) ⇒ 逐分量最终二选一
    // ```
    // **教训**：写字节码级判据前，先把真实字节码打印出来 —— 靠记忆猜编码、或
    // 少穿透一层 `OpCompositeExtract`，都会写出**空转**或**过严**的护栏。
    {
        // ① 值为 0.0 的常量 Id
        let mut zero_consts: std::collections::BTreeSet<u32> = Default::default();
        for &(op, i) in &list {
            match op {
                OP_CONSTANT => {
                    let wc = (w[i] >> 16) as usize;
                    if wc >= 4 && w[i + 3] == 0 {
                        zero_consts.insert(w[i + 2]);
                    }
                }
                OP_CONSTANT_NULL => {
                    zero_consts.insert(w[i + 2]);
                }
                _ => {}
            }
        }
        assert!(
            !zero_consts.is_empty(),
            "{N_FS}: 没找到值为 0.0 的常量 —— 判别符 `uv < 0` 必然要有一个"
        );

        // ② 语义锚点：`Location = 3` 的片元输入就是 uv
        let mut uv_var: Option<u32> = None;
        for &(op, i) in &list {
            if op == OP_DECORATE && w[i + 2] == DECORATION_LOCATION && w[i + 3] == UV_LOCATION {
                uv_var = Some(w[i + 1]);
            }
        }
        let uv_var = uv_var.unwrap_or_else(|| {
            panic!("{N_FS}: 找不到 Location = {UV_LOCATION} 的输入变量（冻结接口里那是 uv）")
        });

        // ③ 由 uv 变量 `OpLoad` 出来的值
        let mut uv_loads: std::collections::BTreeSet<u32> = Default::default();
        for &(op, i) in &list {
            if op == OP_LOAD && w[i + 3] == uv_var {
                uv_loads.insert(w[i + 2]);
            }
        }
        assert!(
            !uv_loads.is_empty(),
            "{N_FS}: uv 变量（Id {uv_var}）没有被 `OpLoad` 过 —— 判别符不可能成立"
        );

        // ④ 穿透一层抽取：比较用的可能是 load 结果，也可能是它的 `.x` 抽取
        let uv_parts: std::collections::BTreeSet<u32> = uv_loads
            .iter()
            .flat_map(|l| {
                std::iter::once(*l).chain(extract_of(&fs_bytes, N_FS, *l, 0))
            })
            .collect();

        let mut disc: Vec<u32> = Vec::new();
        for &(op, i) in &list {
            if op != OP_F_ORD_LESS_THAN {
                continue;
            }
            let (a, b) = (w[i + 3], w[i + 4]);
            if (uv_parts.contains(&a) && zero_consts.contains(&b))
                || (uv_parts.contains(&b) && zero_consts.contains(&a))
            {
                disc.push(w[i + 2]);
            }
        }
        assert_eq!(
            disc.len(),
            1,
            "{N_FS}: 必须**恰好有 1 条** `OpFOrdLessThan` 把「uv 的值或它的 0 号分量\
             （Id {uv_parts:?}，其中 {uv_loads:?} 是 uv 的 `OpLoad` 结果）」与 \
             「**值为 0.0 的**常量（Id {zero_consts:?}）」相比 —— 那一条就是判别符 `uv.x < 0`。\
             实得 {} 条（Id {disc:?}）。0 条 ⇒ 判别符被写死成常量、换了别的属性、\
             或把阈值从 0.0 改成了别的数 ⇒ 两条路径中的一条会渲染错，而驱动**不会**报错。",
            disc.len()
        );

        // ⑤ 判别符的结果必须**真的被用上**：以它（或它经一次抽取后的标量）为条件的
        //    `OpSelect` 至少 4 条（逐分量构造最终 vec4）。
        let disc_scalars: std::collections::BTreeSet<u32> =
            std::iter::once(disc[0]).chain(extract_of(&fs_bytes, N_FS, disc[0], 0)).collect();
        let selects_using = list
            .iter()
            .filter(|(op, _)| *op == OP_SELECT)
            .filter(|(_, i)| disc_scalars.contains(&w[i + 3]))
            .count();
        assert!(
            selects_using >= 4,
            "{N_FS}: 判别符（Id {}，标量形式 {disc_scalars:?}）必须至少做 4 条 `OpSelect` 的\
             条件（逐分量二选一）；实得 {selects_using} 条。0 条 ⇒ 判别符算了但**没被用上**，\
             即「统一只是名字统一」。",
            disc[0]
        );

        println!(
            "fs_unified 判别符 ✅：`uv`(Id {uv_loads:?}) < `0.0` ⇒ 比较结果 Id {}，\
             驱动 {selects_using} 条 OpSelect 做逐分量二选一",
            disc[0]
        );
    }

    println!(
        "fs_unified 非预乘 ✅（alpha = cov × color.a，Id {text_out_alpha}；rgb 原样；\
         零 OpVectorTimesScalar）—— 与 SRC_ALPHA/ONE_MINUS_SRC_ALPHA 混合相容"
    );

    // ── ⑤ 无控制流（统一不等于引入分支） ─────────────────────────────────────
    for (name, bytes) in unified_shaders() {
        let mut labels = 0usize;
        for (op, _) in instrs(&bytes, name) {
            assert!(
                !matches!(
                    op,
                    OP_BRANCH | OP_BRANCH_CONDITIONAL | OP_LOOP_MERGE | OP_PHI | OP_SWITCH
                ),
                "{name}: 不允许控制流指令（opcode {op}）—— 判别必须用 OpSelect 而不是分支"
            );
            if op == OP_LABEL {
                labels += 1;
            }
        }
        assert_eq!(labels, 1, "{name}: 必须是单基本块（实得 {labels} 个 OpLabel）");
    }

    // ── ⑥ 官方 spirv-val（与 all_shaders 同一条流程、同一个 find_spirv_val） ──
    let Some(val) = find_spirv_val() else {
        println!("跳过官方 spirv-val（未找到可执行文件）—— 上面的纯字节自检仍然有效");
        return;
    };
    for (name, bytes) in unified_shaders() {
        let path = std::env::temp_dir().join(format!("deer_b5_{name}.spv"));
        std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("写 {name} 失败：{e}"));
        let out = Command::new(&val)
            .arg(&path)
            .output()
            .unwrap_or_else(|e| panic!("跑 spirv-val 失败：{e}"));
        assert!(
            out.status.success(),
            "{name} 未通过官方 spirv-val：\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        println!("  {name}: 通过官方 spirv-val ✅");
    }
}
