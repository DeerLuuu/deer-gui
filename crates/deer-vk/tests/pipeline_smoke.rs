//! M2a-3 驱动验收：**渲染通道 + 管线布局 + 图形管线**。
//!
//! ## 为什么这一步才是「SPIR-V 的真正验收」
//!
//! 上一步证明了「`vkCreateShaderModule` 接受我们的产物」，但实测发现那个函数**很宽容**
//! —— 连 `bound = 0` 的非法模块都接受。而 `vkCreateGraphicsPipelines` 会把
//! **顶点/片段两个阶段链接起来、并与管线状态一起校验**：拒绝就意味着着色器或状态
//! 真的有问题是。
//!
//! 所以本文件的通过，才算「自研 SPIR-V 汇编器可用于真实渲染管线」。
//!
//! 无 Vulkan 环境时优雅跳过，不伪装成通过。

use deer_vk::device::VkDevice;
use deer_vk::ffi_dev as vk;
use deer_vk::spirv;

/// `DEER_VK_VALIDATION=1` 是否被请求（判据与 `deer_vk::ffi::Instance::validation_from_env()` 一致）。
///
/// **不再在这里重写一遍判据**：直接调那个共用实现 —— 两处判据分叉过一次就会出现
/// 「测试以为没开校验、实际开着」的静默错位。它也容忍
/// `cmd /c "set DEER_VK_VALIDATION=1 && …"` 的尾空格（`"1 "`），见 `deer_vk::ffi::env_flag`。
fn validation_requested() -> bool {
    deer_vk::ffi::Instance::validation_from_env()
}

fn open() -> Option<VkDevice> {
    match VkDevice::open(0) {
        Ok(d) => Some(d),
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan（{e}）");
            None
        }
    }
}

/// 断言「到目前为止校验层**没有**报过任何消息」。
///
/// ## 用哪个计数、为什么（M3c 复查澄清，两次都踩过）
///
/// 这条断言原本用**进程级**计数 + `== 0`。问题：
/// - 它是**绝对**断言（"整个进程至今零消息"），而在**并行测试**里，别的测试产生的
///   消息也会让它非 0 ⇒ 一旦有任何一条测试合法地产生消息，**所有**这类断言都假红；
/// - 而 M3c 期间恰恰发生了这件事：我一条新测试的顶点属性表不全（只声明 1 个，
///   着色器消费 4/3 个）⇒ 5 条 `VUID-...-Input-07904` ⇒
///   `texture_sampler_descriptor_are_clean_under_validation` **假红**。
///
/// 改为**按线程**计数 + **窗口差值**（快照 → 跑 → 再快照）：
/// - 差值只看**本测试自己这几次调用**产生了什么 ⇒ 别的测试（别的线程）**不可能**影响它；
/// - 也天然不受「本进程里有人合法产生消息」影响（例如本文件里那条**故意违规**的
///   `validation_counter_is_thread_local_not_process_wide`）。
///
/// ## 强度上的取舍（诚实说明）
///
/// 按线程 + 差值看不见「**别的线程**在本测试期间报了什么」。
/// 那条更弱的覆盖由**另一条**测试补上：
/// [`process_reports_zero_validation_messages_in_a_fresh_child`] —— 它在**全新的子进程**里
/// 跑一段最小工作负载，然后断言那个进程的**全局**计数为 0
/// （子进程里没有别的测试 ⇒ 绝对断言在那里是**成立且有意义**的）。
/// 两条合起来 = 「本测试干净」（按线程）+ 「本进程的常规路径不产生消息」（子进程全局）。
fn assert_no_validation_messages(context: &str) {
    if !validation_requested() {
        // 层没开时计数恒为 0，断言没有意义（会给出虚假的安全感）——明确说出来。
        println!("（{context}：未请求校验层，跳过「零校验消息」断言）");
        return;
    }
    let n = deer_vk::ffi::validation_message_count();
    assert_eq!(
        n,
        0,
        "{context}: **本线程**收到 {n} 条校验层消息 —— \
         请在上面输出里找 [VK ERROR]/[VALIDATION] 字样。（按线程计数 ⇒ \
         这个数字只反映本测试自己触发的消息，不含别的测试）"
    );
}

/// **「层真的在跑」与「请求」必须一致**（T3 review F7 的同一条要求）。
///
/// 只在请求了校验层时检查：请求了就必须真的启用（否则后面那些「零消息」
/// 断言是在对一个没跑的回调断言，等于没验）。
fn validation_layer_is_actually_running(dev: &VkDevice) -> bool {
    if validation_requested() {
        assert!(
            dev.validation_enabled(),
            "DEER_VK_VALIDATION 已请求，但设备报告校验层未启用 —— \
             这时「零校验消息」毫无意义（回调根本不会被调用）"
        );
    }
    dev.validation_enabled()
}

/// 完整链路：渲染通道 → 管线布局 → 两个着色器模块 → **图形管线**。
///
/// 用不带推送常量的着色器 —— 推送常量那条路当前不可用（见
/// `spirv::vertex_shader_rect_pushconstant` 的说明）。
#[test]
fn graphics_pipeline_is_created_on_real_driver() {
    let Some(dev) = open() else { return };

    // ① 渲染通道：RGBA8、每帧清屏、最终布局 = 可回读
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("创建渲染通道");
    assert!(!pass.handle().is_null());

    // ② 管线布局：无推送常量、无描述符集
    let layout = dev.create_pipeline_layout(None).expect("创建管线布局");
    assert!(!layout.handle().is_null());

    // ③ 两个着色器：三角形（逐分量 OpSelect，无动态索引）
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]))
        .expect("顶点着色器应被接受");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([0.2, 0.6, 1.0, 1.0]))
        .expect("片段着色器应被接受");

    // ④ **M2a-3 的验收**：建图形管线
    let pipeline = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("vkCreateGraphicsPipelines 应成功");
    assert!(
        !pipeline.handle().is_null(),
        "管线句柄不能为空（驱动返回成功却不写输出参数时，这里会红）"
    );

    println!("渲染通道 + 管线布局 + 图形管线 全部创建成功 ✅（句柄 {:?}）", pipeline.handle());
    dev.wait_idle().expect("空闲等待");
}

