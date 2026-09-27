//! 把手写着色器汇编器生成的所有 SPIR-V 写到 `spirv_probe/*.spv`，
//! 交给 Khronos 官方 `spirv-val` 校验（**权威判据，不靠推理**）。
//!
//! ```sh
//! cargo test -p deer-vk --test export_spirv
//! C:/VulkanSDK/1.4.357.0/Bin/spirv-val.exe spirv_probe/vs_triangle.spv
//! ```
//!
//! 为什么要这一步：本项目自研了 SPIR-V 汇编器（不依赖 SDK 的 `glslc`）。
//! `vkCreateShaderModule` 极宽容（连非法 `bound` 都接受），而驱动在编译时
//! **不报错也不画** —— 所以必须有一个**独立**的校验器来判断产物是否合法。

use deer_vk::spirv;

#[test]
fn write_all_shaders_for_spirv_val() {
    let out = std::path::Path::new("spirv_probe");
    std::fs::create_dir_all(out).ok();

    let shaders: Vec<(&str, Vec<u8>)> = vec![
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
        // M3a（矩形属性着色器）：顶点属性透传 + 片元形状判据
        ("vs_rect_attrs", spirv::vertex_shader_rect_attrs()),
        ("fs_rect_shape", spirv::fragment_shader_rect_shape()),
    ];

    println!("写出 {} 个 .spv 到 spirv_probe/", shaders.len());
    for (name, bytes) in &shaders {
        let p = out.join(format!("{name}.spv"));
        std::fs::write(&p, bytes).unwrap_or_else(|e| panic!("写 {} 失败：{e}", p.display()));
        println!("  {:<26} {} 字节", p.display(), bytes.len());
    }
}
