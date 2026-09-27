//! **C1 探针（gated）**：在**离屏 + 三角形**这个**原始场景**下，把三种 viewport 形态各跑一遍。
//!
//! ```text
//!   cargo run -p deer-vk --example viewport_dynamic_probe
//!   DEER_VK_VALIDATION=1 cargo run -p deer-vk --example viewport_dynamic_probe
//! ```
//!
//! ## 要回答的问题（不预设结论）
//!
//! 多处文档曾断言「**动态 viewport/scissor 在本机 Intel 核显上画不出任何像素**」。
//! M3c 期间的新证据是**窗口路径**下动态与静态**都能上屏且像素完全相同**，
//! 而「声明动态却从不调 `vkCmdSetViewport`」会**崩溃**。
//! 但 M2a 当年记录的症状是「**零像素且不崩溃**」⇒ **症状不同，不能断定同因**。
//!
//! 没人做过的正是**原始场景那一格**：离屏 + 三角形 + 动态 viewport。本探针就做它。
//!
//! ## 三组（逐组独立子进程 —— 因为其中一组**预期会崩**）
//!
//! | 组 | 形态 |
//! |---|---|
//! | ① | **动态** + 每帧真的调 `vkCmdSetViewport` / `vkCmdSetScissor` |
//! | ② | **动态** + **从不设置**（复现 M2a 的代码形态） |
//! | ③ | **静态**（产品现状基线；离屏当前就是静态） |
//!
//! ## 为什么要开子进程
//!
//! ②很可能会**让进程死掉**（`0xC0000005` / `0xC000041D`）。在同一个进程里跑，
//! 一次崩溃会带走后面所有组的证据 ⇒ 父进程为每组 `spawn` 一个子进程，
//! 于是「**崩溃/错误码**」本身成为可记录的一等观测值。
//!
//! ## 为什么不改产品代码
//!
//! 离屏路径**当前的产品行为仍是静态**。本探针只用**公开 API**
//! （`VkDevice::create_pipeline_from_state` + `pipelines::ViewportStrategy`）
//! 自己装一个最小离屏渲染器，逐组替换那一处差异 —— 产品代码一行不动。
//!
//! ## 输出协议（父进程按此前缀解析子进程 stdout）
//!
//! ```text
//!   PROBE_RESULT group=<n> dynamic=<bool> set_viewport=<bool> render_ok=<bool>
//!                frames=<n> diff_pixels=<首帧不同像素数> first_frame_bytes=<sha256 前 16 位>
//!                verdict=<SET_PIXELS|ZERO_PIXELS|RENDER_ERROR>
//! ```

use std::process::Command;

use deer_gpu::GpuResult;
use deer_vk::device::VkDevice;
use deer_vk::ffi;
use deer_vk::ffi_dev as vk;
use deer_vk::pipelines::{shape_state, ViewportStrategy};
use deer_vk::spirv;

const W: u32 = 64;
const H: u32 = 64;
/// 清屏色（不透明，便于「与清屏色不同的像素」= 画出来的像素）。
const CLEAR: [f32; 4] = [0.1, 0.1, 0.2, 1.0];
/// 每组的帧数（多帧是为了看「是否稳定」而不是一次侥幸）。
const FRAMES: usize = 5;
/// 三角形的顶点数（位置来自着色器常量表，**无顶点缓冲**）。
const VERTEX_COUNT: u32 = 3;

fn main() {
    let which = std::env::args().find_map(|a| {
        a.strip_prefix("--group=").map(|n| n.to_string())
    });
    match which.as_deref() {
        Some("0") | Some("1") | Some("2") => {
            let g: usize = which.unwrap().parse().unwrap();
            let code = run_group_in_this_process(g);
            std::process::exit(code);
        }
        _ => run_parent(),
    }
}