/// 带推送常量的矩形着色器**当前不可用** —— 把这条实测事实写成测试，
/// 这样它不会被遗忘，也不会被误以为是可用的。
///
/// 详见 `deer_vk::spirv::vertex_shader_rect_pushconstant` 的文档：
/// 三种推送常量写法分别导致「空句柄」或「访问违例」。
#[test]
fn push_constant_rect_shader_is_known_broken() {
    // 校验层下**显式跳过**：同一支损坏的 SPIR-V 会让校验层/驱动把进程打成
    // 0xc0000005（STATUS_ACCESS_VIOLATION）。跳过要打印原因，不伪装通过。
    if validation_requested() {
        eprintln!(
            "跳过：`spirv::vertex_shader_rect_pushconstant()` 已知损坏\
             （PushConstant 变量不是 OpTypeStruct，违反 VUID-StandaloneSpirv-PushConstant-06808），\
             开启校验层时会让进程 0xc0000005 崩溃 —— 本测试在校验层下不执行。"
        );
        return;
    }
    let Some(dev) = open() else { return };
    let bytes = spirv::vertex_shader_rect_pushconstant();
    // 模块本身仍会被驱动接受（这正说明建模块的校验极弱）
    let vs = dev
        .create_shader_module(&bytes)
        .expect("模块本身应被接受 —— 问题不在建模块");
    println!(
        "注意：推送常量矩形着色器（{} 字节）能建模块，但建管线会失败（已知问题）",
        bytes.len()
    );
    let _ = vs;
}

/// 三角形着色器也能建管线（那支**不带推送常量**，走的是纯常量数组路径）。
#[test]
fn triangle_pipeline_also_builds() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    // 无推送常量
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]))
        .expect("顶点着色器");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]))
        .expect("片段着色器");
    dev.create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("三角形管线也应建成功");
    println!("三角形管线建成功 ✅");
}

/// 三种颜色格式都应能建渲染通道（后续窗口/离屏会用到不同格式）。
#[test]
fn render_pass_supports_all_target_formats() {
    let Some(dev) = open() else { return };
    for (name, format) in [
        ("R8G8B8A8_UNORM", vk::VK_FORMAT_R8G8B8A8_UNORM),
        ("R8G8B8A8_SRGB", vk::VK_FORMAT_R8G8B8A8_SRGB),
        ("B8G8R8A8_SRGB", vk::VK_FORMAT_B8G8R8A8_SRGB),
    ] {
        match dev.create_render_pass(
            format,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        ) {
            Ok(_) => println!("  {name}: 支持 ✅"),
            Err(e) => panic!("{name} 应被支持（GUI 需要它）：{e}"),
        }
    }
}

/// **回归锁（M3c 复查）**：`validation_message_count()` 必须是**按线程**的。
///
/// ## 为什么需要它（与 I-1 同类的缺陷）
///
/// `validation_message_count()` 的第一版是**进程级**计数，而用它做的断言是
/// 「跑完这段代码后**没有增长**」—— **窗口差值**。于是别的测试线程的消息会污染它。
/// 这不是理论风险：M3c 期间我一条新测试的顶点属性表不全（只声明 1 个，着色器消费 4/3 个）
/// ⇒ 校验层报 5 条 `VUID-...-Input-07904` ⇒ 落进进程级计数 ⇒
/// `texture_sampler_descriptor_are_clean_under_validation` **假红**
/// （它断言的是「本测试期间零消息」，却被**别的测试**的消息顶红了）。
///
/// ## 实测前提：回调是**线程亲和**的
///
/// 改 `thread_local!` 之前先实测过（故意的违规探针，工作线程触发 11 条消息、
/// 主线程同时读自己的 TLS）：工作线程 TLS `0→11`，主线程 TLS **全程 0** ⇒
/// 消息记在**触发它的那个线程**上。所以「按线程计数」如实反映「这段代码自己触发了什么」。
///
/// ## 这条锁**是确定性的**（不是偶发 flaky）
///
/// 它不依赖「碰巧并发」：用 channel 精确编排成
/// ① 工作线程触发消息 → ② 工作线程**仍然存活**并阻塞等放行 → ③ 主线程在这期间读**自己的**计数
/// → ④ 放行并 join → ⑤ 主线程再读一次。
/// 若计数器被改回**进程级**：
/// - 步骤 ③ 主线程会读到**工作线程**那 11 条（全局量）⇒ 立即红；
/// - 即使跳过 ③，步骤 ⑤ 也必然非 0 ⇒ 仍红。
///
/// 所以「改回进程级」**必定**红，而不是偶发 —— 这正是 I-1 那把锁的同构写法。
#[test]
fn validation_counter_is_thread_local_not_process_wide() {
    if !validation_requested() {
        println!("跳过：需要 DEER_VK_VALIDATION=1 才有消息可测（这不是通过）");
        return;
    }
    let Some(dev) = open() else { return };
    drop(dev);
    if !deer_vk::ffi::Instance::validation_from_env() {
        println!("跳过：校验层未请求");
        return;
    }

    let (tx_ready, rx_ready) = std::sync::mpsc::channel::<usize>();
    let (tx_go, rx_go) = std::sync::mpsc::channel::<()>();

    // 故意违规必然要把消息交给回调；而**校验层报完就会把调用放给驱动**，驱动拿着
    // 全零 `GraphicsPipelineCreateInfo` 去解引用空指针 ⇒ 本机 `0xc0000005`
    // （2026-10-01 实测：崩在 `validation_counter_is_thread_local_not_process_wide`）。
    // 打开中止开关 ⇒ 层跳过这次调用 ⇒ 驱动看不到非法参数。
    // 开关是**进程级原子量**，所以工作线程里发的消息一样受它约束。
    let worker = std::thread::spawn(move || {
        // 设备在**本线程内**打开（VkDevice 句柄不是 Send）
        let Ok(dev) = VkDevice::open(0) else { return 0usize };
        let fns = *dev.fns();
        let handle = dev.handle();
        let before = deer_vk::ffi::validation_message_count();
        // 中止开关是**线程局部**的 ⇒ 必须在**做违规的这个线程**里设（见 ffi.rs 的说明）。
        deer_vk::ffi::set_validation_abort_on_message(true);
        // **故意违规**：null 渲染通道 + 零阶段 + 全零 sType ⇒ 校验层必然报若干条。
        let mut pipe: vk::PipelineHandle = std::ptr::null_mut();
        // SAFETY: 故意传非法参数 —— 本测试的目的就是**产生**校验消息。
        let info = unsafe { std::mem::zeroed::<vk::GraphicsPipelineCreateInfo>() };
        let _rc = unsafe {
            (fns.create_graphics_pipelines)(
                handle,
                vk::NULL_HANDLE,
                1,
                &info,
                std::ptr::null(),
                &mut pipe,
            )
        };
        deer_vk::ffi::set_validation_abort_on_message(false);
        assert!(
            !deer_vk::ffi::validation_abort_on_message(),
            "中止开关没关回去 —— 会影响本线程后续所有 API 的返回行为"
        );
        let gain = deer_vk::ffi::validation_message_count() - before;
        let _ = tx_ready.send(gain);
        // 阻塞等主线程读完 —— 保证「工作线程仍存活」这个窗口存在
        let _ = rx_go.recv();
        gain
    });

    let worker_gain = rx_ready.recv().expect("工作线程没发信号");
    assert!(
        worker_gain > 0,
        "违规调用没有产生校验层消息（{worker_gain} 条）—— 本测试的前提不成立，\
         请检查校验层是否真的启用（validation_enabled）"
    );
    // ② 工作线程**仍存活**时，主线程读自己的计数：必须是 0
    let main_during = deer_vk::ffi::validation_message_count();
    let _ = tx_go.send(());
    let _ = worker.join().expect("工作线程 panic");
    let main_after = deer_vk::ffi::validation_message_count();

    assert_eq!(
        main_during, 0,
        "主线程在**工作线程存活期间**读到了 {main_during} 条消息 —— 而这些消息是\
         工作线程触发的 ⇒ `validation_message_count()` 不是按线程的（被改回进程级了？）"
    );
    assert_eq!(
        main_after, 0,
        "主线程的计数变成 {main_after}（工作线程触发了 {worker_gain} 条）⇒ \
         `validation_message_count()` 不是按线程的"
    );
    // ③ 全局计数**确实**涨了（证明上面那个 0 是「按线程隔离」而不是「回调没跑」）
    assert!(
        deer_vk::ffi::validation_message_count_global() >= worker_gain,
        "全局计数应当 ≥ 工作线程的增量（{} < {worker_gain}）—— \
         若这里不成立，说明两条计数没有同时自增",
        deer_vk::ffi::validation_message_count_global()
    );
    println!(
        "校验消息按线程隔离 ✅（工作线程触发 {worker_gain} 条；主线程全程 0；\
         全局 ≥ {worker_gain} —— 证明隔离而非回调未跑）"
    );
}

