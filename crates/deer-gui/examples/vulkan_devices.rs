//! 功能示例：**Vulkan 后端能枚举到哪些 GPU**（GPU HAL + 真机驱动）。
//!
//! ```sh
//! cargo run -p deer-gui --example vulkan_devices
//! ```
//!
//! 这是里程碑 M2a 当前的能力边界：**能打开设备、能对着色器做驱动验收，
//! 但还不能渲染出图**（M2a-3 起）。本示例把边界如实打印出来，
//! 避免「跑成功了」被误读成「GPU 渲染可用」。
//!
//! 无 Vulkan 环境时会优雅退出（打印说明），不报错 —— 上层可以回退 CPU 后端。

// `name()` / `adapters()` 来自 HAL 的 `Backend` trait —— **必须把 trait 引进作用域**
// 才能调用（Rust 的 trait 方法规则）。这是新手常见困惑点。
use deer_gpu::Backend;

fn main() {
    // ① GPU HAL：任何后端都通过同一个 trait 暴露能力
    let cpu = deer_gpu::null::CpuBackend::new();
    println!("=== GPU HAL ===");
    println!("后端 {} 的适配器：", cpu.name());
    for (i, a) in cpu.adapters().iter().enumerate() {
        println!("  [{i}] {} | {:?} | {}", a.name, a.kind, a.driver);
    }
    println!("（CPU 后端是**参考实现**：它把绘制列表光栅化成像素，用于对照与无 GPU 环境）");

    // ② Vulkan 后端：实例 + 物理设备枚举
    println!("\n=== Vulkan 后端 ===");
    match deer_vk::VkBackend::new() {
        Ok(vk) => {
            println!("后端名: {}", vk.name());
            for (i, a) in vk.adapters().iter().enumerate() {
                println!("  [{i}] {} | {:?} | {}", a.name, a.kind, a.driver);
            }
            assert!(!vk.adapters().is_empty(), "构造成功就应当有至少一个适配器");
        }
        Err(e) => {
            println!("本机没有可用的 Vulkan：{e}");
            println!("⇒ 可以继续用 CPU 后端（功能等价，只是慢）");
            return;
        }
    }

    // ③ 打开逻辑设备（M2a-2）。失败不影响 CPU 后端可用。
    println!("\n=== 打开逻辑设备 ===");
    match deer_vk::VkDevice::open(0) {
        Ok(dev) => {
            let a = dev.adapter();
            println!("设备: {}", a.name);
            println!("图形队列族索引: {}", dev.queue_family_index());
            println!("内存类型数: {}", dev.memory_type_count());

            // ④ 着色器驱动验收（M2a-1）：自研 SPIR-V 汇编器的产物真的被驱动接受
            println!("\n=== 着色器驱动验收 ===");
            let vs = deer_vk::spirv::vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]);
            let fs = deer_vk::spirv::fragment_shader_solid([0.2, 0.6, 1.0, 1.0]);
            for (name, bytes) in [("顶点着色器", vs), ("片段着色器", fs)] {
                match dev.create_shader_module(&bytes) {
                    Ok(_) => println!("  {name}：{} 字节 → 驱动接受 ✅", bytes.len()),
                    Err(e) => println!("  {name}：驱动拒绝 ❌ {e}"),
                }
            }
            dev.wait_idle().expect("设备应能空闲等待");
        }
        Err(e) => println!("打开设备失败：{e}"),
    }

    // ⑤ 如实说明边界
    println!("\n=== 当前边界（不要误读） ===");
    println!("  ✅ 能：枚举 GPU、打开逻辑设备、对着色器做驱动验收");
    println!("  ✅ 能：离屏出图 + 回读（M2a-4..6）与渲染到窗口 / 上屏（M2b）");
    println!("  ❌ 不能：把界面树（DrawList）送到 GPU —— 那是 M3");
    println!("\n出图示例：cargo run -p deer-gui --example render_to_png（CPU）");
    println!("          cargo run -p deer-gui --example gpu_offscreen（Vulkan 离屏）");
    println!("真窗口上屏：cargo run -p deer-gui --features window --example window_preview");
}