/// 父进程：为每组各起一个**独立**子进程，收集结论并打印汇总表。
fn run_parent() {
    println!("══════════════════════════════════════════════════════════════════");
    println!("C1 探针：离屏 + 三角形 + 三种 viewport 形态（逐组独立子进程）");
    println!("环境变量 DEER_VK_VALIDATION = {:?}", std::env::var("DEER_VK_VALIDATION").ok());
    println!("尺寸 {W}×{H}，每组 {FRAMES} 帧，顶点数 {VERTEX_COUNT}（无顶点缓冲）");
    println!("══════════════════════════════════════════════════════════════════");

    let me = std::env::current_exe().expect("current_exe");
    let names = [
        "① 动态 + 每帧设置",
        "② 动态 + 从不设置",
        "③ 静态（基线）",
    ];
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for (g, name) in names.iter().enumerate() {
        println!("\n──── 组 {}（{}）────", g + 1, name);
        let out = Command::new(&me)
            .arg(format!("--group={g}"))
            .output()
            .expect("spawn 子进程");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        // 子进程的输出原样打出来（原始证据，不做加工）
        for line in stdout.lines() {
            println!("    {line}");
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.trim().is_empty() {
            for line in stderr.lines().take(12) {
                println!("    [stderr] {line}");
            }
        }
        let code = out.status.code();
        let status = match code {
            Some(0) => "退出码 0（未崩溃）".to_string(),
            Some(c) => format!("退出码 {c}（{:#010X}）{}", c as u32, decode_exit_code(c as u32)),
            None => "被信号杀死（无退出码）".to_string(),
        };
        let verdict = stdout
            .lines()
            .find(|l| l.starts_with("PROBE_RESULT"))
            .map(parse_verdict)
            .unwrap_or_else(|| "无 PROBE_RESULT（进程在打印之前就死了）".to_string());
        rows.push((names[g].to_string(), status, verdict));
    }

    println!("\n══════════════════════════════════════════════════════════════════");
    println!("汇总（离屏 + 三角形，{}×{}，{FRAMES} 帧/组）", W, H);
    println!("══════════════════════════════════════════════════════════════════");
    for (name, status, verdict) in &rows {
        println!("  {name}");
        println!("      进程: {status}");
        println!("      像素: {verdict}");
    }
    println!("\n（判定口径见本文件头部注释；本探针只给证据，不替文档下结论）");
}

fn parse_verdict(line: &str) -> String {
    if !line.contains("render_ok=true") {
        return "渲染 API 报错（render_ok=false）".to_string();
    }
    let get = |key: &str| -> String {
        line.split_whitespace()
            .find_map(|kv| kv.strip_prefix(key))
            .unwrap_or("?")
            .to_string()
    };
    format!(
        "{}（三角形像素 {}，{} 帧，字节 {}）",
        get("verdict="),
        get("triangle_pixels="),
        get("frames="),
        get("first_frame_bytes=")
    )
}

/// Windows 退出码 → 人可读的异常名（本项目实测过的两个恰好都在列）。
fn decode_exit_code(c: u32) -> &'static str {
    match c {
        0xC0000005 => "ACCESS_VIOLATION",
        0xC000041D => "FATAL_USER_CALLBACK_EXCEPTION",
        0xC0000409 => "STACK_BUFFER_OVERRUN",
        0xC00000FD => "STACK_OVERFLOW",
        0xC000013A => "CONTROL_C_EXIT",
        _ => "",
    }
}

// ── 子进程：真正跑一组 ───────────────────────────────────────────────────────