/// **计数器自身的机制守卫**：故意产生校验消息，断言「按线程计数确实动了、
/// 且全局计数同时动了」。
///
/// ## 为什么这条必要
///
/// 与它配对的 `assert_no_validation_messages` 断言的是「**0**」——
/// 而**「0」有两种成因**：① 真的没有消息；② **回调没跑 / 计数没接线**。
/// 光断言 0 无法区分，后者会让所有「零消息」结论变成空话。
/// 这条测试**主动制造**消息，从而证明「计数机制在工作、层确实会把消息交给我们的回调」。
///
/// 它同时钉住「两个计数**同时**自增」这个不变式（按线程 + 全局）——
/// 少了它，「本线程一直是 0」可能只是因为我漏接了自增。
#[test]
fn validation_counter_actually_counts_when_a_message_is_emitted() {
    if !validation_requested() {
        println!("跳过：需要 DEER_VK_VALIDATION=1（这不是通过）");
        return;
    }
    let Some(dev) = open() else { return };
    let fns = *dev.fns();
    let handle = dev.handle();
    assert!(
        dev.validation_enabled(),
        "请求了校验层但设备报告未启用 ⇒ 「零消息」结论没有意义"
    );

    let tls_before = deer_vk::ffi::validation_message_count();
    let global_before = deer_vk::ffi::validation_message_count_global();

    // 故意违规：全零 GraphicsPipelineCreateInfo（sType/stageCount/renderPass/layout 全非法）。
    //
    // ⚠️ **必须配 `set_validation_abort_on_message(true)`**（2026-10-01 实测）：
    // 校验层会把消息报出来（实测 11 条），但**报完照样把调用放给驱动** —— 驱动拿着这个
    // 全零结构体去解引用里面的空指针，本机直接 `0xc0000005` 把整个测试二进制打死
    // （于是 `DEER_VK_VALIDATION=1 cargo test -p deer-vk` 后面的靶全都跑不到）。
    // 打开开关 ⇒ 回调返回 `VK_TRUE` ⇒ 层**跳过这次调用** ⇒ 驱动根本看不到非法参数。
    // 我们要的是「有消息 ⇒ 计数自增」，不是「让驱动去啃非法内存」。
    //
    // 开关只在这一次调用周围打开，**紧接着关掉**（下面有一条断言钉「确实关了」）——
    // 默认必须是「不中止」，否则会改变所有正常 API 的返回行为。
    let mut pipe: vk::PipelineHandle = std::ptr::null_mut();
    let info = unsafe { std::mem::zeroed::<vk::GraphicsPipelineCreateInfo>() };
    deer_vk::ffi::set_validation_abort_on_message(true);
    // SAFETY: 故意传非法参数 —— 本测试的目的就是**产生**校验消息；
    // 而上面那个开关保证校验层会拦下这次调用，不让它进驱动。
    let rc_aborted = unsafe {
        (fns.create_graphics_pipelines)(
            handle,
            vk::NULL_HANDLE,
            1,
            &info,
            std::ptr::null(),
            &mut pipe,
        )
    };
    deer_vk::ffi::set_validation_abort_on_message(false);
    assert!(
        !deer_vk::ffi::validation_abort_on_message(),
        "用例结束了但中止开关还开着 —— 那会改变后续所有 API 的返回行为"
    );
    // 中止开关生效的**直接证据**：校验层拦下调用时返回的是它自己的错误码
    // （`VK_ERROR_VALIDATION_FAILED_EXT`，负数），而不是驱动那边的结果。
    println!("  被校验层拦下的调用返回 rc = {rc_aborted}（应为负）");

    let tls_gain = deer_vk::ffi::validation_message_count() - tls_before;
    let global_gain = deer_vk::ffi::validation_message_count_global() - global_before;
    assert!(
        tls_gain > 0,
        "故意违规的 vkCreateGraphicsPipelines 没有产生本线程消息（增量 {tls_gain}）—— \
         要么校验层没真的把消息交给我们的回调，要么按线程计数没接上自增"
    );
    assert!(
        global_gain >= tls_gain,
        "全局增量({global_gain}) 应当 ≥ 按线程增量({tls_gain}) —— 两个计数必须同时自增"
    );
    println!(
        "校验计数机制在工作 ✅（本线程 +{tls_gain} 条，全局 +{global_gain} 条）"
    );
}

/// **「本进程常规路径不产生校验消息」的绝对断言 —— 在全新的子进程里做。**
///
/// ## 为什么必须开子进程
///
/// 「整个进程一条消息都没有」是**绝对**断言，而在本测试二进制里它**不可能**成立：
/// 同文件的 `validation_counter_actually_counts_when_a_message_is_emitted` 与
/// `validation_counter_is_thread_local_not_process_wide` 都**故意**产生消息，
/// 且测试执行顺序不受控 ⇒ 进程级计数迟早非 0。
///
/// 所以把绝对断言放进**干净的进程**：子进程里只跑一个最小工作负载
/// （开设备 → 建渲染通道/统一管线 → 建纹理/采样器/描述符 → 销毁），
/// 然后断言那个进程的**全局**计数为 0。
///
/// ## 它是怎么做到「确定性」的
///
/// 子进程用**同一份测试二进制**、以 `--exact` 只跑 `child_probe_zero_messages` 这一个用例
/// （由 `DEER_VK_TLS_VALIDATION_PROBE` 环境变量打开），于是子进程里没有别的测试 ⇒
/// 绝对断言成立且有意义。父进程负责：注入 `DEER_VK_VALIDATION`、设开关、
/// 断言子进程 exit code == 0，并在失败时把它 stderr 里的校验消息打出来。
///
/// 若子进程二进制找不到（环境特殊），**明确跳过并说明**，不伪装通过。
#[test]
fn process_reports_zero_validation_messages_in_a_fresh_child() {
    // 自己是子进程时不要递归
    if std::env::var("DEER_VK_TLS_VALIDATION_PROBE").is_ok() {
        return;
    }
    if !validation_requested() {
        println!("跳过：需要 DEER_VK_VALIDATION=1（这不是通过）");
        return;
    }
    let Some(exe) = find_lib_test_binary() else {
        println!(
            "跳过：找不到 deer-vk 的 lib 测试二进制（先跑一次 `cargo test -p deer-vk --lib` \
             即可生成）—— 明确说明，不伪装通过"
        );
        return;
    };

    let out = std::process::Command::new(&exe)
        .args(["child_probe_zero_messages", "--exact", "--nocapture"])
        .env("DEER_VK_VALIDATION", "1")
        .env("DEER_VK_TLS_VALIDATION_PROBE", "1")
        .output()
        .expect("启动子进程");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // 子进程的 stderr 里若出现校验消息，原样打出来（这就是失败的原因）
    for line in stderr.lines().filter(|l| l.contains("VK ERROR") || l.contains("VALIDATION")) {
        println!("  子进程校验消息: {line}");
    }
    assert!(
        out.status.success(),
        "子进程（干净进程里的绝对断言）失败：exit={:?}\n--- stdout ---\n{}",
        out.status.code(),
        stdout.lines().rev().take(12).collect::<Vec<_>>().join("\n")
    );
    println!("干净子进程内「全局零校验消息」成立 ✅");
}