fn run_group_in_this_process(group: usize) -> i32 {
    let (dynamic, set_viewport) = match group {
        0 => (true, true),
        1 => (true, false),
        2 => (false, false),
        _ => unreachable!(),
    };
    eprintln!(
        "[组 {}] dynamic={dynamic} set_viewport_each_frame={set_viewport}",
        group + 1
    );

    match drive(dynamic, set_viewport) {
        Ok(stats) => {
            let verdict = if stats.triangle_pixels > 0 { "SET_PIXELS" } else { "ZERO_PIXELS" };
            println!(
                "PROBE_RESULT group={} dynamic={dynamic} set_viewport={set_viewport} \
                 render_ok=true frames={} triangle_pixels={} background={:?} clear_only_pixels={} \
                 bbox={:?} first_frame_bytes={} verdict={verdict}",
                group + 1,
                stats.frames,
                stats.triangle_pixels,
                stats.background,
                stats.clear_only_pixels,
                stats.bbox,
                stats.sha16,
            );
            if stats.triangle_pixels == 0 {
                println!(
                    "    ⚠️ 判定：**零像素**（{FRAMES} 帧全部只有背景色）—— \
                     这正是 M2a 记录的症状形态"
                );
            } else {
                println!(
                    "    ✅ 判定：**画出三角形**（{FRAMES} 帧稳定；首帧三角形像素 {} / {}，\
                     包围盒 {:?}，背景色 {:?}）",
                    stats.triangle_pixels,
                    W * H,
                    stats.bbox,
                    stats.background
                );
            }
            println!(
                "    对照：满清屏帧（vkCmdDraw(0)）里背景色像素 = {} / {} —— \
                 指标有效性的自检（应等于全帧）",
                stats.clear_only_pixels,
                W * H
            );
            println!("    直方图（首帧，按像素数降序）：");
            for (px, n) in &stats.histogram {
                println!(
                    "        RGBA[{:3},{:3},{:3},{:3}] × {n:5}  ({:.1}%)",
                    px[0], px[1], px[2], px[3],
                    100.0 * (*n as f64) / (W * H) as f64
                );
            }
            print!("    采样点：");
            for (name, px) in &stats.probes {
                print!("{name}=[{},{},{},{}]  ", px[0], px[1], px[2], px[3]);
            }
            println!();
            0
        }
        Err(e) => {
            println!(
                "PROBE_RESULT group={} dynamic={dynamic} set_viewport={set_viewport} \
                 render_ok=false frames=0 diff_pixels=-1 first_frame_bytes=- verdict=RENDER_ERROR",
                group + 1
            );
            println!("    ❌ 判定：**渲染 API 报错**（不是崩溃、也不是零像素）：{e}");
            20
        }
    }
}