/// 在 target 目录里找 deer-vk 的 **lib** 测试二进制。
///
/// 选择理由：它同时含有 lib 单测与（通过 `DEER_VK_TLS_VALIDATION_PROBE` 打开的）
/// 下面的子进程探针，且**不含**本文件这些故意违规的集成测试 ⇒ 干净。
fn find_lib_test_binary() -> Option<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/deps");
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // `deer_vk-<hash>.exe`（lib 测试）；排除 `deer_vk-<hash>.d` 等
        if !name.starts_with("deer_vk-") || !name.ends_with(".exe") {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let Ok(mt) = md.modified() else { continue };
        if best.as_ref().map(|(t, _)| mt > *t).unwrap_or(true) {
            best = Some((mt, e.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// **子进程探针**：在干净进程里跑一条最小工作负载，断言**全局**校验计数为 0。
///
/// 由 [`process_reports_zero_validation_messages_in_a_fresh_child`] 以环境变量打开；
/// 默认（直接跑整个测试套件时）它**立即返回**，不产生噪声。
#[test]
fn child_probe_zero_messages() {
    if std::env::var("DEER_VK_TLS_VALIDATION_PROBE").is_err() {
        return;
    }
    let Some(dev) = open() else {
        eprintln!("子进程：没有可用 Vulkan");
        std::process::exit(2);
    };
    assert!(dev.validation_enabled(), "子进程里校验层必须真的启用");

    // 最小工作负载：渲染通道 + **统一管线** + 纹理/采样器/描述符
    let Ok(pass) = dev.create_render_pass(
        vk::VK_FORMAT_R8G8B8A8_UNORM,
        vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
        vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    ) else {
        eprintln!("子进程：建渲染通道失败");
        std::process::exit(3);
    };
    let shape = [deer_vk::VertexAttr {
        location: 0,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 0,
    }];
    // B5-3：旧的两条旧管线已删 ⇒ 探针改为建**统一管线**（界面路径唯一建的那一条），
    // 工作负载里仍然有真正的 `vkCreateGraphicsPipelines`（否则「零消息」会变成空转）。
    let pipes = deer_vk::pipelines::build_pipeline_resources(&dev).expect("子进程：建共用资源失败");
    let state = deer_vk::pipelines::shape_state(
        vk::VK_FORMAT_R8G8B8A8_UNORM,
        deer_vk::pipelines::ViewportStrategy::Dynamic,
        8,
        shape.to_vec(),
    );
    let vs = dev
        .create_shader_module(&deer_vk::spirv::vertex_shader_unified())
        .expect("子进程：建 VS 模块失败");
    let fs = dev
        .create_shader_module(&deer_vk::spirv::fragment_shader_unified())
        .expect("子进程：建 FS 模块失败");
    let _pipeline = dev
        .create_pipeline_from_state(
            &state,
            &[
                (&vs, vk::VK_SHADER_STAGE_VERTEX_BIT),
                (&fs, vk::VK_SHADER_STAGE_FRAGMENT_BIT),
            ],
            &pipes.text_layout,
            &pass,
        )
        .expect("子进程：建统一管线失败");
    let tex = dev.create_texture_r8(4, 4, &[0u8; 16]).expect("纹理");
    let sampler = dev.create_sampler().expect("采样器");
    let pool = dev.create_descriptor_pool(1).expect("池");
    let set = dev
        .allocate_descriptor_set(&pool, &pipes.text_set_layout)
        .expect("集");
    dev.update_descriptor_texture(&set, &tex, &sampler)
        .expect("写描述符");
    dev.wait_idle().expect("空闲");
    drop(set);
    drop(pool);
    drop(tex);

    let n = deer_vk::ffi::validation_message_count_global();
    if n != 0 {
        eprintln!("子进程：干净进程里出现了 {n} 条校验消息 —— 绝对断言失败");
        std::process::exit(1);
    }
    println!("子进程：全局校验计数 = 0 ✅");
}

// ── M3c-T1：共用管线层 ────────────────────────────────────────────────────────

/// **M3c-T1 验收**：共用层 + 统一管线**能在两种颜色格式 / 两种 viewport 策略下建出来**
/// （离屏 ↔ 窗口），即「同一批状态换两个参数就能服务另一条路径」。
///
/// ## 为什么这条测试的重点是「两种格式都能建出来」
///
/// M3c 要让窗口用同一套画法，而窗口的颜色格式是 **`B8G8R8A8_SRGB`（本机实测 `0x32`）**、
/// 离屏是 `R8G8B8A8_UNORM` —— 两者走的是**同一份**共用资源
/// （`pipelines::build_pipeline_resources`）+ **同一个**建管线函数
/// （[`deer_vk::gpu_render::build_unified_pipeline`]），只是 `color_format` 参数不同。
/// 所以「共用层对两种格式都能建出管线」是这层能用的前提。
///
/// ⚠️ **B5-3 的落点变化（如实登记）**：B5-2 之前这条断言的是
/// `build_pipelines` 建出的**形状 + 文本两条**管线（`set.shape` / `set.text` 句柄非空）。
/// 那两条管线已删（它们没有任何绑定点）⇒ 本条改为断言**统一管线**在两种组合下都能建出来，
/// 判据的**意思不变**（「同一批状态参数化后能服务两条路径」），只是对象换成了真正在用的那条。
///
/// 注意这里**只**验证管线建得出来（驱动接受状态组合）；**像素语义**在
/// `pipelines.rs` 的纯函数测试里定（sRGB 混合空间差 44 字节 ⇒ 结论是窗口要用
/// 线性格式，见那里的 `srgb_attachment_blending_diverges_far_beyond_one_lsb`）。
#[test]
fn shared_pipeline_layer_builds_both_paths() {
    let Some(dev) = open() else { return };

    // (名字, 颜色格式, viewport 策略) —— 覆盖两条路径的**真实**组合
    let cases: [(&str, i32, deer_vk::pipelines::ViewportStrategy); 2] = [
        (
            "离屏 R8G8B8A8_UNORM + 静态",
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            deer_vk::pipelines::ViewportStrategy::Static {
                width: 64,
                height: 48,
            },
        ),
        (
            "窗口 B8G8R8A8_SRGB + 动态",
            vk::VK_FORMAT_B8G8R8A8_SRGB,
            deer_vk::pipelines::ViewportStrategy::Dynamic,
        ),
    ];

    // 共用资源只建一份（它不含格式/viewport —— 那两样是统一管线的参数）
    let pipes = deer_vk::pipelines::build_pipeline_resources(&dev).expect("共用资源建成");

    for (name, format, viewport) in cases {
        let pass = dev
            .create_render_pass(
                format,
                vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            )
            .unwrap_or_else(|e| panic!("{name}: 建渲染通道失败：{e}"));

        let (pipeline, vs, fs) = deer_vk::gpu_render::build_unified_pipeline(
            &dev,
            &pass,
            format,
            viewport,
            &pipes.text_layout,
        )
        .unwrap_or_else(|e| panic!("{name}: 统一管线建不出来：{e}"));

        // ★ 前置条件：句柄真的非空（否则下面那句断言是空转）
        assert!(
            !pipeline.handle().is_null(),
            "{name}: 统一管线句柄不能为空"
        );
        // 着色器模块必须比管线活得久（这里只是把它们留在作用域里到循环末）
        assert!(!vs.handle().is_null() && !fs.handle().is_null());
        println!("  {name}: 统一管线建成 ✅");
    }
}

/// **实测记录**：颜色格式与渲染通道**不一致**时，本机驱动**不报错**。
///
/// ## 为什么这条测试断言的是「接受」而不是「拒绝」
///
/// 我最初写的断言是「不一致必须被拒绝」，**实测红了** —— 本机 Intel 驱动对
/// 「管线 `colorAttachment` 格式 ≠ 渲染通道附件格式」的组合返回成功。
/// 这与本项目反复踩到的那一类缺陷同源：**驱动接受非法/不一致的状态，症状是
/// 「不报错也不画」（或画出错色）**，而不是一个清晰的错误码。
///
/// 所以这条测试改成**记录事实**，于是它有两个作用：
/// 1. 钉住「不能指望驱动替我们发现格式传错」⇒ 调用方必须自己传对
///    （`build_pipeline_resources` 的文档里写明这条前提）；
/// 2. 若某天驱动升级后开始拒绝，这条会红，我们会知道「驱动变严了」——
///    那也是必须知道的变化（它意味着别处可能有依赖「被宽容接受」的代码）。
///
/// ⚠️ 因此**不把这条当护栏用**：真正防「传错格式」的手段是调用方传
/// `render_pass` 的同一格式（离屏 `COLOR_FORMAT`、窗口 `swapchain.format()`），
/// 以及 M3c-T3 的窗口读回对照（格式传错时像素会明显不对）。
#[test]
fn format_mismatch_is_accepted_by_this_driver_and_must_be_guarded_by_the_caller() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_B8G8R8A8_SRGB,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("建渲染通道");

    // ⚠️ 属性表必须**覆盖着色器真正消费的所有 location**，否则校验层会报
    // `VUID-VkGraphicsPipelineCreateInfo-Input-07904`（"does not have a Location N,
    // but VERTEX has ... at that Location"）——本条测试**只想**验证格式不一致的行为，
    // 不该顺带引入别的 VUID（那会让「零校验消息」的断言被无关消息污染：
    // 这正是我第一版只声明 1 个属性时踩到的，实测 5 条 VUID）。
    //
    // B5-3 起这里不需要手写属性表了：建的是**统一管线**，而它的属性表由实现侧的
    // `gpu_render::unified_attrs()`（`offset_of!` 生成、5 个 location）提供 ——
    // 「表覆盖着色器的 location」由 `spirv_val.rs` 的接口断言与
    // `gpu_vs_cpu::unified_vertex_layout_matches_the_unified_attribute_offsets` 钉住。
    let pipes = deer_vk::pipelines::build_pipeline_resources(&dev).expect("共用资源建成");
    let r = deer_vk::gpu_render::build_unified_pipeline(
        &dev,
        &pass,
        vk::VK_FORMAT_R8G8B8A8_UNORM, // ← 与通道的 SRGB **不一致**
        deer_vk::pipelines::ViewportStrategy::Dynamic,
        &pipes.text_layout,
    );
    match r {
        Ok(_) => println!(
            "实测：颜色格式与渲染通道不一致时驱动**接受**（不报错）—— \
             这正是本项目反复遇到的那类「静默不一致」；格式必须由调用方传对"
        ),
        Err(e) => println!(
            "实测：本机驱动**拒绝**了格式不一致（{e}）—— \
             比预期的更严，说明驱动版本变了；本测试的断言需要随之更新"
        ),
    }
}

/// 推送常量大小必须是 4 的倍数 —— 这类错误应当在**调用驱动前**就被拦下。
#[test]
fn pipeline_layout_rejects_misaligned_push_constant() {
    let Some(dev) = open() else { return };
    let e = match dev.create_pipeline_layout(Some((vk::VK_SHADER_STAGE_VERTEX_BIT, 0, 6))) {
        Ok(_) => panic!("6 不是 4 的倍数，必须在调用驱动前就报错"),
        Err(e) => e,
    };
    assert!(
        e.to_string().contains("4 的倍数"),
        "错误信息应当说清原因，实际：{e}"
    );
}

/// **关于「驱动是否校验」的实测记录**（写成测试，避免以后重复踩）。
///
/// 本机驱动（Intel RaptorLake / Vulkan 1.4.309）在**非法阶段配置**下**直接崩溃**
/// 而不返回错误码。实测两种非法输入都是 `STATUS_ACCESS_VIOLATION`：
/// 1. 两个阶段都设成 `VERTEX`（规范禁止重复阶段）；
/// 2. 把片段着色器模块放到顶点槽位。
///
/// 结论与影响：
/// - **不能用非法输入来证明驱动在校验**（它会崩，测不出「返回错误」）；
/// - 所以本文件的「管线建成功」是**正向证据**（句柄非空），
///   而「驱动会拒绝坏输入」这件事**没有**在本机得到证明；
/// - 但 `vkCreateGraphicsPipelines` 确实做了**真实编译**：早期版本用
///   `OpAccessChain` 动态索引常量数组时，它在这里崩，而 `vkCreateShaderModule`
///   却接受了同样的字节 —— 说明编译发生在建管线阶段。
///
/// 本测试只记录事实，不做「必须失败」的断言（那种断言会依赖驱动的崩溃行为）。
#[test]
fn driver_crashes_on_invalid_stage_config_is_recorded() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    let layout = dev.create_pipeline_layout(None).expect("管线布局");

    // 合法的组合仍然能建成功 —— 这是本测试唯一断言的东西
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]))
        .expect("vs");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([1.0, 0.0, 0.0, 1.0]))
        .expect("fs");
    let p = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("合法组合应建成功");
    assert!(!p.handle().is_null());
    println!("合法组合建管线成功 ✅（非法组合会让驱动崩溃，见本测试文档注释）");
}

/// 反复创建/销毁管线不能崩或泄漏（验证 RAII 包装的 Drop 顺序）。
#[test]
fn pipeline_lifecycle_is_stable() {
    let Some(dev) = open() else { return };
    for i in 0..5 {
        let pass = dev
            .create_render_pass(
                vk::VK_FORMAT_R8G8B8A8_UNORM,
                vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            )
            .expect("渲染通道");
        let layout = dev
            .create_pipeline_layout(None)
            .expect("管线布局");
        // 用**已知可用**的着色器（推送常量那条路当前不可用）
        let vs = dev
            .create_shader_module(&spirv::vertex_shader_triangle([
                [-1.0, -1.0],
                [3.0, -1.0],
                [-1.0, 3.0],
            ]))
            .expect("vs");
        let fs = dev
            .create_shader_module(&spirv::fragment_shader_solid([0.1, 0.2, 0.3, 1.0]))
            .expect("fs");
        let _pipeline = dev
            .create_graphics_pipeline(&vs, &fs, &layout, &pass)
            .expect("管线");
        // 离开作用域时按「管线 → 着色器 → 布局 → 渲染通道」逆序析构
        let _ = i;
    }
    dev.wait_idle().expect("空闲等待");
    println!("连续 5 轮 创建/销毁 均正常 ✅");
}

/// **`create_vertex_pipeline` 的 5 条错误路径**（T3 review **F6** —— 上两轮 review 与 ledger
/// 都没提到它，属「既没做也没说」的那条；本轮补上）。
///
/// 分两层覆盖：
/// - **纯函数层**：`device.rs` 的 `tests::vertex_pipeline_args_reject_all_five_bad_inputs`
///   （无需 GPU，任何机器都会跑）；
/// - **本测试（公开路径层）**：走真实 `VkDevice`，证明校验**确实接在公开入口上**
///   —— 抽函数时最容易犯的错就是「抽出来了但调用点没接」。
///
/// 5 条都由函数**开头**的参数校验返回，不会创建任何 Vulkan 对象（因此无 GPU 时本条跳过，
/// 而纯函数那层仍然覆盖）。
#[test]
fn create_vertex_pipeline_rejects_bad_arguments() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    let extent = vk::Extent2D {
        width: 16,
        height: 16,
    };
    // 校验发生在 `stages` 被使用之前 ⇒ 空阶段数组就够（也不会走到 build_pipeline）
    let stages: [vk::PipelineShaderStageCreateInfo; 0] = [];
    let attr0 = deer_vk::VertexAttr {
        location: 0,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 0,
    };
    let attr_oob = deer_vk::VertexAttr {
        location: 0,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 44, // == stride ⇒ 越界
    };
    let attr_dup_a = deer_vk::VertexAttr {
        location: 1,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 0,
    };
    let attr_dup_b = deer_vk::VertexAttr {
        location: 1,
        format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
        offset: 8,
    };
    let zero = vk::Extent2D {
        width: 0,
        height: 16,
    };

    let cases: [(&str, vk::Extent2D, u32, &[deer_vk::VertexAttr], &str); 5] = [
        ("stride = 0", extent, 0, &[attr0], "stride"),
        ("attrs 空", extent, 44, &[], "属性"),
        ("extent 宽为 0", zero, 44, &[attr0], "viewport"),
        ("offset ≥ stride", extent, 44, &[attr_oob], "offset"),
        ("location 重复", extent, 44, &[attr_dup_a, attr_dup_b], "重复"),
    ];

    for (what, e, stride, attrs, needle) in cases {
        let err = match dev.create_vertex_pipeline(&stages, &layout, &pass, e, stride, attrs) {
            Ok(_) => panic!("{what} 必须被拒，却建成了管线"),
            Err(err) => err,
        };
        // 不只断言 `is_err()`：还要断言**拒的原因**（否则「拒了但拒错理由」也算通过）
        let msg = format!("{err}");
        assert!(
            msg.contains(needle),
            "{what}: 错误信息应当提到「{needle}」，实际：{msg}"
        );
        println!("  {what} ⇒ 已拒：{msg}");
    }
}

// ── M3b：纹理 / 采样器 / 描述符集 ────────────────────────────────────────────

/// 一帧可用的 `R8_UNORM` 覆盖率数据：左边一列 255、右边一列 0。
///
/// 用**有梯度**的数据而不是全 0：全 0 的纹理即使上传路径整段坏掉
/// （例如拷贝没发生、屏障漏了），采样结果也是 0 —— 「传没传成功」无法区分。
/// 一列 255 一列 0 时，只要采样能读到正确的 `x`，就能证明上传真的生效。
fn coverage_8x2() -> Vec<u8> {
    let mut data = vec![0u8; 8 * 2];
    for row in 0..2 {
        data[row * 8] = 255;
        data[row * 8 + 1] = 128;
    }
    data
}