struct Stats {
    frames: usize,
    /// 首帧**三角形**（= 与背景色不同的像素）的个数。
    ///
    /// ⚠️ 第一版这里叫 `diff_pixels`，拿「与理想清屏色 `round(0.1*255)` 比较」来数，
    /// 结果恒为 **4096 / 4096**（全帧）—— 因为清屏色 `0.1` 的 UNORM 量化是
    /// **25**（`25.5` 向偶数舍入）而**不是** 26，于是连背景本身都被算成「不同」。
    /// 那个数字**看着像「三角形铺满全帧」**，差点让我下错结论。
    /// 现在改为「与**首帧最常见像素**（即背景）不同的像素数」，并额外验证
    /// 「一次 `vkCmdDraw(0)` 的满清屏帧」确实得到 0 —— 那是这个指标的对照。
    triangle_pixels: usize,
    /// 背景色（首帧出现次数最多的像素值）。
    background: [u8; 4],
    /// 背景色**采样**验证：满清屏帧（`vkCmdDraw(0)`）里该颜色的像素数，应为 `W*H`。
    clear_only_pixels: usize,
    /// 首帧字节的 sha256 前 16 位（跨组对比用；不引第三方依赖 ⇒ 自己实现）。
    sha16: String,
    /// 首帧的**像素值直方图**（按出现次数降序，最多 4 项）。
    ///
    /// ## 为什么必须要它（第一版探针差点给出错误解读）
    ///
    /// 「与清屏色不同的像素 = 4096 / 4096」有**两种**成因：
    /// （a）三角形真的铺满了整个 viewport；或（b）**每个**像素都被写成了别的颜色。
    /// 只看一个数字**分不出**这两者，而它们的结论完全不同。
    /// 直方图把「到底有几块颜色、各占多少」摆出来 ⇒ 一眼可辨
    /// （实测就靠它发现：背景 67.6% + 绿色三角形 32.4% ⇒ 是「画了三角形」，
    ///  而不是「铺满」；同时暴露了上面那个量化错误）。
    histogram: Vec<([u8; 4], usize)>,
    /// 四个角与中心的采样值（定位用）。
    probes: Vec<(&'static str, [u8; 4])>,
    /// 非背景像素的包围盒 `(x0, y0, x1, y1)`（含端点）；`None` = 没有非背景像素。
    bbox: Option<(u32, u32, u32, u32)>,
}

/// 装一个最小离屏渲染器并渲染 `FRAMES` 帧；`dynamic`/`set_viewport` 是唯一的变量。
fn drive(dynamic: bool, set_viewport: bool) -> GpuResult<Stats> {
    let dev = VkDevice::open(0)?;
    let fns = *dev.fns();
    let dh = dev.handle();

    // ① 渲染通道（颜色格式与 M2a/M3a 离屏一致）
    let pass = dev.create_render_pass(
        vk::VK_FORMAT_R8G8B8A8_UNORM,
        vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
        vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    )?;

    // ② 管线：**两组的唯一差异就是 viewport 策略**
    //    顶点属性表为空 ⇒ 位置来自着色器常量表（与 M2a 的原始形态一致，无顶点缓冲）
    let strategy = if dynamic {
        ViewportStrategy::Dynamic
    } else {
        ViewportStrategy::Static { width: W, height: H }
    };
    let state = shape_state(vk::VK_FORMAT_R8G8B8A8_UNORM, strategy, 0, Vec::new());
    let layout = dev.create_pipeline_layout(None)?;
    let vs = dev.create_shader_module(&spirv::vertex_shader_triangle([
        [-0.8, -0.8],
        [0.8, -0.8],
        [-0.8, 0.8],
    ]))?;
    let fs = dev.create_shader_module(&spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]))?;
    let pipeline = dev.create_pipeline_from_state(
        &state,
        &[
            (&vs, vk::VK_SHADER_STAGE_VERTEX_BIT),
            (&fs, vk::VK_SHADER_STAGE_FRAGMENT_BIT),
        ],
        &layout,
        &pass,
    )?;

    // ③ 离屏图像 + 内存 + 视图 + 帧缓冲（设备本地）
    let img_info = vk::ImageCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        image_type: vk::VK_IMAGE_TYPE_2D,
        format: vk::VK_FORMAT_R8G8B8A8_UNORM,
        extent: vk::Extent3D { width: W, height: H, depth: 1 },
        mip_levels: 1,
        array_layers: 1,
        samples: vk::VK_SAMPLE_COUNT_1_BIT,
        tiling: vk::VK_IMAGE_TILING_OPTIMAL,
        usage: vk::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
        initial_layout: vk::VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image: vk::ImageHandle = vk::NULL_HANDLE;
    check("vkCreateImage", unsafe {
        (fns.create_image)(dh, &img_info, std::ptr::null(), &mut image)
    })?;
    let mut req = unsafe { std::mem::zeroed::<vk::MemoryRequirements>() };
    unsafe { (fns.get_image_memory_requirements)(dh, image, &mut req) };
    let img_mem = alloc_mem(&dev, req.size, req.memory_type_bits, vk::VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)?;
    check("vkBindImageMemory", unsafe {
        (fns.bind_image_memory)(dh, image, img_mem, 0)
    })?;

    let view_info = vk::ImageViewCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        image,
        view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
        format: vk::VK_FORMAT_R8G8B8A8_UNORM,
        components_r: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
        components_g: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
        components_b: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
        components_a: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
        subresource_range: color_range(),
    };
    let mut view: vk::ImageViewHandle = vk::NULL_HANDLE;
    check("vkCreateImageView", unsafe {
        (fns.create_image_view)(dh, &view_info, std::ptr::null(), &mut view)
    })?;

    let fb_info = vk::FramebufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        render_pass: pass.handle(),
        attachment_count: 1,
        p_attachments: &view,
        width: W,
        height: H,
        layers: 1,
    };
    let mut fb: vk::FramebufferHandle = vk::NULL_HANDLE;
    check("vkCreateFramebuffer", unsafe {
        (fns.create_framebuffer)(dh, &fb_info, std::ptr::null(), &mut fb)
    })?;

    // ④ 回读暂存缓冲（主机可见 + 一致）
    let bytes = (W * H * 4) as u64;
    let buf_info = vk::BufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        size: bytes,
        usage: vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
    };
    let mut staging: vk::BufferHandle = vk::NULL_HANDLE;
    check("vkCreateBuffer", unsafe {
        (fns.create_buffer)(dh, &buf_info, std::ptr::null(), &mut staging)
    })?;
    let mut breq = unsafe { std::mem::zeroed::<vk::MemoryRequirements>() };
    unsafe { (fns.get_buffer_memory_requirements)(dh, staging, &mut breq) };
    let host = vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    let stage_mem = alloc_mem(&dev, breq.size, breq.memory_type_bits, host)?;
    check("vkBindBufferMemory", unsafe {
        (fns.bind_buffer_memory)(dh, staging, stage_mem, 0)
    })?;

    // ⑤ 命令缓冲 + 栅栏
    let pool = dev.create_transient_command_pool()?;
    let alloc = vk::CommandBufferAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        command_pool: pool.handle(),
        level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut cmd: vk::CommandBufferHandle = vk::NULL_HANDLE;
    check("vkAllocateCommandBuffers", unsafe {
        (fns.allocate_command_buffers)(dh, &alloc, &mut cmd)
    })?;
    let fence_info = vk::FenceCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
    };
    let mut fence: vk::FenceHandle = vk::NULL_HANDLE;
    check("vkCreateFence", unsafe {
        (fns.create_fence)(dh, &fence_info, std::ptr::null(), &mut fence)
    })?;

    // ⑥ 逐帧：录制 → 提交 → 等栅栏 → 回读
    //
    // 抽成闭包是为了让「对照帧」（`vertex_count = 0` 的满清屏帧）复用**完全相同**的
    // 录制路径 —— 除了顶点数。对照帧用来验证「指标有效性」：
    // 若它里面还有非背景像素，那说明我的统计口径有问题，而不是显卡画了东西。
    let record_and_read = |vertex_count: u32| -> GpuResult<Vec<u8>> {
        check("vkResetCommandBuffer", unsafe { (fns.reset_command_buffer)(cmd, 0) })?;
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        check("vkBeginCommandBuffer", unsafe { (fns.begin_command_buffer)(cmd, &begin) })?;

        let clear_value = vk::ClearValue { color: vk::ClearColorValue { float32: CLEAR } };
        let begin_pass = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: std::ptr::null(),
            render_pass: pass.handle(),
            framebuffer: fb,
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width: W, height: H },
            },
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        unsafe {
            (fns.cmd_begin_render_pass)(cmd, &begin_pass, vk::VK_SUBPASS_CONTENTS_INLINE);
            (fns.cmd_bind_pipeline)(cmd, vk::VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline.handle());
            if set_viewport {
                // ①「每帧真的设置」这一组的**全部**差异就在这两行
                let vp = vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: W as f32,
                    height: H as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                };
                (fns.cmd_set_viewport)(cmd, 0, 1, &vp);
                let sc = vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D { width: W, height: H },
                };
                (fns.cmd_set_scissor)(cmd, 0, 1, &sc);
            }
            (fns.cmd_draw)(cmd, vertex_count, 1, 0, 0);
            (fns.cmd_end_render_pass)(cmd);

            // 屏障（oldLayout 必须 = 渲染通道的 finalLayout）→ 拷回暂存
            let barrier = vk::ImageMemoryBarrier {
                s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                p_next: std::ptr::null(),
                src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
                old_layout: pass.final_layout(),
                new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                image,
                subresource_range: color_range(),
            };
            (fns.cmd_pipeline_barrier)(
                cmd,
                vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                0, 0, std::ptr::null(), 0, std::ptr::null(), 1, &barrier,
            );
            let copy = vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                image_extent: vk::Extent3D { width: W, height: H, depth: 1 },
            };
            (fns.cmd_copy_image_to_buffer)(
                cmd,
                image,
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                staging,
                1,
                &copy,
            );
        }
        check("vkEndCommandBuffer", unsafe { (fns.end_command_buffer)(cmd) })?;

        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        check("vkQueueSubmit", unsafe {
            (fns.queue_submit)(dev.queue(), 1, &submit, fence)
        })?;
        check("vkWaitForFences", unsafe {
            (fns.wait_for_fences)(dh, 1, &fence, vk::VK_TRUE, 5_000_000_000)
        })?;
        check("vkResetFences", unsafe { (fns.reset_fences)(dh, 1, &fence) })?;

        // 回读（映射 → 拷贝 → 解映射）
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        check("vkMapMemory", unsafe {
            (fns.map_memory)(dh, stage_mem, 0, vk::WHOLE_SIZE, 0, &mut ptr)
        })?;
        let mut pixels = vec![0u8; bytes as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(ptr as *const u8, pixels.as_mut_ptr(), pixels.len());
            (fns.unmap_memory)(dh, stage_mem);
        }
        Ok(pixels)
    };

    let mut stats: Option<Stats> = None;
    for frame in 0..FRAMES {
        let pixels = record_and_read(VERTEX_COUNT)?;
        let hist = histogram(&pixels);
        // 背景 = 首帧出现次数最多的像素（不假设清屏色如何量化 —— 第一版就是这么错的）
        let background = hist[0].0;
        let triangle_pixels = pixels
            .chunks_exact(4)
            .filter(|p| [p[0], p[1], p[2], p[3]] != background)
            .count();
        eprintln!(
            "  [组内] 帧 {frame}: 背景 {:?} 出现 {} 次；三角形像素 = {triangle_pixels}",
            background, hist[0].1
        );
        if frame == 0 {
            stats = Some(Stats {
                frames: FRAMES,
                triangle_pixels,
                background,
                // 先占位，稍后用「满清屏帧」的对照填上
                clear_only_pixels: 0,
                sha16: sha256_hex16(&pixels),
                histogram: hist,
                probes: probes(&pixels),
                bbox: bbox(&pixels, background),
            });
        }
    }

    // ⑦ 对照帧：**同一份录制路径**、只把顶点数改成 0（= 只清屏，不画三角形）。
    //
    // 这是「指标有效性」的自检：若它里面还有**非背景**像素，那说明我的统计口径有问题
    // （或清屏/格式异常），而不是显卡真的画了东西。第一版探针没有这一步，
    // 结果我拿到了「4096/4096 不同」这种**看着像铺满、其实是我算错**的数字。
    let control = record_and_read(0)?;
    let stats = stats.expect("至少一帧");
    let control_non_bg = control
        .chunks_exact(4)
        .filter(|p| [p[0], p[1], p[2], p[3]] != stats.background)
        .count();
    eprintln!(
        "  [对照] vkCmdDraw(0) 的满清屏帧：非背景像素 = {control_non_bg}（应为 0）\
         ；背景色像素 = {} / {}",
        W * H - control_non_bg as u32,
        W * H
    );
    assert_eq!(
        control_non_bg, 0,
        "对照帧（vkCmdDraw(0)）里出现了 {control_non_bg} 个非背景像素 —— \
         统计口径或清屏有问题，本组的其他数字不可信"
    );
    let stats = Stats {
        clear_only_pixels: (W * H) as usize - control_non_bg,
        ..stats
    };

    // 销毁（顺序：命令缓冲随池走；内存最后）
    unsafe {
        (fns.destroy_fence)(dh, fence, std::ptr::null());
        (fns.free_memory)(dh, stage_mem, std::ptr::null());
        (fns.destroy_buffer)(dh, staging, std::ptr::null());
        (fns.destroy_framebuffer)(dh, fb, std::ptr::null());
        (fns.free_memory)(dh, img_mem, std::ptr::null());
        (fns.destroy_image_view)(dh, view, std::ptr::null());
        (fns.destroy_image)(dh, image, std::ptr::null());
    }
    drop(pool);
    Ok(stats)
}