/// 建一遍 M3b 的全部资源（纹理 → 采样器 → 布局 → 池 → 集 → 写描述符），
/// 返回尺寸等**可断言的事实**，资源本体在返回前全部析构。
///
/// ## 为什么把「创建」和「析构」放在同一个函数里
///
/// `vkFreeDescriptorSets` 要求池建时带 `FREE_DESCRIPTOR_SET_BIT`，而**不带时的报错
/// 只在析构那一刻才出现**（实测：校验层报
/// `VUID-vkFreeDescriptorSets-descriptorPool-00312`）。如果测试只在「创建完」时
/// 断零消息，这条错误就永远漏掉 —— 这正是本次开发的真实经历。所以这里让资源
/// 在函数返回前走完 `Drop`，调用方随后断消息计数才是「全生命周期」的结论。
fn create_and_destroy_texture_pipeline(mut check: impl FnMut(&deer_vk::device::Texture)) -> (u32, u32) {
    fn run(check: &mut impl FnMut(&deer_vk::device::Texture)) -> (u32, u32) {
        let Some(dev) = open() else { return (0, 0) };
        let tex = dev.create_texture_r8(8, 2, &coverage_8x2()).expect("创建 R8 纹理");
        assert_eq!(tex.width(), 8);
        assert_eq!(tex.height(), 2);
        assert!(!tex.image_view().is_null(), "图像视图句柄不能为空");
        assert_eq!(
            tex.layout(),
            vk::VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            "上传完成后必须处于 SHADER_READ_ONLY_OPTIMAL（描述符里声明的是它）"
        );
        check(&tex);

        let sampler = dev.create_sampler().expect("创建采样器");
        assert!(!sampler.handle().is_null(), "采样器句柄不能为空");
        let dsl = dev
            .create_descriptor_set_layout_combined_sampler()
            .expect("创建描述符集布局");
        assert!(!dsl.handle().is_null(), "布局句柄不能为空");
        // ⚠️ 声明顺序**故意**是 `pool` 在前、`set` 在后：
        // Rust 的局部量按**声明逆序**析构 ⇒ `set` 先析构（`vkFreeDescriptorSets`
        // 要求池此刻仍存活），`pool` 后析构。反过来（`set` 在前）会让
        // `vkFreeDescriptorSets` 拿到已销毁的池 —— reviewer 实测那会
        // **0xc0000005 崩溃**，不是一句清晰的错误码。
        //
        // ⚠️⚠️ **别把这条和「结构体字段」的规则混在一起 —— 两者方向相反**：
        //   · 局部量（`let`）：按声明**逆序**析构，即「后声明的先析构」⇒ 本处池在前。
        //   · 结构体字段：按声明**顺序**析构，即「先声明的先析构」⇒ 若把池与集放进
        //     同一个结构体，**池的字段必须声明在集之后**（才能让集先析构）。
        //   两处结论看似矛盾，其实是两条不同的规则；`device.rs` 的
        //   `DescriptorSet` 类型文档写的就是「结构体字段」那条。
        let pool = dev.create_descriptor_pool(1).expect("创建描述符池");
        assert_eq!(pool.max_sets(), 1);
        let set = dev.allocate_descriptor_set(&pool, &dsl).expect("分配描述符集");
        assert!(!set.handle().is_null(), "描述符集句柄不能为空");
        dev.update_descriptor_texture(&set, &tex, &sampler).expect("写描述符");
        dev.wait_idle().expect("空闲等待");
        (tex.width(), tex.height())
        // 此处按声明逆序 `Drop`：set → pool → dsl → sampler → tex。
        // 全过程都在校验层眼皮下，所以调用方的计数断言覆盖了创建 + 使用 + 析构。
    }
    run(&mut check)
}

/// **M3b T1 验收**：`R8_UNORM` 纹理 → staging 上传 → 图像视图 → 最近邻采样器
/// → 描述符池/布局/集 → 写入描述符，**在校验层下零校验消息（计数断言 ==0）**。
///
/// 覆盖的是「自研汇编器第一次接触描述符体系」时最容易错的几种状态：
/// 布局转换的 `oldLayout` 必须与图像**实际**布局一致、描述符的 `imageLayout`
/// 必须与上传后的布局一致、`usage` 必须含 `SAMPLED | TRANSFER_DST`、
/// 池必须允许归还单个集。这几条错了校验层都会报，而驱动可能只是
/// 「采样到全黑」或「析构时报一条」。
#[test]
fn texture_sampler_descriptor_are_clean_under_validation() {
    let Some(dev) = open() else { return };
    let running = validation_layer_is_actually_running(&dev);
    drop(dev);

    let mut created = false;
    let (w, h) = create_and_destroy_texture_pipeline(|_tex| {
        created = true;
    });
    assert!(created, "纹理创建回调没被调用（说明资源根本没建起来）");
    println!(
        "R8 纹理（{w}×{h}，{} 字节）+ 采样器 + 描述符集 全生命周期走完 ✅（校验层 {}）",
        coverage_8x2().len(),
        if running { "已启用" } else { "未启用" }
    );
    assert_no_validation_messages("M3b 纹理/采样器/描述符（含析构）");
}

/// 纹理的**尺寸与数据长度**错误必须在调用驱动前被拒（纯函数校验的驱动侧复验）。
///
/// 纯函数那一侧在 `device.rs` 单元测试里已覆盖（无 GPU 也能跑）；这里额外走一遍
/// **真实设备**的入口，防止「校验函数写了但 `create_texture_r8` 忘了调」这类
/// 接线错误 —— 那种错在纯函数测试里是**看不见**的。
#[test]
fn texture_r8_rejects_bad_args_before_touching_driver() {
    let Some(dev) = open() else { return };

    let e = dev.create_texture_r8(0, 4, &[]).expect_err("宽为 0 必须被拒");
    assert!(format!("{e}").contains("宽高"), "{e}");

    let e = dev.create_texture_r8(4, 4, &[0u8; 15]).expect_err("数据少 1 字节必须被拒");
    assert!(format!("{e}").contains("15"), "{e}");

    let e = dev.create_texture_r8(4, 4, &[0u8; 17]).expect_err("数据多 1 字节必须被拒");
    assert!(format!("{e}").contains("17"), "{e}");

    println!("纹理参数错误在调用驱动前被拒 ✅");
}

/// 描述符池容量为 0 必须被拒（池的边界条件）。
#[test]
fn descriptor_pool_rejects_zero_capacity() {
    let Some(dev) = open() else { return };
    let e = dev.create_descriptor_pool(0).expect_err("max_sets=0 必须被拒");
    assert!(format!("{e}").contains("max_sets"), "{e}");
    println!("描述符池 max_sets=0 被拒 ✅");
}

// ── B5-3：**析构顺序**的具名判据（复审 Important F-3）────────────────────────

/// 子进程探针成功跑完时打印的哨兵（父进程用它区分「真的跑了」与「0 tests run」）。
const DTOR_SENTINEL: &str = "【DTOR 探针】";

/// **子进程探针**：构造 + 用 + **析构**一个离屏 [`deer_vk::GpuGeometryRenderer`]。
///
/// ## 为什么这件事需要一个**具名**判据（复审 F-3 的实测）
///
/// `GpuGeometryRenderer` 的字段顺序是**硬契约**：`descriptor_set` 必须声明在
/// `descriptor_pool` **之前**（`DescriptorSet::drop` 会调
/// `vkFreeDescriptorSets(device, pool, ..)`）。把顺序写反 ⇒ **每条测试的逻辑都通过**
/// （像素逐字节相同、计数 1/1、`render` 无错），但**进程在退出时
/// `STATUS_ACCESS_VIOLATION`（`0xC0000005`）**，而且**没有任何失败用例名**、
/// 关掉校验层时**没有任何消息**。
///
/// 也就是说：这不是**静默**缺陷（`cargo` 会报 `exit code: 0xc0000005`），
/// 但**定位极差**（连「是哪条用例」都不知道）。本探针把它变成可局部化的判据：
/// **构造 → 渲染一帧（含文本 ⇒ 图集 + 描述符改指）→ 析构 → 进程正常退出**。
/// 顺序写反时子进程的退出码就不是 0 ⇒ 父进程那条**具名用例**失败。
///
/// 由 [`renderer_construct_and_destruct_in_a_fresh_child_exits_cleanly`] 以
/// `DEER_VK_DTOR_PROBE` 打开；默认（直接跑整个测试套件时）**立即返回**，不产生噪声。
#[test]
fn child_probe_offscreen_renderer_release() {
    if std::env::var("DEER_VK_DTOR_PROBE").is_err() {
        return;
    }
    use deer_gpu::draw::{Color, DrawCmd, DrawList};
    use deer_gpu::{Extent, RectI};

    let extent = Extent {
        width: 48,
        height: 32,
    };
    let Ok(mut r) = deer_vk::GpuGeometryRenderer::new(0, extent, Color::rgb(16, 16, 16)) else {
        eprintln!("子进程：本机没有可用的 Vulkan GPU");
        std::process::exit(2);
    };
    // ① 形状帧（覆盖：统一管线 + 顶点缓冲 + 屏障 + 哑纹理描述符）
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(2, 2, 20, 16),
        color: Color::WHITE,
    });
    r.render(&l).expect("子进程：形状帧渲染失败");
    // ② 有系统字体时连文本路径一起走（图集纹理 + **描述符改指**也是析构契约的一部分）
    let text_ran = match deer_gpu::text::TextEngine::from_system_font(16.0) {
        Ok(engine) => {
            let mut r = r.with_text(engine).expect("子进程：with_text 失败");
            let mut lt = DrawList::new();
            lt.push(DrawCmd::Text {
                rect: RectI::new(2, 2, 40, 20),
                text: "Wg".into(),
                color: Color::WHITE,
                size: 16.0,
                align: 0,
            });
            r.render(&lt).expect("子进程：文本帧渲染失败");
            drop(r);
            true
        }
        Err(e) => {
            eprintln!("子进程：拿不到系统字体（{e}）⇒ 只覆盖形状帧路径");
            drop(r);
            false
        }
    };
    println!(
        "{DTOR_SENTINEL}离屏渲染器（统一管线 + 描述符集/池{text_state}）已构造并**析构**，进程准备正常退出",
        text_state = if text_ran { " + 图集" } else { "" }
    );
}

/// **具名判据**：在**干净子进程**里构造 + 析构一个离屏渲染器，子进程必须**正常退出**。
///
/// 与 [`child_probe_offscreen_renderer_release`] 配对；它抓的是「字段顺序写反 ⇒
/// `vkFreeDescriptorSets` 访问已销毁的池 ⇒ 进程 abort」这条**原本没有名字**的失效模式。
///
/// ## 为什么用 `std::env::current_exe()` 而不是扫 target 目录
///
/// 本文件就是一个测试二进制 ⇒ 子进程用**同一个可执行文件** + `--exact <探针名>`
/// 只跑那一个用例（不依赖「在哪找 deer_vk-<hash>.exe」这种脆弱约定）。
/// 父进程**必须**校验两件事，否则判据会空转：
/// 1. 退出码 == 0（顺序写反时是 `0xC0000005`）；
/// 2. stdout 里有哨兵 `【DTOR 探针】`（`--exact` 没匹配到时 libtest 会报 `0 passed`
///    并且同样 exit 0 ⇒ 只看退出码会把「什么都没跑」当成通过）。
#[test]
fn renderer_construct_and_destruct_in_a_fresh_child_exits_cleanly() {
    // 自己是子进程时不要递归
    if std::env::var("DEER_VK_DTOR_PROBE").is_ok() {
        return;
    }
    let exe = std::env::current_exe().expect("拿不到当前测试二进制路径");
    let out = std::process::Command::new(&exe)
        .args([
            "--exact",
            "child_probe_offscreen_renderer_release",
            "--nocapture",
        ])
        .env("DEER_VK_DTOR_PROBE", "1")
        .output()
        .expect("启动子进程");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // ★ 先看**退出码**：它才是这条判据的主体 —— 顺序写反时子进程会以 `0xC0000005` 退出。
    //   （顺序不能反：崩掉的子进程**不会**打印哨兵，先查哨兵会把「崩了」误报成「空转」。）
    assert!(
        out.status.success(),
        "子进程（构造 + 析构一个离屏渲染器）**没有正常退出**：exit={:?}\n\
         若是 `0xC0000005`（STATUS_ACCESS_VIOLATION）：几乎一定是 `GpuGeometryRenderer`\
         的**字段顺序被写反**了（`descriptor_pool` 提到 `descriptor_set` 之前 ⇒\
         `vkFreeDescriptorSets` 访问已销毁的池）。\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        stdout.lines().rev().take(8).collect::<Vec<_>>().join("\n"),
        stderr.lines().rev().take(8).collect::<Vec<_>>().join("\n")
    );
    // ★ 再看**哨兵**：退出码 0 但没有哨兵 ⇒ 探针根本没跑（`--exact` 没匹配到）
    //   ⇒ 这条判据会**空转**，必须单独报出来（别把「什么都没跑」当成通过）。
    assert!(
        stdout.contains(DTOR_SENTINEL),
        "子进程退出码正常，但没有跑探针（stdout 里没有哨兵 `{DTOR_SENTINEL}`）⇒ 本判据会空转\
         （`--exact` 没匹配到用例名？）\n--- stdout ---\n{}",
        stdout
    );
    println!("干净子进程里「构造 + 析构」正常退出 ✅（析构顺序契约有名字了）");
}