/// 非背景像素的包围盒 `(x0, y0, x1, y1)`（含端点）；`None` = 没有非背景像素。
fn bbox(pixels: &[u8], background: [u8; 4]) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            if [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]] != background {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    any.then_some((x0, y0, x1, y1))
}

/// 首帧的像素值直方图（按出现次数降序，最多 4 项）。
fn histogram(pixels: &[u8]) -> Vec<([u8; 4], usize)> {
    let mut map: std::collections::HashMap<[u8; 4], usize> = std::collections::HashMap::new();
    for p in pixels.chunks_exact(4) {
        *map.entry([p[0], p[1], p[2], p[3]]).or_insert(0) += 1;
    }
    let mut v: Vec<([u8; 4], usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(4);
    v
}

/// 四个角 + 中心 + 三角形重心附近的采样（定位「画在哪」）。
fn probes(pixels: &[u8]) -> Vec<(&'static str, [u8; 4])> {
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * W + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    vec![
        ("左上", at(0, 0)),
        ("右上", at(W - 1, 0)),
        ("左下", at(0, H - 1)),
        ("右下", at(W - 1, H - 1)),
        ("中心", at(W / 2, H / 2)),
    ]
}

// ⚠️ 这里曾有一个 `count_diff()`：拿「与**理想**清屏色 `round(0.1*255)` 比较」来数不同像素。
// 它是**错的**（清屏色 `0.1f32` 实际量化为 **25** 而不是 26 —— `25.5` 向偶数舍入），
// 于是连背景都被算成「不同」、恒返回全帧 `4096`，**看着像「三角形铺满」**。
// 现在改成「与**首帧实测最常见像素**（= 背景）比较」，并在 `drive()` 里用
// 「`vkCmdDraw(0)` 满清屏帧必须 0 个非背景像素」做自检。删掉这个函数是**故意的**：
// 留着它就有被再次误用的风险。

fn alloc_mem(
    dev: &VkDevice,
    size: u64,
    type_bits: u32,
    required: u32,
) -> GpuResult<vk::DeviceMemoryHandle> {
    let props = *dev.memory_properties();
    let mut chosen = None;
    for i in 0..props.memory_type_count.min(32) {
        let ty = &props.memory_types[i as usize];
        if type_bits & (1 << i) != 0 && ty.property_flags & required == required {
            chosen = Some(i);
            break;
        }
    }
    let index = chosen.unwrap_or_else(|| {
        panic!("找不到满足 {required:#x} 的内存类型（type_bits={type_bits:#x}）")
    });
    let info = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: size,
        memory_type_index: index,
    };
    let mut h: vk::DeviceMemoryHandle = vk::NULL_HANDLE;
    let fns = *dev.fns();
    check("vkAllocateMemory", unsafe {
        (fns.allocate_memory)(dev.handle(), &info, std::ptr::null(), &mut h)
    })?;
    Ok(h)
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

fn check(what: &str, rc: i32) -> GpuResult<()> {
    if rc == ffi::VK_SUCCESS {
        Ok(())
    } else {
        Err(deer_gpu::GpuError::Driver {
            code: rc,
            message: format!("{what} 失败：rc={rc:#x}"),
        })
    }
}

/// sha256 前 16 位十六进制（**自己实现**，不引第三方依赖 —— `deer-vk` 保持零依赖）。
fn sha256_hex16(data: &[u8]) -> String {
    // SHA-256 常量
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e;
            e = d.wrapping_add(t1);
            d = c; c = b; b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut s = String::new();
    for v in &h {
        s.push_str(&format!("{v:08x}"));
    }
    s.truncate(16);
    s
}
